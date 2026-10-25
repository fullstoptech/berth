#![allow(clippy::expect_used)]
//! The by-hand check of P4-T5 as a live test: create `berthfx-net` with
//! subnet 172.31.250.0/24 through the actions service, connect
//! `berthfx-web-1`, see `docker inspect -f '{{json .NetworkSettings.Networks}}'`
//! list it, disconnect (the container is back on its original networks) and
//! delete it through the confirm builder.
//!
//! The sheet's Labels field gives the network `dev.berth.fixture=1`; the
//! test only ever removes the network named exactly `berthfx-net`, only when
//! it carries that label, and only disconnects `berthfx-web-1` from it. No
//! other network or container is touched.
//!
//! Run with `BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only
//! networks_it` (live tests share the engine: run them one at a time).

mod common;

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::actions::{apply_finished, ActionsService, Finished};
use berth_app::services::confirm;
use berth_app::services::engines::client_map;
use berth_app::services::networks::{Draft, NetOp};
use berth_app::state::{use_app_state, AppState, EngineScope};
use berth_core::{ContainerId, EngineId};
use berth_engine::EngineClient;

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some(state));
    });
    rsx! { div {} }
}

const NET: &str = "berthfx-net";
const WEB: &str = "berthfx-web-1";
const SUBNET: &str = "172.31.250.0/24";

fn docker(bin: &std::path::Path, host: &str, args: &[&str]) -> std::process::Output {
    std::process::Command::new(bin)
        .env("DOCKER_HOST", host)
        .args(args)
        .output()
        .expect("docker CLI runs")
}

fn text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The fixture label of the network named exactly `berthfx-net`, `""` when
/// it has none or does not exist.
fn fixture_label(bin: &std::path::Path, host: &str) -> String {
    text(&docker(
        bin,
        host,
        &[
            "network",
            "inspect",
            "-f",
            "{{index .Labels \"dev.berth.fixture\"}}",
            NET,
        ],
    ))
}

/// Only the network named exactly `berthfx-net` and only when it carries
/// the fixture label; only the fixture web container leaves it.
fn clean(bin: &std::path::Path, host: &str) {
    if fixture_label(bin, host) != "1" {
        return;
    }
    let _ = docker(bin, host, &["network", "disconnect", "-f", NET, WEB]);
    let _ = docker(bin, host, &["network", "rm", NET]);
}

struct CleanOnDrop(std::path::PathBuf, String);

impl Drop for CleanOnDrop {
    fn drop(&mut self) {
        clean(&self.0, &self.1);
    }
}

struct Rig {
    dom: VirtualDom,
    state: AppState,
    client: Arc<dyn EngineClient>,
    actions: ActionsService,
    finished: tokio::sync::mpsc::UnboundedReceiver<Finished>,
    engine: EngineId,
    bin: std::path::PathBuf,
    host: String,
}

impl Rig {
    fn cli(&self, args: &[&str]) -> String {
        text(&docker(&self.bin, &self.host, args))
    }

    /// Put what the UI would hold into the signals.
    async fn load_state(&mut self) {
        let containers = self.client.containers().await.expect("containers");
        let networks = self.client.networks().await.expect("networks");
        let mut state = self.state;
        self.dom.in_runtime(|| {
            state.containers.set(containers);
            state.networks.set(networks);
        });
    }

    async fn web(&self) -> ContainerId {
        let containers = self.client.containers().await.expect("containers");
        let web = containers
            .iter()
            .find(|c| c.name == WEB)
            .expect("the web fixture exists");
        assert_eq!(
            web.labels.get("dev.berth.fixture").map(String::as_str),
            Some("1"),
            "refusing to touch a container without the fixture label"
        );
        web.id.clone()
    }

    async fn settle(&mut self) -> Finished {
        let done = tokio::time::timeout(std::time::Duration::from_secs(60), self.finished.recv())
            .await
            .expect("the action finishes")
            .expect("channel open");
        let copy = Finished {
            key: done.key.clone(),
            result: done.result.clone(),
            replaced_by: done.replaced_by.clone(),
            message: done.message.clone(),
        };
        let mut state = self.state;
        self.dom.in_runtime(|| apply_finished(&mut state, done));
        copy
    }

    fn run(&self, id: &str, name: &str, op: NetOp) {
        let mut state = self.state;
        let actions = self.actions.clone();
        let engine = self.engine.clone();
        self.dom
            .in_runtime(|| actions.network(&mut state, &engine, id, name, op))
            .expect("the action is accepted");
    }

    fn web_networks(&self) -> String {
        self.cli(&["inspect", "-f", "{{json .NetworkSettings.Networks}}", WEB])
    }

    fn web_network_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .cli(&[
                "inspect",
                "-f",
                "{{range $k, $v := .NetworkSettings.Networks}}{{$k}} {{end}}",
                WEB,
            ])
            .split_whitespace()
            .map(str::to_string)
            .collect();
        names.sort();
        names
    }
}

async fn rig() -> Option<(Rig, CleanOnDrop)> {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return None;
    }
    common::settle_fixture().await;
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let mut state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima_cli = berth_engine::ColimaCli::new(search.clone());
    let engines = berth_engine::discovery::discover(&colima_cli, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default" && e.state == berth_core::EngineState::Running)
        .expect("colima:default running")
        .clone();
    let host = engine.docker_host.clone().expect("docker host");
    let bin = search
        .find("docker")
        .unwrap_or_else(|| std::path::PathBuf::from("docker"));
    let guard = CleanOnDrop(bin.clone(), host.clone());
    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let clients = client_map();
    clients
        .lock()
        .expect("clients")
        .insert(engine.id.clone(), Arc::clone(&client));
    let (actions, finished) = ActionsService::new(
        clients,
        Arc::new(berth_store::Store::open_in_memory().expect("store")),
        Arc::new(berth_engine::CliCompose::new(search.clone())),
        Arc::new(berth_engine::MockColima::bundled()),
        CancellationToken::new(),
    );
    dom.in_runtime(|| {
        state.engines.set(engines.clone());
        state.engine_scope.set(EngineScope::One(engine.id.clone()));
    });
    let rig = Rig {
        dom,
        state,
        client,
        actions,
        finished,
        engine: engine.id.clone(),
        bin,
        host,
    };
    clean(&rig.bin, &rig.host);
    Some((rig, guard))
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn networks_it_create_connect_inspect_disconnect_delete() {
    let Some((mut rig, _guard)) = rig().await else {
        return;
    };
    let original = rig.web_network_names();
    println!("networks_it: {WEB} starts on {original:?}");
    assert!(
        !original.is_empty(),
        "the fixture web container is on a network"
    );

    // Create, from the sheet's draft.
    let draft = Draft {
        name: NET.to_string(),
        subnet: SUBNET.to_string(),
        labels: "dev.berth.fixture=1".to_string(),
        ..Draft::default()
    };
    rig.load_state().await;
    let existing = rig.dom.in_runtime(|| rig.state.networks.peek().clone());
    let spec = berth_app::services::networks::validate(&draft, &existing).expect("a valid draft");
    rig.run(NET, NET, NetOp::Create(spec));
    let done = rig.settle().await;
    assert_eq!(done.result, Ok(()), "the engine created it");
    assert_eq!(done.message.as_deref(), Some("Created network berthfx-net"));
    assert_eq!(
        rig.cli(&[
            "network",
            "inspect",
            "-f",
            "{{(index .IPAM.Config 0).Subnet}}",
            NET
        ]),
        SUBNET
    );
    assert_eq!(
        fixture_label(&rig.bin, &rig.host),
        "1",
        "the sheet's Labels field reached the engine"
    );

    // Connect berthfx-web-1.
    rig.load_state().await;
    let network_id = rig
        .dom
        .in_runtime(|| {
            rig.state
                .networks
                .peek()
                .iter()
                .find(|n| n.name == NET)
                .map(|n| n.id.clone())
        })
        .expect("the new network is listed");
    let web = rig.web().await;
    rig.run(
        &network_id,
        NET,
        NetOp::Connect {
            network: NET.to_string(),
            container: web.clone(),
        },
    );
    let done = rig.settle().await;
    assert_eq!(done.result, Ok(()), "the engine connected it");
    let json = rig.web_networks();
    println!("networks_it: after connect: {json}");
    assert!(json.contains(NET), "docker inspect lists {NET}: {json}");
    rig.load_state().await;
    let listed = rig
        .dom
        .in_runtime(|| {
            rig.state
                .networks
                .peek()
                .iter()
                .find(|n| n.name == NET)
                .cloned()
        })
        .expect("listed");
    assert_eq!(
        listed.containers,
        vec![web.clone()],
        "Berth lists the member"
    );

    // Deleting while a container is on it is refused before the engine.
    let refused = {
        let mut state = rig.state;
        let actions = rig.actions.clone();
        let engine = rig.engine.clone();
        rig.dom.in_runtime(|| {
            actions.remove(
                &mut state,
                &engine,
                &berth_core::ResourceRef::Network(network_id.clone()),
                NET,
                false,
            )
        })
    };
    assert_eq!(refused, Err("1 container is connected".to_string()));

    // Disconnect: web-1 is back on its original networks.
    rig.run(
        &network_id,
        NET,
        NetOp::Disconnect {
            network: NET.to_string(),
            container: web,
        },
    );
    let done = rig.settle().await;
    assert_eq!(done.result, Ok(()), "the engine disconnected it");
    assert_eq!(rig.web_network_names(), original, "left as it was");

    // Delete through the confirm dialog's model, only a fixture-labelled one.
    assert_eq!(
        fixture_label(&rig.bin, &rig.host),
        "1",
        "refusing to delete a network without the fixture label"
    );
    rig.load_state().await;
    let model = {
        let state = rig.state;
        rig.dom
            .in_runtime(|| confirm::network_delete_model(&state, &network_id))
            .expect("the dialog opens for the unused network")
    };
    assert_eq!(confirm::button_label(&model.effect), "Delete network");
    let launched = {
        let actions = rig.actions.clone();
        let mut state = rig.state;
        rig.dom
            .in_runtime(|| confirm::run(&mut state, &actions, &model, false))
    };
    assert!(launched.refused.is_empty(), "{:?}", launched.refused);
    let done = rig.settle().await;
    assert_eq!(done.result, Ok(()), "the engine deleted it");
    let left = rig.cli(&["network", "ls", "-q", "--filter", &format!("name=^{NET}$")]);
    assert_eq!(left, "", "the network is gone");
    println!(
        "networks_it: created {SUBNET}, connected, listed in inspect, disconnected, deleted; {WEB} ends on {:?}",
        rig.web_network_names()
    );
}
