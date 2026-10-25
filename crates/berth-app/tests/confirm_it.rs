#![allow(clippy::expect_used)]
//! The delete dialog's by-hand check as a test (P2-T8 done-when): "delete
//! a `berthfx-scratch` container through the dialog; it is removed and the
//! result text repeats the dialog's count". The test builds the dialog's
//! `ConfirmModel` for a throwaway fixture container, runs it the way the
//! Delete button does (`confirm::run` through the actions service), and
//! checks the container is gone and the result text repeats the button's
//! count. Nobody needs Accessibility permission.
//!
//! Run with `scripts/it.sh -p berth-app confirm_it` (sets BERTH_IT=1 and
//! brings the fixtures up first). The test creates `berthfx-scratch-1`
//! (label `dev.berth.fixture=1`), refuses to delete anything without that
//! label, and removes the scratch container again on every exit path. No
//! other container is touched; Colima is never touched.

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::actions::{apply_finished, ActionsService};
use berth_app::services::confirm;
use berth_app::services::engines::client_map;
use berth_app::state::{use_app_state, AppState, Overlay};
use berth_core::ContainerState;
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

const SCRATCH: &str = "berthfx-scratch-1";

fn docker(path: &std::path::Path, host: &str, args: &[&str]) -> std::process::Output {
    std::process::Command::new(path)
        .env("DOCKER_HOST", host)
        .args(args)
        .output()
        .expect("docker CLI runs")
}

/// Removes the scratch container when the test ends, whatever happened.
struct Scratch {
    docker: std::path::PathBuf,
    host: String,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = docker(&self.docker, &self.host, &["rm", "-f", SCRATCH]);
    }
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1 via scripts/it.sh"]
async fn confirm_it_delete_berthfx_scratch_through_the_dialog_model() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
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
    let docker_bin = search
        .find("docker")
        .unwrap_or_else(|| std::path::PathBuf::from("docker"));
    let _cleanup = Scratch {
        docker: docker_bin.clone(),
        host: host.clone(),
    };

    // A throwaway running container, labelled as a fixture.
    let _ = docker(&docker_bin, &host, &["rm", "-f", SCRATCH]);
    let created = docker(
        &docker_bin,
        &host,
        &[
            "run",
            "-d",
            "--label",
            "dev.berth.fixture=1",
            "--name",
            SCRATCH,
            "alpine:3.20",
            "sleep",
            "600",
        ],
    );
    assert!(
        created.status.success(),
        "docker run failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );

    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let containers = client.containers().await.expect("containers");
    let scratch = containers
        .iter()
        .find(|c| c.name == SCRATCH)
        .expect("the scratch container exists")
        .clone();
    assert_eq!(
        scratch.labels.get("dev.berth.fixture").map(String::as_str),
        Some("1"),
        "refusing to delete a container without the fixture label"
    );
    assert_eq!(scratch.state, ContainerState::Running);
    let volumes = client.volumes().await.expect("volumes");

    dom.in_runtime(|| {
        state.engines.set(engines.clone());
        state.containers.set(containers.clone());
        state.volumes.set(volumes.clone());
    });

    // The dialog the Delete button opens for this one container.
    let ids = [scratch.id.clone()];
    let model = dom
        .in_runtime(|| confirm::delete_model(&state, &ids))
        .expect("a delete dialog for the scratch container");
    let button = confirm::button_label(&model.effect);
    assert_eq!(model.title, format!("Delete container {SCRATCH}?"));
    assert_eq!(button, "Stop and delete", "it is running");
    assert_eq!(model.effect.count, 1);
    dom.in_runtime(|| {
        state.overlays.write().push(Overlay::Confirm(model.clone()));
    });

    let clients = client_map();
    clients
        .lock()
        .expect("clients")
        .insert(engine.id.clone(), Arc::clone(&client));
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let (service, mut finished) = ActionsService::new(
        clients,
        store,
        Arc::new(berth_engine::CliCompose::new(search.clone())),
        Arc::new(berth_engine::MockColima::bundled()),
        CancellationToken::new(),
    );

    // "Stop and delete" pressed, anonymous volumes left unchecked.
    let launched = dom.in_runtime(|| confirm::run(&mut state, &service, &model, false));
    assert_eq!(launched.refused, Vec::<String>::new());
    assert_eq!(launched.keys.len(), 1);
    let done = tokio::time::timeout(std::time::Duration::from_secs(30), finished.recv())
        .await
        .expect("the delete finishes")
        .expect("channel open");
    assert_eq!(done.result, Ok(()), "the engine removed it");
    dom.in_runtime(|| apply_finished(&mut state, done));

    let pending = dom.in_runtime(|| state.pending.peek().clone());
    let succeeded = confirm::finished_count(&pending, &launched.keys).expect("nothing runs now");
    let result = confirm::result_label(&model.effect, succeeded);

    let listed = docker(
        &docker_bin,
        &host,
        &["ps", "-a", "-q", "--filter", &format!("name=^{SCRATCH}$")],
    );
    let still_there = String::from_utf8_lossy(&listed.stdout).trim().to_string();
    let after = client.containers().await.expect("containers again");
    println!(
        "confirm_it: title={:?} button={button:?} result={result:?} left_in_docker={still_there:?}",
        model.title
    );

    assert!(still_there.is_empty(), "docker ps -a still lists it");
    assert!(
        after.iter().all(|c| c.name != SCRATCH),
        "the engine's list still has it"
    );
    assert_eq!(result, "Deleted 1 container");
    assert_eq!(
        model.effect.count as usize, succeeded,
        "the result repeats the dialog's count"
    );
}
