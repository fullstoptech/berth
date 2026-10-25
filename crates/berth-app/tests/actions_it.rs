#![allow(clippy::expect_used)]
//! Live actions service against the berthfx fixture (P2-T5 done-when).
//! Run with `scripts/it.sh -p berth-app actions_it` (sets BERTH_IT=1 and
//! brings the fixtures up first). One test, so the three scenarios run in
//! order against the same containers:
//!
//! 1. Stop `berthfx-web-1`: "Stopping…" is pending before the engine
//!    answers, and the signal shows Exited within 2 s.
//! 2. Restart three berthfx containers in one call.
//! 3. Stop project then Start project on berthfx (real Compose).
//!
//! Colima itself is never touched (the Colima runner is the mock), and the
//! four fixture containers are started again on every exit path.

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::actions::{
    apply_finished, ActionsService, ContainerTarget, Finished, ProjectOp,
};
use berth_app::services::engines::{client_map, run_engine, Applier, EngineUpdate};
use berth_app::state::{use_app_state, AppState, PendingKey, PendingState};
use berth_core::{ContainerAction, ContainerState, Subject};
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

fn harness() -> (VirtualDom, AppState) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    (dom, state)
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
    fn container_state(&self, name: &str) -> Option<ContainerState> {
        self.dom.in_runtime(|| {
            self.state
                .containers
                .peek()
                .iter()
                .find(|c| c.name == name)
                .map(|c| c.state)
        })
    }

    fn pending(&self) -> Vec<(PendingKey, PendingState)> {
        self.dom.in_runtime(|| {
            self.state
                .pending
                .peek()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
    }

    fn all_state(&self, want: ContainerState) -> bool {
        FIXTURE
            .iter()
            .all(|name| self.container_state(name) == Some(want))
    }

    /// Apply engine updates and finished actions until `done` holds or the
    /// budget runs out. Returns whether `done` held.
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

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1 via scripts/it.sh"]
async fn actions_it_stop_restart_three_and_project_cycle_on_berthfx() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let (dom, mut state) = harness();
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

    // Make sure the fixture is up and in the signal before we start.
    let _ = env
        .pump_until(std::time::Duration::from_secs(10), |e| {
            e.container_state("berthfx-web-1").is_some()
        })
        .await;
    if !env.all_state(ContainerState::Running) {
        for name in FIXTURE {
            let _ = run_docker(&docker, &host, &["start", name]);
        }
    }
    let ready = env
        .pump_until(std::time::Duration::from_secs(15), |e| {
            e.all_state(ContainerState::Running)
        })
        .await;

    // --- 1. Stop web: Stopping… first, Exited within 2 s. ---
    let web = env
        .dom
        .in_runtime(|| {
            env.state
                .containers
                .peek()
                .iter()
                .find(|c| c.name == "berthfx-web-1")
                .map(ContainerTarget::from)
        })
        .expect("web in the signal");
    let web_key = PendingKey {
        subject: Subject::Container(web.id.clone()),
        action: "container.stop",
    };
    let t0 = std::time::Instant::now();
    let accepted = {
        let mut state = env.state;
        env.dom.in_runtime(|| {
            service.container(&mut state, &web, ContainerAction::Stop { timeout_s: None })
        })
    };
    let pending_right_after = env.pending();
    let exited = env
        .pump_until(std::time::Duration::from_secs(2), |e| {
            e.container_state("berthfx-web-1") == Some(ContainerState::Exited)
        })
        .await;
    let exited_in = t0.elapsed();
    let _ = env
        .pump_until(std::time::Duration::from_secs(3), |e| {
            e.pending().is_empty()
        })
        .await;
    let pending_after_stop = env.pending();

    // --- 2. Restart three containers in one call. ---
    let _ = run_docker(&docker, &host, &["start", "berthfx-web-1"]);
    let web_back = env
        .pump_until(std::time::Duration::from_secs(10), |e| {
            e.container_state("berthfx-web-1") == Some(ContainerState::Running)
        })
        .await;
    let three: Vec<ContainerTarget> = env.dom.in_runtime(|| {
        env.state
            .containers
            .peek()
            .iter()
            .filter(|c| {
                ["berthfx-web-1", "berthfx-db-1", "berthfx-cache-1"].contains(&c.name.as_str())
            })
            .map(ContainerTarget::from)
            .collect()
    });
    let results = {
        let mut state = env.state;
        env.dom.in_runtime(|| {
            service.containers(
                &mut state,
                &three,
                &ContainerAction::Restart { timeout_s: Some(5) },
            )
        })
    };
    let pending_three = env.pending().len();
    let restarted = env
        .pump_until(std::time::Duration::from_secs(30), |e| {
            e.pending().is_empty() && e.all_state(ContainerState::Running)
        })
        .await;
    let pending_after_restart = env.pending();

    // --- 3. Stop project, then Start project. ---
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
    let stop_project = {
        let mut state = env.state;
        env.dom
            .in_runtime(|| service.project(&mut state, &project, ProjectOp::Stop))
    };
    let stop_pending = env.pending();
    let all_exited = env
        .pump_until(std::time::Duration::from_secs(30), |e| {
            e.pending().is_empty() && e.all_state(ContainerState::Exited)
        })
        .await;
    let stopped_status = env.dom.in_runtime(|| {
        env.state
            .projects
            .peek()
            .iter()
            .find(|p| p.name == "berthfx")
            .map(|p| p.status)
    });
    let start_project = {
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
        let mut state = env.state;
        env.dom
            .in_runtime(|| service.project(&mut state, &project, ProjectOp::Start))
    };
    let all_running = env
        .pump_until(std::time::Duration::from_secs(60), |e| {
            e.pending().is_empty() && e.all_state(ContainerState::Running)
        })
        .await;
    let final_pending = env.pending();

    // Leave the fixtures running whatever happened above.
    let mut restored = Vec::new();
    for name in FIXTURE {
        let out = run_docker(&docker, &host, &["start", name]);
        restored.push(out.status.success());
    }
    cancel.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), engine_task).await;
    let entries = store
        .activity_query(&berth_store::ActivityFilter::default(), None, 100)
        .expect("activity")
        .into_iter()
        .filter(|e| e.origin == berth_core::ActivityOrigin::Berth)
        .map(|e| (e.action, e.outcome))
        .collect::<Vec<_>>();

    println!(
        "actions_it: ready={ready} accepted={accepted:?} pending_right_after={pending_right_after:?} \
         exited={exited} in {exited_in:?} pending_after_stop={pending_after_stop:?} web_back={web_back} \
         restart_results={results:?} pending_three={pending_three} restarted={restarted} \
         stop_project={stop_project:?} stop_pending={stop_pending:?} all_exited={all_exited} \
         stopped_status={stopped_status:?} start_project={start_project:?} all_running={all_running} \
         final_pending={final_pending:?}\nactivity={entries:?}"
    );

    assert!(ready, "all four fixture containers running first");
    assert_eq!(accepted, Ok(()));
    assert_eq!(
        pending_right_after,
        vec![(
            web_key.clone(),
            PendingState::Running {
                label: "Stopping…".to_string()
            }
        )],
        "Stopping… pending before the engine answered"
    );
    assert!(exited, "signal showed Exited");
    assert!(
        exited_in <= std::time::Duration::from_secs(2),
        "Exited took {exited_in:?}, over the 2 s budget"
    );
    assert!(pending_after_stop.is_empty(), "{pending_after_stop:?}");
    assert!(web_back, "web running again");
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(Result::is_ok), "{results:?}");
    assert_eq!(pending_three, 3, "three pending states set in one call");
    assert!(
        restarted,
        "three restarted and cleared: {pending_after_restart:?}"
    );
    assert_eq!(stop_project, Ok(()));
    assert_eq!(
        stop_pending,
        vec![(
            PendingKey {
                subject: Subject::Project(project.id.clone()),
                action: "project.stop"
            },
            PendingState::Running {
                label: "Stopping…".to_string()
            }
        )]
    );
    assert!(all_exited, "Stop project stopped all four");
    assert_eq!(stopped_status, Some(berth_core::ProjectStatus::Stopped));
    assert_eq!(start_project, Ok(()));
    assert!(all_running, "Start project brought all four back");
    assert!(final_pending.is_empty(), "{final_pending:?}");
    assert!(restored.iter().all(|ok| *ok), "fixtures left running");
    let ok_actions: Vec<&str> = entries
        .iter()
        .filter(|(_, o)| *o == berth_core::Outcome::Ok)
        .map(|(a, _)| a.as_str())
        .collect();
    assert!(ok_actions.contains(&"container.stop"));
    assert_eq!(
        ok_actions
            .iter()
            .filter(|a| **a == "container.restart")
            .count(),
        3
    );
    assert!(ok_actions.contains(&"project.stop"));
    assert!(ok_actions.contains(&"project.start"));
}
