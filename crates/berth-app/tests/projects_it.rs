#![allow(clippy::expect_used)]
//! Live Projects screen against the berthfx fixture (P2-T6 done-when).
//! Run with `scripts/it.sh -p berth-app projects_it` (sets BERTH_IT=1 and
//! brings the fixtures up first).
//!
//! The row model built from the live signals shows berthfx with four
//! running services; the row's own click path (`run_project_op`) stops the
//! project: all four containers are Exited within 5 s, the row shows the
//! pending "Stopping…" label at once and offers Start project afterwards;
//! Start project brings all four back. Colima itself is never touched (the
//! Colima runner is the mock) and the four fixture containers are started
//! again on every exit path.

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::actions::{
    apply_finished, with_view, ActionsService, Finished, ProjectOp,
};
use berth_app::services::engines::{client_map, run_engine, Applier, EngineUpdate};
use berth_app::state::{use_app_state, AppState};
use berth_app::ui::project_row::{row_model, run_project_op, PrimaryOp, ProjectRowModel, RowKind};
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

const FIXTURE: [&str; 4] = [
    "berthfx-web-1",
    "berthfx-db-1",
    "berthfx-cache-1",
    "berthfx-logger-1",
];

struct Env {
    dom: VirtualDom,
    state: AppState,
    applier: Applier,
    updates: tokio::sync::mpsc::Receiver<EngineUpdate>,
    finished: tokio::sync::mpsc::UnboundedReceiver<Finished>,
}

impl Env {
    fn all_state(&self, want: ContainerState) -> bool {
        self.dom.in_runtime(|| {
            let containers = self.state.containers.peek();
            FIXTURE.iter().all(|name| {
                containers
                    .iter()
                    .find(|c| c.name == *name)
                    .is_some_and(|c| c.state == want)
            })
        })
    }

    fn pending_count(&self) -> usize {
        self.dom.in_runtime(|| self.state.pending.peek().len())
    }

    /// The berthfx row exactly as the Projects screen builds it.
    fn berthfx_row(&self) -> Option<ProjectRowModel> {
        self.dom.in_runtime(|| {
            let projects = self.state.projects.peek();
            let project = projects.iter().find(|p| p.name == "berthfx")?;
            let problems = self.state.problems.peek();
            Some(with_view(&self.state, |view| {
                row_model(project, view, &problems, &berth_sys::paths::home())
            }))
        })
    }

    async fn pump_until(
        &mut self,
        budget: std::time::Duration,
        done: impl Fn(&Self) -> bool,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            if done(self) {
                return true;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return false;
            }
            tokio::select! {
                () = tokio::time::sleep(deadline - now) => {}
                update = self.updates.recv() => {
                    if let Some(update) = update {
                        let mut state = self.state;
                        self.dom.in_runtime(|| self.applier.apply(&mut state, update));
                    }
                }
                fin = self.finished.recv() => {
                    if let Some(fin) = fin {
                        let mut state = self.state;
                        self.dom.in_runtime(|| apply_finished(&mut state, fin));
                    }
                }
            }
        }
    }
}

fn run_docker(docker: &std::path::Path, host: &str, args: &[&str]) -> std::process::Output {
    std::process::Command::new(docker)
        .env("DOCKER_HOST", host)
        .args(args)
        .output()
        .expect("docker CLI runs")
}

/// Recreate the four fixture containers from this tree. The fixtures are
/// shared between worktrees and Compose records the folder it ran from on
/// each container, so containers created from another tree would show as a
/// second "berthfx" project; the Projects screen is right to list them
/// apart, and this test needs one project.
fn recreate_fixture_from_this_tree(docker: &std::path::Path, host: &str) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/compose/berthfx")
        .canonicalize()
        .expect("fixture dir");
    let out = std::process::Command::new(docker)
        .env("DOCKER_HOST", host)
        .arg("compose")
        .arg("--project-directory")
        .arg(&dir)
        .arg("-f")
        .arg(dir.join("compose.yaml"))
        .args(["-p", "berthfx", "up", "-d", "--force-recreate", "--wait"])
        .output()
        .expect("docker compose runs");
    assert!(
        out.status.success(),
        "fixture recreate failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1 via scripts/it.sh"]
async fn projects_it_berthfx_row_shows_four_running_and_stop_project_stops_all_within_5s() {
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
    let docker = search
        .find("docker")
        .unwrap_or_else(|| std::path::PathBuf::from("docker"));
    recreate_fixture_from_this_tree(&docker, &host);
    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));

    let clients = client_map();
    clients
        .lock()
        .expect("clients")
        .insert(engine.id.clone(), Arc::clone(&client));
    let cancel = CancellationToken::new();
    let (service, finished) = ActionsService::new(
        clients,
        Arc::clone(&store),
        Arc::new(berth_engine::CliCompose::new(search.clone())),
        Arc::new(berth_engine::MockColima::bundled()),
        cancel.clone(),
    );

    let mut applier = Applier::new(Vec::new());
    dom.in_runtime(|| {
        applier.apply(&mut state, EngineUpdate::EnginesDiscovered(engines.clone()));
    });
    let (tx, updates) = tokio::sync::mpsc::channel(256);
    let engine_task = tokio::spawn(run_engine(
        Arc::clone(&client),
        Arc::clone(&store),
        Vec::new(),
        false,
        tx,
        cancel.clone(),
        std::time::Duration::from_millis(200),
    ));
    let mut env = Env {
        dom,
        state,
        applier,
        updates,
        finished,
    };

    // Fixture up first (a previous run may have left it stopped).
    let _ = env
        .pump_until(std::time::Duration::from_secs(10), |e| {
            e.berthfx_row().is_some()
        })
        .await;
    if !env.all_state(ContainerState::Running) {
        for name in FIXTURE {
            let _ = run_docker(&docker, &host, &["start", name]);
        }
    }
    let ready = env
        .pump_until(std::time::Duration::from_secs(20), |e| {
            e.all_state(ContainerState::Running)
        })
        .await;

    // --- The row: four services, four green dots, Stop project offered. ---
    let row_running = env.berthfx_row().expect("berthfx row");
    env.dom.in_runtime(|| {
        let projects = env.state.projects.peek();
        let containers = env.state.containers.peek();
        for p in projects.iter().filter(|p| p.name == "berthfx") {
            eprintln!(
                "DIAG project {} services={:?}",
                p.name,
                p.services
                    .iter()
                    .map(|s| (s.name.clone(), s.state, s.containers.len()))
                    .collect::<Vec<_>>()
            );
        }
        for c in containers.iter().filter(|c| c.name.contains("berthfx")) {
            eprintln!(
                "DIAG container {} {:?} svc={:?} proj={:?}",
                c.name, c.state, c.service, c.compose_project
            );
        }
    });

    // --- Stop project through the row's click path. ---
    let project = env
        .dom
        .in_runtime(|| {
            env.state
                .projects
                .peek()
                .iter()
                .find(|p| p.name == "berthfx")
                .cloned()
        })
        .expect("berthfx project");
    let t0 = std::time::Instant::now();
    {
        let mut state = env.state;
        env.dom.in_runtime(|| {
            run_project_op(&mut state, Some(&service), &project.id, ProjectOp::Stop);
        });
    }
    let row_stopping = env.berthfx_row().expect("berthfx row while stopping");
    let all_exited = env
        .pump_until(std::time::Duration::from_secs(5), |e| {
            e.all_state(ContainerState::Exited)
        })
        .await;
    let stopped_in = t0.elapsed();
    let _ = env
        .pump_until(std::time::Duration::from_secs(10), |e| {
            e.pending_count() == 0
        })
        .await;
    let row_stopped = env.berthfx_row().expect("berthfx row after stop");

    // --- Start project brings all four back. ---
    let project = env
        .dom
        .in_runtime(|| {
            env.state
                .projects
                .peek()
                .iter()
                .find(|p| p.name == "berthfx")
                .cloned()
        })
        .expect("berthfx project");
    {
        let mut state = env.state;
        env.dom.in_runtime(|| {
            run_project_op(&mut state, Some(&service), &project.id, ProjectOp::Start);
        });
    }
    let all_running = env
        .pump_until(std::time::Duration::from_secs(60), |e| {
            e.pending_count() == 0 && e.all_state(ContainerState::Running)
        })
        .await;
    // Health checks settle a few seconds after the containers run; the dots
    // stay amber (starting) until then, which is the right thing to show.
    let _ = env
        .pump_until(std::time::Duration::from_secs(30), |e| {
            e.berthfx_row().is_some_and(|row| {
                row.dots.len() == 4 && row.dots.iter().all(|d| d.tone == berth_core::Tone::Success)
            })
        })
        .await;
    let row_back = env.berthfx_row().expect("berthfx row after start");

    // Leave the fixtures running whatever happened above.
    let mut restored = Vec::new();
    for name in FIXTURE {
        let out = run_docker(&docker, &host, &["start", name]);
        restored.push(out.status.success());
    }
    cancel.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), engine_task).await;

    println!(
        "projects_it: ready={ready} dots_running={} label={:?} note={:?} stopping_pending={:?} \
         all_exited={all_exited} in {stopped_in:?} stopped_primary={:?} all_running={all_running} \
         back_dots={}",
        row_running.dots.len(),
        row_running.services_label,
        row_running.note,
        row_stopping.pending,
        row_stopped.primary,
        row_back.dots.len(),
    );

    assert!(ready, "all four fixture containers running first");
    assert_eq!(row_running.kind, RowKind::Normal);
    assert_eq!(row_running.services_label, "4 services");
    assert_eq!(row_running.dots.len(), 4);
    assert!(
        row_running
            .dots
            .iter()
            .all(|d| d.tone == berth_core::Tone::Success),
        "four green dots: {:?}",
        row_running.dots
    );
    assert_eq!(row_running.primary, PrimaryOp::Stop);
    assert_eq!(
        row_stopping.pending.as_ref().map(|p| p.label.as_str()),
        Some("Stopping…"),
        "the row shows Stopping… before the engine answered"
    );
    assert!(all_exited, "Stop project stopped all four within 5 s");
    assert!(
        stopped_in <= std::time::Duration::from_secs(5),
        "took {stopped_in:?}"
    );
    assert_eq!(row_stopped.primary, PrimaryOp::Start);
    assert!(all_running, "Start project brought all four back");
    assert_eq!(row_back.dots.len(), 4);
    assert!(row_back
        .dots
        .iter()
        .all(|d| d.tone == berth_core::Tone::Success));
    assert!(restored.iter().all(|ok| *ok), "fixtures left running");
}
