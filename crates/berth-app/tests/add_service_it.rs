#![allow(clippy::expect_used)]
//! Live Add service against the Colima engine (P5-T9 done-when).
//!
//! The project is a temp copy of `fixtures/compose/berthfx` named `berthfx-add-<n>`, so its Compose
//! project is `berthfx-add-<n>` (never the real `berthfx`), on host ports 56000-56999 (CLAUDE.md,
//! AD33). The test adds Redis and Postgres to it through the sheet's own functions (`submit`, then
//! Review changes' `commit`), checks `docker compose config -q`, starts each new service and ends with
//! `docker compose down -v` on its own project. Nothing else on the engine is touched.
//!
//! Run from the worktree with `BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only add_service_it`
//! under the engine lock.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::actions::{apply_finished, ActionsService, Finished};
use berth_app::services::engines::client_map;
use berth_app::services::plans::{commit, env_for_validation, Committed, PlanStore};
use berth_app::sheets::add_service::model::{plan_inputs, AddError};
use berth_app::sheets::add_service::submit;
use berth_app::state::{use_app_state, AppState};
use berth_core::{Project, ProjectId};

const PROJECT: &str = "berthfx-add-1";

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

/// `<tmp>/berthfx-add-1` holding a copy of the berthfx fixture's files.
fn project_copy() -> (tempfile::TempDir, PathBuf) {
    let parent = tempfile::tempdir().expect("tmp");
    let folder = parent.path().join(PROJECT);
    std::fs::create_dir(&folder).expect("folder");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/compose/berthfx");
    for entry in std::fs::read_dir(source).expect("fixture").flatten() {
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

    /// `docker compose -p berthfx-add-1 …` in the copy; only this test's own project.
    fn compose(&self, folder: &Path, args: &[&str]) -> std::process::Output {
        assert_eq!(
            folder.file_name().and_then(|n| n.to_str()),
            Some(PROJECT),
            "only the berthfx-add copy"
        );
        std::process::Command::new(&self.docker)
            .env("DOCKER_HOST", &self.host)
            .args(["compose", "-p", PROJECT, "--project-directory"])
            .arg(folder)
            .args(args)
            .output()
            .expect("docker compose runs")
    }

    /// `name state` of every container of this test's Compose project.
    fn project_containers(&self) -> Vec<String> {
        self.docker(&[
            "ps",
            "-a",
            "--filter",
            &format!("label=com.docker.compose.project={PROJECT}"),
            "--format",
            "{{.Names}} {{.State}}",
        ])
        .lines()
        .map(str::to_string)
        .collect()
    }

    /// The host ports every running container on the engine publishes.
    fn taken_host_ports(&self) -> Vec<u16> {
        let text = self.docker(&["ps", "--format", "{{.Ports}}"]);
        let mut ports = Vec::new();
        for mapping in text.split(',') {
            // "127.0.0.1:56080->80/tcp" or "0.0.0.0:5432->5432/tcp, [::]:5432->5432/tcp"
            if let Some((left, _)) = mapping.split_once("->") {
                if let Some(port) = left.rsplit(':').next().and_then(|p| p.trim().parse().ok()) {
                    ports.push(port);
                }
            }
        }
        ports
    }

    /// The copy as a project of the engine, the way the Projects list would hold it.
    fn project(&self, folder: &Path) -> Project {
        let canonical = std::fs::canonicalize(folder).expect("canonical");
        let mut p = berth_engine::MockWorld::bundled()
            .projects
            .into_iter()
            .next()
            .expect("a mock project");
        p.id = ProjectId::for_folder(&canonical, &self.engine.id);
        p.name = PROJECT.to_string();
        p.folder = Some(canonical);
        p.compose_files = vec![PathBuf::from("compose.yaml")];
        p.engine = self.engine.id.clone();
        p.services = Vec::new();
        p
    }

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

/// `docker compose down -v` for the test's own project, also when an assertion fails.
struct DownOnDrop {
    docker: PathBuf,
    host: String,
    folder: PathBuf,
}

impl Drop for DownOnDrop {
    fn drop(&mut self) {
        assert_eq!(
            self.folder.file_name().and_then(|n| n.to_str()),
            Some(PROJECT),
            "only the berthfx-add copy"
        );
        let _ = std::process::Command::new(&self.docker)
            .env("DOCKER_HOST", &self.host)
            .args(["compose", "-p", PROJECT, "--project-directory"])
            .arg(&self.folder)
            .args(["down", "-v", "--remove-orphans"])
            .output();
    }
}

fn env_value(path: &Path, key: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()?.lines().find_map(|l| {
        l.strip_prefix(&format!("{key}="))
            .map(|v| v.trim().to_string())
    })
}

fn template(id: &str) -> berth_core::Template {
    berth_rules::templates::built_in()
        .into_iter()
        .find(|t| t.id == id)
        .expect("template")
}

/// Add one template to the copy the way the sheet does (build the plan, park it), check what
/// Review changes would check, write it with Review changes' own writer and start the service.
async fn add_and_start(live: &mut Live, folder: &Path, id: &str, name: &str, port: u16) {
    let project = live.project(folder);
    let t = template(id);
    let plans = PlanStore::default();
    let inputs = plan_inputs(
        &t,
        &[("PORT".to_string(), port.to_string())]
            .into_iter()
            .collect(),
        &[],
        &berth_rules::templates::generate_password(),
    );
    let compose_before = std::fs::read_to_string(folder.join("compose.yaml")).expect("compose");
    let env_before = std::fs::read_to_string(folder.join(".env")).expect(".env");

    // The sheet's button: build the plan, park it. Nothing is written yet.
    let plan_id = submit(&plans, project.clone(), t.clone(), name.to_string(), inputs)
        .await
        .expect("the plan is parked");
    let pending = plans.get(&plan_id).expect("parked plan");
    assert_eq!(
        std::fs::read_to_string(folder.join("compose.yaml")).expect("compose"),
        compose_before,
        "nothing is written before the click on Write"
    );

    // Review changes checks the compose text before Write is enabled.
    let (_, text) =
        berth_app::services::plans::compose_file_of(&pending.plan).expect("compose in plan");
    let env = env_for_validation(&pending.plan);
    live.service
        .validate_compose_text(folder, text, &env)
        .await
        .expect("the new compose text is valid");

    // Write files and start.
    let run = pending.run.clone().expect("a run for the project");
    let command = pending.plan.after_write.clone().expect("after_write");
    assert_eq!(
        command,
        berth_core::ComposeCommand::Up {
            services: vec![name.to_string()],
            build: false
        },
        "start only the new service"
    );
    let done = commit(pending, Some(Arc::clone(&live.store))).await;
    assert!(matches!(done, Committed::Written { .. }), "{done:?}");

    let compose_after = std::fs::read_to_string(folder.join("compose.yaml")).expect("compose");
    assert!(
        compose_after.contains(&format!("  {name}:")),
        "{compose_after}"
    );
    assert!(
        compose_after.contains(&format!("\"{port}:")),
        "the chosen port is written: {compose_after}"
    );
    assert!(
        compose_after.contains("image: nginx:alpine"),
        "the fixture's own services stay"
    );
    let env_after = std::fs::read_to_string(folder.join(".env")).expect(".env");
    assert!(env_after.starts_with(&env_before), "old .env lines stay");

    // `docker compose config -q` passes on what is on disk now.
    let config = live.compose(folder, &["config", "-q"]);
    assert!(
        config.status.success(),
        "docker compose config -q: {}",
        String::from_utf8_lossy(&config.stderr)
    );

    // Start brings the new service up.
    {
        let mut state = live.state;
        live.dom
            .in_runtime(|| live.service.run_for_folder(&mut state, &run, command))
            .expect("started");
    }
    let result = live.wait_finished(300).await;
    assert_eq!(result, Ok(()), "{:?}", live.project_containers());
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn add_service_it_adds_redis_and_postgres_to_a_berthfx_copy_and_starts_them() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let mut live = live().await;
    let (_keep, folder) = project_copy();
    // A leftover of an earlier run goes first (this project name only), and this run's project goes
    // when the test ends, however it ends.
    let _ = live.compose(&folder, &["down", "-v", "--remove-orphans"]);
    let _down = DownOnDrop {
        docker: live.docker.clone(),
        host: live.host.clone(),
        folder: folder.clone(),
    };

    // The fixture's `cache` service is Redis already: that name is taken, another is free.
    {
        let project = live.project(&folder);
        let t = template("redis");
        let inputs = plan_inputs(&t, &Default::default(), &[], "unused");
        let plans = PlanStore::default();
        let taken = submit(&plans, project, t, "cache".to_string(), inputs).await;
        assert_eq!(taken, Err(AddError::NameTaken("cache".to_string())));
        assert!(plans.is_empty(), "a refused name parks nothing");
    }

    // A free 56xxx port, suggested the way the sheet suggests one.
    let taken = live.taken_host_ports();
    let redis_port = berth_rules::ports::suggest(56390, &taken);
    let env_path = folder.join(".env");
    let compose_path = folder.join("compose.yaml");
    let env_original = std::fs::read_to_string(&env_path).expect(".env");
    add_and_start(&mut live, &folder, "redis", "redis", redis_port).await;
    // Redis has no secrets: .env is left exactly as it was. Postgres adds a generated password.
    assert_eq!(
        std::fs::read_to_string(&env_path).expect(".env"),
        env_original,
        "redis adds nothing to .env"
    );
    let mut taken = live.taken_host_ports();
    taken.push(redis_port);
    let pg_port = berth_rules::ports::suggest(redis_port + 1, &taken);
    add_and_start(&mut live, &folder, "postgres", "postgres", pg_port).await;
    let password = env_value(&env_path, "POSTGRES_PASSWORD").expect("a generated password in .env");
    assert_eq!(
        password.len(),
        24,
        "a generated password, not a placeholder"
    );
    assert!(
        !std::fs::read_to_string(&compose_path)
            .expect("compose")
            .contains(&password),
        "the password never reaches compose.yaml"
    );
    assert!(
        env_value(&env_path, "BERTHFX_DB_PASSWORD").is_some(),
        "the fixture's own .env line stays"
    );

    // `docker compose ps` lists the new services running; the fixture's own were never started.
    let ps = live.compose(&folder, &["ps", "--format", "{{.Service}} {{.State}}"]);
    let ps = String::from_utf8_lossy(&ps.stdout).into_owned();
    eprintln!("compose ps:\n{ps}");
    assert!(ps.contains("redis running"), "{ps}");
    assert!(ps.contains("postgres running"), "{ps}");
    assert!(
        !ps.contains("web "),
        "only the added services were started: {ps}"
    );
    let port_line = live.docker(&["port", &format!("{PROJECT}-redis-1")]);
    assert!(
        port_line.contains(&redis_port.to_string()),
        "redis answers on the chosen port: {port_line}"
    );

    let down = live.compose(&folder, &["down", "-v", "--remove-orphans"]);
    assert!(down.status.success());
    assert!(
        live.project_containers().is_empty(),
        "compose down -v removed everything of this project"
    );
}
