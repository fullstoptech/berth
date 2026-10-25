#![allow(clippy::expect_used)]
//! Live Project detail against the engine (P5-T6 done-when). Run each test
//! on its own with `BERTH_IT=1`, under the engine lock when other sessions
//! share the engine:
//!
//! - `project_detail_it_berthfx_shows_its_services_compose_file_and_folder`
//!   reads the running `berthfx` fixture and changes nothing: four services
//!   in the Services tab's order, the folder's listing with `compose.yaml`
//!   and the `.env` names (never a value), and a Compose file that loads.
//! - `project_detail_it_delete_project_on_a_throwaway_copy` copies the
//!   fixture folder to a temp directory, brings up a throwaway Compose
//!   project `berthfx-del-<n>` (one built service, a named volume, one
//!   published port in 56000-56999), then runs Delete project through the
//!   sheet's own confirm path twice: with both boxes off (containers and
//!   network go, the volume and the image stay) and with both on (the volume
//!   and the image go). The fixture project `berthfx` is never named in a
//!   write, and everything of the throwaway is removed on every exit path.

mod common;

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::screens::project_detail::{detail_model, project_container_ids};
use berth_app::services::actions::{apply_finished, ActionsService, Finished, StateView};
use berth_app::services::compose;
use berth_app::services::confirm::{project_delete_effect, project_holdings, DeleteOptions};
use berth_app::services::engines::client_map;
use berth_app::services::project_files::{DiskFs, ProjectFs};
use berth_app::sheets::delete_project::{confirm_delete, delete_key};
use berth_app::state::{use_app_state, AppState, Overlay};
use berth_core::{ComposeCommand, ComposeTarget, Engine, Project};
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

struct Live {
    dom: VirtualDom,
    state: AppState,
    engine: Engine,
    client: Arc<dyn EngineClient>,
    search: berth_sys::SearchPath,
    runner: berth_engine::CliCompose,
    service: ActionsService,
    finished: tokio::sync::mpsc::UnboundedReceiver<Finished>,
}

async fn connect() -> Live {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
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
    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let clients = client_map();
    clients
        .lock()
        .expect("clients")
        .insert(engine.id.clone(), Arc::clone(&client));
    let (service, finished) = ActionsService::new(
        clients,
        Arc::new(berth_store::Store::open_in_memory().expect("store")),
        Arc::new(berth_engine::CliCompose::new(search.clone())),
        Arc::new(berth_engine::MockColima::bundled()),
        CancellationToken::new(),
    );
    dom.in_runtime(|| {
        let mut state = state;
        state.engines.set(engines.clone());
    });
    Live {
        dom,
        state,
        engine,
        client,
        runner: berth_engine::CliCompose::new(search.clone()),
        search,
        service,
        finished,
    }
}

impl Live {
    /// Read the engine's lists into the state, as a refresh does, and return the projects.
    async fn refresh(&self) -> Vec<Project> {
        let containers = self.client.containers().await.expect("containers");
        let volumes = self.client.volumes().await.expect("volumes");
        let projects = berth_engine::projects::group(&self.engine.id, &containers, &[], false);
        self.dom.in_runtime(|| {
            let mut state = self.state;
            state.containers.set(containers.clone());
            state.volumes.set(volumes.clone());
            state.projects.set(projects.clone());
        });
        projects
    }
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn project_detail_it_berthfx_shows_its_services_compose_file_and_folder() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    common::settle_fixture().await;
    let live = connect().await;
    let projects = live.refresh().await;
    let project = projects
        .iter()
        .find(|p| p.name == "berthfx")
        .expect("the berthfx project")
        .clone();

    // Four services, in the Services tab's order.
    let containers = live.client.containers().await.expect("containers");
    let ids = project_container_ids(&project, &containers);
    let names: Vec<String> = ids
        .iter()
        .filter_map(|id| containers.iter().find(|c| &c.id == id))
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(
        names,
        [
            "berthfx-cache-1",
            "berthfx-db-1",
            "berthfx-logger-1",
            "berthfx-web-1"
        ]
    );

    // The header model, as the screen builds it.
    let model = live.dom.in_runtime(|| {
        let engines = live.state.engines.peek();
        let projects = live.state.projects.peek();
        let containers = live.state.containers.peek();
        let pending = live.state.pending.peek();
        detail_model(
            &project,
            &StateView {
                engines: &engines,
                containers: &containers,
                projects: &projects,
                images: &[],
                volumes: &[],
                networks: &[],
                pending: &pending,
                backing: berth_app::services::backing::Backing::Live,
            },
            &[],
            &berth_sys::paths::home(),
        )
    });
    assert_eq!(model.row.services_label, "4 services");
    assert!(model.folder.is_some(), "a folder on this Mac");

    // The folder: its Compose file first, then the `.env` and what else is there.
    let folder = project.folder.clone().expect("folder");
    assert!(!project.compose_files.is_empty(), "a Compose file is known");
    let fs = DiskFs::new(&folder);
    let listing = fs.list("").expect("the folder lists");
    let listed: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(listed.first().copied(), Some("compose.yaml"), "{listed:?}");
    assert!(listed.contains(&".env"), "{listed:?}");
    let keys = fs.env_keys(".env").expect("env keys");
    assert!(
        keys.contains(&"BERTHFX_DB_PASSWORD".to_string()),
        "{keys:?}"
    );
    assert!(
        fs.preview(".env").is_err(),
        "the text of the env file is never previewed"
    );
    // The compose file loads, through the real Compose.
    live.service
        .validate_compose(&folder, &project.compose_files)
        .await
        .expect("compose config -q passes");
    println!(
        "project_detail_it: services={names:?} label={:?} folder={} listing={listed:?} env_keys={}",
        model.row.services_label,
        folder.display(),
        keys.len()
    );
}

const THROWAWAY_COMPOSE: &str = "services:
  app:
    build: .
    labels: [dev.berth.fixture=1]
    command: [\"sleep\", \"600\"]
    ports:
      - \"127.0.0.1:PORT:80\"
    volumes:
      - data:/data
volumes:
  data:
    labels: [dev.berth.fixture=1]
networks:
  default:
    labels: [dev.berth.fixture=1]
";

const THROWAWAY_DOCKERFILE: &str = "FROM alpine:3\nLABEL dev.berth.fixture=1\n";

/// Removes everything the throwaway owns, by its own project name only, when
/// it goes out of scope: after the last assertion and on a panic.
struct Cleanup {
    docker: std::path::PathBuf,
    host: String,
    name: String,
}

impl Cleanup {
    fn run(&self, args: &[&str]) -> std::process::Output {
        std::process::Command::new(&self.docker)
            .env("DOCKER_HOST", &self.host)
            .args(args)
            .output()
            .expect("docker CLI runs")
    }

    fn sweep(&self) {
        let name = &self.name;
        assert!(
            name.starts_with("berthfx-del-"),
            "only a throwaway is removed"
        );
        let by_label = format!("label=com.docker.compose.project={name}");
        let ids = self.run(&["ps", "-aq", "--filter", &by_label]);
        for id in String::from_utf8_lossy(&ids.stdout).split_whitespace() {
            let _ = self.run(&["rm", "-f", "-v", id]);
        }
        let _ = self.run(&["network", "rm", &format!("{name}_default")]);
        let _ = self.run(&["volume", "rm", &format!("{name}_data")]);
        let _ = self.run(&["rmi", &format!("{name}-app")]);
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.sweep();
    }
}

async fn up(live: &Live, target: &ComposeTarget) {
    compose::run(
        &live.runner,
        target,
        ComposeCommand::Up {
            services: Vec::new(),
            build: true,
        },
        CancellationToken::new(),
    )
    .await
    .map_err(|failure| failure.message)
    .expect("the throwaway comes up");
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn project_detail_it_delete_project_on_a_throwaway_copy() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let mut live = connect().await;
    let n = std::process::id() % 90 + 10;
    let name = format!("berthfx-del-{n}");
    assert!(
        name.starts_with("berthfx-del-"),
        "only a throwaway is ever deleted"
    );
    let port = 56700 + n;

    // A temp copy of the fixture folder, its Compose file replaced by the throwaway's.
    let dir = tempfile::tempdir().expect("tempdir");
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/compose/berthfx");
    for entry in std::fs::read_dir(&source).expect("fixture folder") {
        let entry = entry.expect("entry");
        std::fs::copy(entry.path(), dir.path().join(entry.file_name())).expect("copy");
    }
    std::fs::write(
        dir.path().join("compose.yaml"),
        THROWAWAY_COMPOSE.replace("PORT", &port.to_string()),
    )
    .expect("compose");
    std::fs::write(dir.path().join("Dockerfile"), THROWAWAY_DOCKERFILE).expect("dockerfile");
    let target = ComposeTarget {
        engine: live.engine.id.clone(),
        docker_host: live.engine.docker_host.clone().expect("host"),
        project_name: name.clone(),
        folder: dir.path().to_path_buf(),
        files: vec![dir.path().join("compose.yaml")],
    };
    let cleanup = Cleanup {
        docker: live
            .search
            .find("docker")
            .unwrap_or_else(|| std::path::PathBuf::from("docker")),
        host: live.engine.docker_host.clone().expect("host"),
        name: name.clone(),
    };
    cleanup.sweep();

    let outcome = async {
        let mut report = Vec::new();
        for (round, options) in [
            DeleteOptions::default(),
            DeleteOptions {
                volumes: true,
                remove_images: true,
            },
        ]
        .into_iter()
        .enumerate()
        {
            up(&live, &target).await;
            let projects = live.refresh().await;
            let project = projects
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("{name} is listed"))
                .clone();
            let holdings = live.dom.in_runtime(|| {
                project_holdings(
                    &project,
                    &live.state.containers.peek(),
                    &live.state.volumes.peek(),
                )
            });
            assert_eq!(holdings.containers, [format!("{name}-app-1")]);
            assert_eq!(holdings.volumes.len(), 1, "{holdings:?}");
            assert_eq!(holdings.volumes[0].0, format!("{name}_data"));
            let effect = project_delete_effect(&project, &holdings, None, options);
            assert_eq!(effect.typed_name.is_some(), options.volumes);

            // The sheet's own confirm path, against the real Compose and engine.
            let launched = live.dom.in_runtime(|| {
                let mut state = live.state;
                state
                    .overlays
                    .write()
                    .push(Overlay::DeleteProject(project.id.clone()));
                confirm_delete(&mut state, Some(&live.service), &project.id, options)
            });
            assert_eq!(launched.keys, [delete_key(&project.id)]);
            let done =
                tokio::time::timeout(std::time::Duration::from_secs(90), live.finished.recv())
                    .await
                    .expect("the delete finishes")
                    .expect("channel open");
            assert_eq!(done.result, Ok(()), "round {round}");
            live.dom.in_runtime(|| {
                let mut state = live.state;
                apply_finished(&mut state, done);
            });

            let containers = live.client.containers().await.expect("containers");
            let volumes = live.client.volumes().await.expect("volumes");
            let images = live.client.images().await.expect("images");
            let has_container = containers.iter().any(|c| c.name == format!("{name}-app-1"));
            let has_volume = volumes.iter().any(|v| v.name == format!("{name}_data"));
            let has_image = images.iter().any(|i| {
                i.repo_tags
                    .iter()
                    .any(|t| t.starts_with(&format!("{name}-app")))
            });
            let has_network = live
                .client
                .networks()
                .await
                .expect("networks")
                .iter()
                .any(|n| n.name == format!("{name}_default"));
            assert!(!has_container, "round {round}: containers go");
            assert!(!has_network, "round {round}: the network goes");
            assert_eq!(
                has_volume, !options.volumes,
                "round {round}: volume kept only when unticked"
            );
            assert_eq!(
                has_image, !options.remove_images,
                "round {round}: image kept only when unticked"
            );
            assert!(
                dir.path().join("compose.yaml").exists(),
                "the project folder is never touched"
            );
            report.push((round, has_container, has_volume, has_image, has_network));
        }
        report
    }
    .await;
    drop(cleanup);
    println!(
        "project_detail_it: delete rounds (round, container, volume, image, network) = {outcome:?}"
    );
}
