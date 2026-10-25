#![allow(clippy::expect_used)]
//! Live Generate and Review changes against the Colima engine (P5-T3 done-when).
//!
//! Every project here is a temp copy of a fixture folder named `berthfx-gen-<n>`, so its Compose
//! project is `berthfx-gen-<n>`, on host ports 56000-56999 (CLAUDE.md, AD33). Each test ends with
//! `docker compose down -v --rmi local` on its own project and touches nothing else.
//!
//! Run from the worktree with `BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only plans_it`
//! under the engine lock.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::actions::{apply_finished, ActionsService, Finished};
use berth_app::services::engines::client_map;
use berth_app::services::plans::{commit, Committed, PendingPlan, PlanRun};
use berth_app::sheets::generate::model::{build_plan, options, Choices};
use berth_app::state::{use_app_state, AppState, PendingState};
use berth_core::ComposeCommand;

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

fn harness() -> (VirtualDom, AppState) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    (dom, state)
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/folders")
        .join(name)
}

/// `<tmp>/berthfx-gen-<n>` holding a copy of the fixture's files.
fn project_copy(fixture_name: &str, n: u16) -> (tempfile::TempDir, PathBuf) {
    let parent = tempfile::tempdir().expect("tmp");
    let folder = parent.path().join(format!("berthfx-gen-{n}"));
    std::fs::create_dir(&folder).expect("folder");
    for entry in std::fs::read_dir(fixture(fixture_name))
        .expect("fixture")
        .flatten()
    {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), folder.join(entry.file_name())).expect("copy");
        }
    }
    (parent, folder)
}

struct Live {
    engine: berth_core::Engine,
    docker: PathBuf,
    host: String,
    service: ActionsService,
    finished: tokio::sync::mpsc::UnboundedReceiver<Finished>,
    store: Arc<berth_store::Store>,
    dom: VirtualDom,
    state: AppState,
}

async fn live() -> Live {
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
    let docker = search
        .find("docker")
        .unwrap_or_else(|| PathBuf::from("docker"));
    let client: Arc<dyn berth_engine::EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let clients = client_map();
    clients
        .lock()
        .expect("clients")
        .insert(engine.id.clone(), client);
    let (service, finished) = ActionsService::new(
        clients,
        Arc::clone(&store),
        Arc::new(berth_engine::CliCompose::new(search)),
        Arc::new(berth_engine::MockColima::bundled()),
        CancellationToken::new(),
    );
    let (dom, state) = harness();
    Live {
        engine,
        docker,
        host,
        service,
        finished,
        store,
        dom,
        state,
    }
}

impl Live {
    fn docker(&self, args: &[&str]) -> String {
        let out = std::process::Command::new(&self.docker)
            .env("DOCKER_HOST", &self.host)
            .args(args)
            .output()
            .expect("docker runs");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// `name state` of every container of the Compose project.
    fn project_containers(&self, project: &str) -> Vec<String> {
        self.docker(&[
            "ps",
            "-a",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
            "--format",
            "{{.Names}} {{.State}}",
        ])
        .lines()
        .map(str::to_string)
        .collect()
    }

    /// `docker compose down -v --rmi local` for one of this test's own projects.
    fn down(&self, folder: &Path) {
        let project = folder
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        assert!(
            project.starts_with("berthfx-gen-"),
            "only berthfx-gen projects: {project}"
        );
        let _ = std::process::Command::new(&self.docker)
            .env("DOCKER_HOST", &self.host)
            .args(["compose", "-p", &project, "--project-directory"])
            .arg(folder)
            .args(["down", "-v", "--rmi", "local", "--remove-orphans"])
            .output();
    }

    /// The pending plan for a copy: choices from the detection, ports moved into 56000-56999.
    fn plan_for(&self, folder: &Path, ports: &[(&str, u16)]) -> PendingPlan {
        let d = berth_rules::detect::detect(folder).expect("detect");
        let mut choices = Choices::from_detection(&d, &[]);
        for (key, port) in ports {
            choices.ports.insert((*key).to_string(), port.to_string());
        }
        let opts = options(&d, &choices, &[]).expect("options");
        let plan = build_plan(&d, &opts).expect("plan");
        let mut pending = PendingPlan::new(plan, "berthfx-gen");
        pending.run = Some(PlanRun::for_folder(
            &self.engine.id,
            folder,
            vec![PathBuf::from("compose.yaml")],
        ));
        pending.pin = Some(self.engine.id.clone());
        pending
    }

    /// Wait for the project action's `Finished`, apply it, and return whether it succeeded.
    async fn wait_finished(&mut self, secs: u64) -> Result<(), String> {
        let fin = tokio::time::timeout(std::time::Duration::from_secs(secs), self.finished.recv())
            .await
            .expect("the project action finished in time")
            .expect("channel open");
        let result = fin.result.clone();
        let mut state = self.state;
        self.dom.in_runtime(|| apply_finished(&mut state, fin));
        result
    }
}

fn env_value(path: &Path, key: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()?.lines().find_map(|l| {
        l.strip_prefix(&format!("{key}="))
            .map(|v| v.trim().to_string())
    })
}

/// Write files only: the files appear, the folder is pinned, and no container exists.
/// A file edited between Generate and Write refuses the write and leaves the folder as it was.
#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn plans_it_write_files_only_writes_validates_pins_and_refuses_a_conflict() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let live = live().await;

    // --- Write files only on a copy of node-next-pnpm. ---
    let (_keep, folder) = project_copy("node-next-pnpm", 1);
    let pending = live.plan_for(&folder, &[("app", 56310), ("db", 56311), ("redis", 56312)]);
    let compose_text = pending
        .plan
        .files
        .iter()
        .find(|f| f.path == Path::new("compose.yaml"))
        .and_then(|f| f.new_text.clone())
        .expect("compose text");
    // Before Write is enabled the new compose text is checked with `docker compose config`.
    let env = berth_app::services::plans::env_for_validation(&pending.plan);
    assert!(
        env.iter().any(|(k, _)| k == "POSTGRES_PASSWORD"),
        "the new .env gives the password the compose text requires"
    );
    live.service
        .validate_compose_text(&folder, &compose_text, &env)
        .await
        .expect("the generated compose text is valid");

    assert!(
        !folder.join("compose.yaml").exists(),
        "nothing is written before the click"
    );
    let done = commit(pending.clone(), Some(Arc::clone(&live.store))).await;
    assert!(
        matches!(done, Committed::Written { count } if count >= 2),
        "{done:?}"
    );
    assert!(folder.join("compose.yaml").is_file());
    let password =
        env_value(&folder.join(".env"), "POSTGRES_PASSWORD").expect("a generated password");
    assert!(
        password.len() >= 16,
        "a generated password, not a placeholder"
    );
    assert!(
        live.project_containers("berthfx-gen-1").is_empty(),
        "Write files only starts nothing"
    );
    let canonical = std::fs::canonicalize(&folder).expect("canonical");
    assert!(live
        .store
        .pins()
        .expect("pins")
        .iter()
        .any(|p| p.folder == canonical));
    assert!(live
        .store
        .recent_folders(5)
        .expect("recent")
        .contains(&canonical));

    // --- A file changed between Generate and Write: refused, nothing written. ---
    let (_keep2, folder2) = project_copy("node-next-pnpm", 2);
    let pending = live.plan_for(&folder2, &[("app", 56320), ("db", 56321), ("redis", 56322)]);
    let edited = "# edited between Generate and Write\nDATABASE_URL=postgres://other\n";
    std::fs::write(folder2.join(".env"), edited).expect("edit .env");
    let done = commit(pending, Some(Arc::clone(&live.store))).await;
    assert_eq!(done, Committed::Conflict(PathBuf::from(".env")));
    assert!(
        !folder2.join("compose.yaml").exists(),
        "a conflict writes nothing"
    );
    assert_eq!(
        std::fs::read_to_string(folder2.join(".env")).expect(".env"),
        edited
    );
    assert!(
        !live
            .store
            .pins()
            .expect("pins")
            .iter()
            .any(|p| p.folder.ends_with("berthfx-gen-2")),
        "a refused write does not pin the folder"
    );

    live.down(&folder);
    live.down(&folder2);
}

/// Write files and start on the node-next-pnpm copy: the project comes up or the run fails with
/// Compose's own words (the fixture's `pnpm build` has no app to build, so a failure is the
/// expected outcome here and the clear message is the check).
#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn plans_it_write_and_start_node_fixture_reaches_running_or_a_clear_failure() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let mut live = live().await;
    let (_keep, folder) = project_copy("node-next-pnpm", 3);
    let pending = live.plan_for(&folder, &[("app", 56330), ("db", 56331), ("redis", 56332)]);
    let run = pending.run.clone().expect("run");
    let command = pending.plan.after_write.clone().expect("after_write");
    assert_eq!(
        command,
        ComposeCommand::Up {
            services: Vec::new(),
            build: true
        }
    );
    let done = commit(pending, Some(Arc::clone(&live.store))).await;
    assert!(matches!(done, Committed::Written { .. }), "{done:?}");

    let key = {
        let mut state = live.state;
        live.dom
            .in_runtime(|| live.service.run_for_folder(&mut state, &run, command))
            .expect("started")
    };
    let pending_now = live
        .dom
        .in_runtime(|| live.state.pending.peek().get(&key).cloned());
    assert!(
        matches!(pending_now, Some(PendingState::Running { .. })),
        "Starting… is pending at once: {pending_now:?}"
    );
    let result = live.wait_finished(600).await;
    let containers = live.project_containers("berthfx-gen-3");
    eprintln!("node fixture start: {result:?}; containers: {containers:?}");
    match result {
        Ok(()) => assert!(
            containers.iter().any(|c| c.ends_with(" running")),
            "Ok means it is up: {containers:?}"
        ),
        Err(message) => {
            assert!(!message.trim().is_empty(), "a failure says why");
            let lines = live.service.compose_failure_lines(&run.project);
            assert!(
                !lines.is_empty(),
                "the output is kept for the Problems detail"
            );
            let failed = live
                .dom
                .in_runtime(|| live.state.pending.peek().get(&key).cloned());
            assert!(
                matches!(failed, Some(PendingState::Failed { .. })),
                "{failed:?}"
            );
        }
    }
    live.down(&folder);
}

/// Write files and start on a copy of static-site (no Dockerfile: the recipe writes one): the
/// project reaches running, answers on its port, and the folder is pinned.
#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn plans_it_write_and_start_static_site_reaches_running() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let mut live = live().await;
    let (_keep, folder) = project_copy("static-site", 4);
    let pending = live.plan_for(&folder, &[("app", 56340)]);
    let run = pending.run.clone().expect("run");
    let command = pending.plan.after_write.clone().expect("after_write");
    let done = commit(pending, Some(Arc::clone(&live.store))).await;
    assert!(matches!(done, Committed::Written { .. }), "{done:?}");
    assert!(
        folder.join("Dockerfile").is_file(),
        "the recipe wrote a Dockerfile"
    );

    {
        let mut state = live.state;
        live.dom
            .in_runtime(|| live.service.run_for_folder(&mut state, &run, command))
            .expect("started");
    }
    let result = live.wait_finished(600).await;
    let containers = live.project_containers("berthfx-gen-4");
    eprintln!("static site start: {result:?}; containers: {containers:?}");
    assert_eq!(result, Ok(()), "{containers:?}");
    assert!(
        containers.iter().any(|c| c.ends_with(" running")),
        "{containers:?}"
    );
    let body = reqwest_free_get("127.0.0.1:56340");
    assert!(body.contains("200"), "the site answers on its port: {body}");
    assert!(
        live.dom.in_runtime(|| live.state.pending.peek().is_empty()),
        "the pending state cleared when the run succeeded"
    );
    live.down(&folder);
}

/// A bare HTTP GET over a TCP socket: the status line (no HTTP client in the app's tree).
fn reqwest_free_get(addr: &str) -> String {
    use std::io::{Read, Write};
    let mut last = String::new();
    for _ in 0..20 {
        if let Ok(mut stream) = std::net::TcpStream::connect(addr) {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(3)));
            let _ = stream.write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n");
            let mut text = String::new();
            let _ = stream.read_to_string(&mut text);
            last = text.lines().next().unwrap_or_default().to_string();
            if last.contains("200") {
                return last;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    last
}
