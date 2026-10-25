#![allow(clippy::expect_used)]
//! The by-hand checks of P3-T7 as live tests: on a throwaway container
//! `berthfx-scratch-limits` (label `dev.berth.fixture=1`, host port in
//! 56000-56999),
//!
//! (a) memory set to 256 MB applies live (`HostConfig.Memory` is 268435456),
//! (b) a changed port recreates the container and the new port shows,
//! (c) the image `berthfx/does-not-exist:1` gives the failed state and the
//!     old container still runs,
//! (d) a duplicate is named `<name>-copy` and is created, not started,
//! (e) "Recreate with latest image" leaves the container alone when the
//!     pull fails, and (with `BERTH_NET=1`) replaces it when it works.
//!
//! Each test goes the way the UI goes: the container's detail from the
//! engine, the sheet's draft and spec, `offer`, then the actions service.
//!
//! Run with `BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only
//! edit_recreate_it` (live tests share the engine: run them one at a time).
//! Every container and image tag this file makes carries the name
//! `berthfx-scratch-limits…` or `berthfx/…`; it removes them on every exit
//! path and touches nothing else. Colima is never touched.

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::actions::{apply_finished, ActionsService, ContainerTarget, EditOp};
use berth_app::services::engines::client_map;
use berth_app::sheets::edit_recreate::model::{failure_text, offer, Draft, Offer};
use berth_app::state::{use_app_state, AppState, PendingKey, PendingState};
use berth_core::{ContainerDetail, ContainerState, Subject};
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

const SCRATCH: &str = "berthfx-scratch-limits";
const PORT_A: &str = "127.0.0.1:56971:80";
const PORT_B: &str = "127.0.0.1:56972:80";
const PROBE_TAG: &str = "berthfx/pull-probe:1";
const NET: &str = "berthfx-scratch-limits-net";

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

/// Everything the tests share: the engine, the actions service and the
/// docker CLI that checks what the engine really did.
struct Rig {
    dom: VirtualDom,
    state: AppState,
    client: Arc<dyn EngineClient>,
    actions: ActionsService,
    finished: tokio::sync::mpsc::UnboundedReceiver<berth_app::services::actions::Finished>,
    bin: std::path::PathBuf,
    host: String,
}

impl Rig {
    fn cli(&self, args: &[&str]) -> String {
        text(&docker(&self.bin, &self.host, args))
    }

    /// Remove only what carries the scratch name and the fixture label.
    fn clean(&self) {
        clean(&self.bin, &self.host);
    }

    /// Start the scratch container: nginx on `PORT_A`, labelled as a fixture.
    fn start_scratch(&self, image: &str, command: &[&str], publish: Option<&str>) {
        self.start_scratch_with(&[], image, command, publish);
    }

    /// The same with extra `docker run` flags (user, workdir, health, caps).
    fn start_scratch_with(
        &self,
        extra: &[&str],
        image: &str,
        command: &[&str],
        publish: Option<&str>,
    ) {
        remove_scratch_containers(&self.bin, &self.host);
        let mut args = vec![
            "run",
            "-d",
            "--label",
            "dev.berth.fixture=1",
            "--name",
            SCRATCH,
        ];
        if let Some(port) = publish {
            args.extend(["-p", port]);
        }
        args.extend(extra);
        args.push(image);
        args.extend(command);
        let out = docker(&self.bin, &self.host, &args);
        assert!(
            out.status.success(),
            "docker run failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    async fn scratch(&self) -> ContainerDetail {
        let containers = self.client.containers().await.expect("containers");
        let found = containers
            .iter()
            .find(|c| c.name == SCRATCH)
            .expect("the scratch container exists");
        assert_eq!(
            found.labels.get("dev.berth.fixture").map(String::as_str),
            Some("1"),
            "refusing to touch a container without the fixture label"
        );
        self.client.container(&found.id).await.expect("detail")
    }

    /// Put what the UI would hold into the signals.
    async fn load_state(&mut self) {
        let containers = self.client.containers().await.expect("containers");
        let mut state = self.state;
        self.dom.in_runtime(|| state.containers.set(containers));
    }

    async fn run(
        &mut self,
        detail: &ContainerDetail,
        op: EditOp,
    ) -> berth_app::services::actions::Finished {
        self.load_state().await;
        let target = ContainerTarget::from(&detail.container);
        let mut state = self.state;
        let actions = self.actions.clone();
        self.dom
            .in_runtime(|| actions.edit(&mut state, &target, op))
            .expect("the action is accepted");
        tokio::time::timeout(std::time::Duration::from_secs(120), self.finished.recv())
            .await
            .expect("the edit finishes")
            .expect("channel open")
    }

    fn apply(&mut self, done: berth_app::services::actions::Finished) {
        let mut state = self.state;
        self.dom.in_runtime(|| apply_finished(&mut state, done));
    }
}

fn clean(bin: &std::path::Path, host: &str) {
    remove_scratch_containers(bin, host);
    let _ = docker(bin, host, &["rmi", PROBE_TAG]);
    // The scratch network, only when it carries the fixture label.
    let listed = docker(
        bin,
        host,
        &[
            "network",
            "ls",
            "-q",
            "--filter",
            "label=dev.berth.fixture=1",
            "--filter",
            &format!("name={NET}"),
        ],
    );
    for id in text(&listed).lines() {
        let _ = docker(bin, host, &["network", "rm", id.trim()]);
    }
}

/// Containers only: named `berthfx-scratch-limits…` and labelled as fixtures.
fn remove_scratch_containers(bin: &std::path::Path, host: &str) {
    let listed = docker(
        bin,
        host,
        &[
            "ps",
            "-a",
            "-q",
            "--filter",
            "label=dev.berth.fixture=1",
            "--filter",
            &format!("name={SCRATCH}"),
        ],
    );
    for id in text(&listed).lines() {
        let _ = docker(bin, host, &["rm", "-f", id.trim()]);
    }
}

struct CleanOnDrop(std::path::PathBuf, String);

impl Drop for CleanOnDrop {
    fn drop(&mut self) {
        clean(&self.0, &self.1);
    }
}

async fn rig() -> Option<(Rig, CleanOnDrop)> {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return None;
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
    dom.in_runtime(|| state.engines.set(engines.clone()));
    let rig = Rig {
        dom,
        state,
        client,
        actions,
        finished,
        bin,
        host,
    };
    rig.clean();
    Some((rig, guard))
}

fn pending_of(
    rig: &Rig,
    id: &berth_core::ContainerId,
    action: &'static str,
) -> Option<PendingState> {
    let key = PendingKey {
        subject: Subject::Container(id.clone()),
        action,
    };
    let state = rig.state;
    rig.dom
        .in_runtime(|| state.pending.peek().get(&key).cloned())
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn edit_recreate_it_memory_256_mb_applies_live() {
    let Some((mut rig, _guard)) = rig().await else {
        return;
    };
    rig.start_scratch("nginx:alpine", &[], Some(PORT_A));
    let detail = rig.scratch().await;
    let id = detail.container.id.clone();

    // The Limits tab's path: the draft of the container's spec, memory 256m.
    let base = berth_core::ContainerSpec::from_detail(&detail);
    let mut draft = Draft::from_spec(&base);
    draft.memory = "256m".to_string();
    let new = draft.to_spec(&base).expect("a valid draft");
    assert_eq!(offer(&base, &new), Offer::ApplyLimits, "limits only");

    let done = rig.run(&detail, EditOp::apply_limits(&base, &new)).await;
    assert_eq!(done.result, Ok(()), "the engine applied it");
    assert_eq!(
        done.message.as_deref(),
        Some("Limits applied to berthfx-scratch-limits")
    );
    rig.apply(done);

    let memory = rig.cli(&["inspect", "-f", "{{.HostConfig.Memory}}", SCRATCH]);
    println!("edit_recreate_it: HostConfig.Memory={memory}");
    assert_eq!(memory, "268435456");
    // Applied live: the same container, still running, not recreated.
    let after = rig.scratch().await;
    assert_eq!(after.container.id, id);
    assert_eq!(after.container.state, ContainerState::Running);
    assert_eq!(after.limits.memory, Some(bytesize::ByteSize(268_435_456)));
    // And a CPU limit and the restart policy go the same way.
    let base = berth_core::ContainerSpec::from_detail(&after);
    let mut draft = Draft::from_spec(&base);
    draft.cpus = "0.5".to_string();
    draft.restart = berth_app::sheets::edit_recreate::model::RestartKind::UnlessStopped;
    let new = draft.to_spec(&base).expect("a valid draft");
    let done = rig.run(&after, EditOp::apply_limits(&base, &new)).await;
    assert_eq!(done.result, Ok(()));
    rig.apply(done);
    assert_eq!(
        rig.cli(&[
            "inspect",
            "-f",
            "{{.HostConfig.NanoCpus}} {{.HostConfig.RestartPolicy.Name}} {{.HostConfig.Memory}}",
            SCRATCH
        ]),
        "500000000 unless-stopped 268435456"
    );
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn edit_recreate_it_a_changed_port_recreates_and_the_run_settings_survive() {
    let Some((mut rig, _guard)) = rig().await else {
        return;
    };
    // A container with a user, a working directory, a health check, an
    // added capability and an alias on a user network (D16).
    let net = docker(
        &rig.bin,
        &rig.host,
        &["network", "create", "--label", "dev.berth.fixture=1", NET],
    );
    assert!(
        net.status.success(),
        "{}",
        String::from_utf8_lossy(&net.stderr)
    );
    rig.start_scratch_with(
        &[
            "--user",
            "1000:1000",
            "--workdir",
            "/tmp",
            "--cap-add",
            "NET_ADMIN",
            "--health-cmd",
            "true",
            "--health-interval",
            "20s",
            "--health-retries",
            "4",
            "--network",
            NET,
            "--network-alias",
            "svc-alias",
        ],
        "alpine:3",
        &["sleep", "600"],
        Some(PORT_A),
    );
    let inspect_line = |rig: &Rig| {
        rig.cli(&[
            "inspect",
            "-f",
            "{{.Config.User}}|{{.Config.WorkingDir}}|{{.Config.Healthcheck.Test}} {{.Config.Healthcheck.Retries}}|{{.HostConfig.CapAdd}}",
            SCRATCH,
        ])
    };
    let aliases = |rig: &Rig| {
        rig.cli(&[
            "inspect",
            "-f",
            "{{(index .NetworkSettings.Networks \"berthfx-scratch-limits-net\").Aliases}}",
            SCRATCH,
        ])
    };
    let before = inspect_line(&rig);
    println!("edit_recreate_it: before={before}");
    assert!(
        before.starts_with("1000:1000|/tmp|[CMD-SHELL true] 4|[CAP_NET_ADMIN]"),
        "{before}"
    );
    assert!(aliases(&rig).contains("svc-alias"), "{}", aliases(&rig));

    let detail = rig.scratch().await;
    let old_id = detail.container.id.clone();
    assert_eq!(
        berth_core::ContainerSpec::uncarried(&detail),
        Vec::<String>::new(),
        "nothing here is beyond the spec"
    );

    let base = berth_core::ContainerSpec::from_detail(&detail);
    println!("edit_recreate_it: spec aliases={:?}", base.network_aliases);
    assert_eq!(base.user.as_deref(), Some("1000:1000"));
    assert_eq!(base.working_dir.as_deref(), Some("/tmp"));
    // Docker reports the capability with its CAP_ prefix.
    assert_eq!(base.cap_add, ["CAP_NET_ADMIN"]);
    assert!(base.healthcheck.is_some());
    assert_eq!(base.ports.len(), 1);
    let mut draft = Draft::from_spec(&base);
    let row = draft.ports[0].0;
    draft.set_port(row, PORT_B);
    let new = draft.to_spec(&base).expect("a valid draft");
    assert_eq!(
        offer(&base, &new),
        Offer::Recreate,
        "a port needs a recreate"
    );
    assert_eq!(
        berth_core::spec_diff(&base, &new)
            .iter()
            .map(|c| c.field.as_str())
            .collect::<Vec<_>>(),
        ["ports"]
    );

    let mut state = rig.state;
    let done = rig
        .run(
            &detail,
            EditOp::Recreate {
                spec: Box::new(new),
                pull: false,
            },
        )
        .await;
    assert_eq!(done.result, Ok(()));
    let new_id = done.replaced_by.clone().expect("a new container");
    assert_ne!(new_id, old_id);
    rig.dom.in_runtime(|| {
        state.selection.set(berth_app::state::Selection::Containers(
            vec![old_id.clone()],
        ));
    });
    rig.apply(done);
    assert_eq!(
        rig.dom.in_runtime(|| state.selection.peek().clone()),
        berth_app::state::Selection::Containers(vec![new_id.clone()]),
        "the inspector follows the new container"
    );

    let ports = rig.cli(&[
        "ps",
        "--filter",
        &format!("name=^{SCRATCH}$"),
        "--format",
        "{{.Ports}}",
    ]);
    println!("edit_recreate_it: docker ps ports={ports}");
    assert!(ports.contains("127.0.0.1:56972->80/tcp"), "{ports}");
    assert!(!ports.contains("56971"), "{ports}");
    // The four run settings (and the alias) came through the recreate.
    let after = inspect_line(&rig);
    println!(
        "edit_recreate_it: nets={}",
        rig.cli(&[
            "inspect",
            "-f",
            "{{json .NetworkSettings.Networks}}",
            SCRATCH
        ])
    );
    println!("edit_recreate_it: after={after}");
    assert!(
        after.starts_with("1000:1000|/tmp|[CMD-SHELL true] 4|[CAP_NET_ADMIN]"),
        "{after}"
    );
    assert!(aliases(&rig).contains("svc-alias"), "{}", aliases(&rig));
    // One container by that name, running, with the new id; no backup left.
    let names = rig.cli(&[
        "ps",
        "-a",
        "--filter",
        "label=dev.berth.fixture=1",
        "--filter",
        &format!("name={SCRATCH}"),
        "--format",
        "{{.Names}}",
    ]);
    assert_eq!(names, SCRATCH, "no -berth-old left behind");
    let current = rig.scratch().await;
    assert_eq!(current.container.id, new_id);
    assert_eq!(current.container.state, ContainerState::Running);
    assert_eq!(current.user.as_deref(), Some("1000:1000"));
    assert_eq!(current.working_dir.as_deref(), Some("/tmp"));
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn edit_recreate_it_a_missing_image_fails_and_the_old_container_keeps_running() {
    let Some((mut rig, _guard)) = rig().await else {
        return;
    };
    rig.start_scratch("nginx:alpine", &[], Some(PORT_A));
    let detail = rig.scratch().await;
    let old_id = detail.container.id.clone();

    let base = berth_core::ContainerSpec::from_detail(&detail);
    let mut draft = Draft::from_spec(&base);
    draft.image = "berthfx/does-not-exist:1".to_string();
    let new = draft.to_spec(&base).expect("a valid draft");
    assert_eq!(offer(&base, &new), Offer::Recreate);

    let done = rig
        .run(
            &detail,
            EditOp::Recreate {
                spec: Box::new(new),
                pull: false,
            },
        )
        .await;
    let message = done.result.clone().expect_err("the image does not exist");
    println!("edit_recreate_it: failure={message:?}");
    assert!(done.replaced_by.is_none());
    rig.apply(done);

    // The failed state, as the sheet shows it.
    let Some(PendingState::Failed { message }) = pending_of(&rig, &old_id, "container.recreate")
    else {
        panic!("the recreate is in the failed state");
    };
    let shown = failure_text(&message);
    assert!(shown.starts_with("The old container was kept."), "{shown}");

    // The old container is still there, the same one, running, on its port.
    let running = rig.cli(&[
        "inspect",
        "-f",
        "{{.Id}} {{.State.Running}} {{.Config.Image}}",
        SCRATCH,
    ]);
    assert_eq!(running, format!("{} true nginx:alpine", old_id.0));
    let ports = rig.cli(&[
        "ps",
        "--filter",
        &format!("name=^{SCRATCH}$"),
        "--format",
        "{{.Ports}}",
    ]);
    assert!(ports.contains("127.0.0.1:56971->80/tcp"), "{ports}");
    let names = rig.cli(&[
        "ps",
        "-a",
        "--filter",
        "label=dev.berth.fixture=1",
        "--filter",
        &format!("name={SCRATCH}"),
        "--format",
        "{{.Names}}",
    ]);
    assert_eq!(names, SCRATCH, "the backup name was given back");
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn edit_recreate_it_duplicate_is_named_copy_and_created_not_started() {
    let Some((mut rig, _guard)) = rig().await else {
        return;
    };
    rig.start_scratch("nginx:alpine", &[], Some(PORT_A));
    let detail = rig.scratch().await;
    let done = rig.run(&detail, EditOp::Duplicate).await;
    assert_eq!(done.result, Ok(()));
    assert_eq!(
        done.message.as_deref(),
        Some("Duplicated as berthfx-scratch-limits-copy")
    );
    rig.apply(done);
    let copy = format!("{SCRATCH}-copy");
    let state = rig.cli(&[
        "inspect",
        "-f",
        "{{.State.Status}} {{index .Config.Labels \"dev.berth.fixture\"}}",
        &copy,
    ]);
    assert_eq!(state, "created 1", "created, not started, still a fixture");
    // The original is untouched and keeps its port.
    assert_eq!(rig.scratch().await.container.state, ContainerState::Running);
    // A second duplicate finds the next free name.
    let again = rig.run(&detail, EditOp::Duplicate).await;
    assert_eq!(
        again.message.as_deref(),
        Some("Duplicated as berthfx-scratch-limits-copy-2")
    );
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn edit_recreate_it_latest_image_leaves_the_container_alone_when_the_pull_fails() {
    let Some((mut rig, _guard)) = rig().await else {
        return;
    };
    // A tag that exists only here: the registry has no such image.
    assert!(docker(&rig.bin, &rig.host, &["tag", "alpine:3", PROBE_TAG])
        .status
        .success());
    rig.start_scratch(PROBE_TAG, &["sleep", "600"], None);
    let detail = rig.scratch().await;
    let old_id = detail.container.id.clone();
    let spec = berth_core::ContainerSpec::from_detail(&detail);
    let done = rig
        .run(
            &detail,
            EditOp::Recreate {
                spec: Box::new(spec),
                pull: true,
            },
        )
        .await;
    let message = done.result.clone().expect_err("the pull fails");
    println!("edit_recreate_it: pull failure={message:?}");
    assert!(message.contains("Could not pull"), "{message}");
    assert!(message.contains("was not touched"), "{message}");
    rig.apply(done);
    let after = rig.scratch().await;
    assert_eq!(after.container.id, old_id, "nothing was recreated");
    assert_eq!(after.container.state, ContainerState::Running);
}

#[tokio::test]
#[ignore = "needs the live Colima engine and Docker Hub; run with BERTH_IT=1 BERTH_NET=1"]
async fn edit_recreate_it_latest_image_pulls_then_recreates() {
    if std::env::var("BERTH_NET").is_err() {
        eprintln!("BERTH_NET not set, skipping");
        return;
    }
    let Some((mut rig, _guard)) = rig().await else {
        return;
    };
    rig.start_scratch("alpine:3", &["sleep", "600"], None);
    let detail = rig.scratch().await;
    let old_id = detail.container.id.clone();
    let spec = berth_core::ContainerSpec::from_detail(&detail);
    let done = rig
        .run(
            &detail,
            EditOp::Recreate {
                spec: Box::new(spec),
                pull: true,
            },
        )
        .await;
    assert_eq!(done.result, Ok(()));
    let new_id = done.replaced_by.clone().expect("a new container");
    assert_ne!(new_id, old_id);
    rig.apply(done);
    let after = rig.scratch().await;
    assert_eq!(after.container.id, new_id);
    assert_eq!(after.container.state, ContainerState::Running);
    assert_eq!(after.container.image.reference, "alpine:3");
}
