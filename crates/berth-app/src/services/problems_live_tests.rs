//! P5-T8 live check, against the real engine and the `berthfx-broken` fixtures. Run with:
//!   scripts/it.sh -p berth-app problems_it
//! The test brings `berthfx-broken` up and takes it down again itself.
//!
//! The app without its window: the engines service fills the signals, the actions service is
//! asked to start the broken project (Compose fails at the image that does not exist, and its
//! lines are what the rules read), and the problems service lists what it finds. Then a file fix
//! (the port clash) opens Review changes with a correct plan that is never written, and an
//! action fix (the unhealthy service's restart) runs through its confirm dialog and clears.
//!
//! Only containers of `berthfx-broken` are touched, by the project's own Start and the restart of
//! its unhealthy service. Inspect output holds env values: nothing of it is printed.

// Helpers sit outside the #[test] fn, so clippy.toml's test allowance does not reach them.
#![allow(clippy::expect_used)]

use std::time::Duration;

use berth_core::{ProblemKind, Project};

use super::*;

#[path = "../../../../fixtures/rust/broken_set.rs"]
mod broken_set;
use crate::screens::problems::model::{fix_click, Clicked};
use crate::services::actions::{ActionsService, ProjectOp};
use crate::services::confirm;
use crate::services::plans::PlanStore;
use crate::state::{use_app_state, Overlay};

fn engine() -> EngineId {
    EngineId("colima:default".to_string())
}

fn broken_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/compose/berthfx-broken")
        .canonicalize()
        .expect("the fixture folder")
}

struct App {
    state: AppState,
    actions: ActionsService,
    problems: ProblemsService,
    plans: PlanStore,
    services: Services,
    cancel: CancellationToken,
}

thread_local! {
    static SLOT: std::cell::RefCell<Option<App>> = const { std::cell::RefCell::new(None) };
}

/// Everything the App root starts, minus the window.
#[component]
fn Headless() -> Element {
    let state = use_app_state();
    use_hook(move || {
        let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
        for folder in [
            broken_dir(),
            broken_dir()
                .join("../berthfx")
                .canonicalize()
                .expect("berthfx"),
        ] {
            store
                .pin(&berth_core::PinnedFolder {
                    folder,
                    engine: engine(),
                    added: Utc::now(),
                })
                .expect("pin");
        }
        let clients = crate::services::engines::client_map();
        let cancel = CancellationToken::new();
        let (compose, colima) = crate::services::action_runners(false);
        let actions = ActionsService::start(
            state,
            clients.clone(),
            Arc::clone(&store),
            compose,
            colima,
            cancel.clone(),
        );
        let services = Services {
            store: Arc::clone(&store),
            clients: clients.clone(),
            cancel: cancel.clone(),
        };
        let handle = crate::services::engines::start(state, store, clients, cancel.clone(), false);
        let problems = ProblemsService::start(state, services.clone(), actions.clone());
        // What the App root does: a pass whenever what the rules read changes.
        let watcher = problems.clone();
        std::mem::forget(handle);
        SLOT.with(|slot| {
            *slot.borrow_mut() = Some(App {
                state,
                actions,
                problems: watcher,
                plans: PlanStore::default(),
                services,
                cancel,
            });
        });
    });
    let watcher = SLOT.with(|slot| slot.borrow().as_ref().map(|app| app.problems.clone()));
    use_effect(move || {
        let _ = state.engines.read();
        let _ = state.containers.read();
        let _ = state.projects.read();
        let _ = state.pending.read();
        if let Some(problems) = &watcher {
            problems.nudge();
        }
    });
    rsx! { div {} }
}

async fn until(
    dom: &mut VirtualDom,
    app: &App,
    what: &str,
    secs: u64,
    mut done: impl FnMut(&AppState) -> bool,
) {
    let waited = tokio::time::timeout(Duration::from_secs(secs), async {
        let mut ticks = 0_u32;
        loop {
            if dom.in_runtime(|| done(&app.state)) {
                return;
            }
            let _ = tokio::time::timeout(Duration::from_millis(50), dom.wait_for_work()).await;
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
            ticks += 1;
            if ticks.is_multiple_of(10) {
                app.problems.nudge();
            }
        }
    })
    .await;
    if waited.is_err() {
        let listed: Vec<String> = dom.in_runtime(|| {
            app.state
                .problems
                .peek()
                .iter()
                .map(|p| format!("{:?} {}", p.kind, p.title))
                .collect()
        });
        panic!("{what} did not happen; listed: {listed:#?}");
    }
}

fn service_of(state: &AppState, p: &Problem) -> Option<String> {
    match &p.subject {
        Subject::Container(id) => state
            .containers
            .peek()
            .iter()
            .find(|c| c.id == *id)
            .and_then(|c| c.service.clone()),
        _ => None,
    }
}

/// The problems of the broken set that the list does not have yet. `badmount` has no container
/// (Compose cannot create it); the service finds it by a stat of the Compose file's strict binds
/// (AD35), so a pass lists it without any Compose command having failed.
fn missing(state: &AppState) -> Vec<&'static str> {
    let problems = state.problems.peek();
    let on = |wanted: &dyn Fn(&ProblemKind) -> bool, service: &str| {
        problems
            .iter()
            .any(|p| wanted(&p.kind) && service_of(state, p).as_deref() == Some(service))
    };
    let mut out = Vec::new();
    if !on(
        &|k| matches!(k, ProblemKind::PortAllocated { port: 56080 }),
        "portclash",
    ) {
        out.push("PortAllocated for portclash");
    }
    if !problems.iter().any(|p| {
        matches!(&p.kind, ProblemKind::ImageNotFound { reference } if reference == "berthfx/does-not-exist:1")
    }) {
        out.push("ImageNotFound for badimage");
    }
    if !on(
        &|k| matches!(k, ProblemKind::ExitedNoLogs { code: 1 }),
        "crash",
    ) {
        out.push("ExitedNoLogs for crash");
    }
    if !on(&|k| matches!(k, ProblemKind::OutOfMemory), "oom") {
        out.push("OutOfMemory for oom");
    }
    if !problems
        .iter()
        .any(|p| matches!(&p.kind, ProblemKind::EnvVarMissing { var } if var == "BERTHFX_MISSING"))
    {
        out.push("EnvVarMissing for needsvar");
    }
    if !on(&|k| matches!(k, ProblemKind::Unhealthy), "unhealthy") {
        out.push("Unhealthy for unhealthy");
    }
    if !problems.iter().any(|p| {
        matches!(&p.kind, ProblemKind::BindPathMissing { path } if path == "/nonexistent-berthfx-path")
    }) {
        out.push("BindPathMissing for badmount");
    }
    out
}

fn broken_project(state: &AppState) -> Option<Project> {
    state
        .projects
        .peek()
        .iter()
        .find(|p| p.name == "berthfx-broken" && p.folder.is_some())
        .cloned()
}

#[tokio::test]
#[ignore = "live: needs Colima; brings berthfx-broken up and down itself (BERTH_IT=1)"]
async fn problems_it_the_broken_fixtures_list_seven_problems_and_fixes_work() {
    if std::env::var("BERTH_IT").as_deref() != Ok("1") {
        eprintln!("skipped: set BERTH_IT=1");
        return;
    }
    // Brought up here and taken down when the guard drops, even if an assertion fails.
    let _broken = broken_set::broken_up();
    let mut dom = VirtualDom::new(Headless);
    let _ = dom.rebuild_to_vec();
    let app = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("the headless app started");

    // 1. The engines service finds the broken project.
    until(
        &mut dom,
        &app,
        "the broken project is listed",
        90,
        |state| broken_project(state).is_some(),
    )
    .await;
    let project = dom
        .in_runtime(|| broken_project(&app.state))
        .expect("the broken project");

    // 2a. A cold start: before any Compose command, one pass already lists the missing bind
    //     folder (a stat of the Compose file, AD35).
    until(
        &mut dom,
        &app,
        "BindPathMissing is listed before any Compose command ran",
        60,
        |state| {
            state.problems.peek().iter().any(|p| {
                matches!(&p.kind, ProblemKind::BindPathMissing { path } if path == "/nonexistent-berthfx-path")
            })
        },
    )
    .await;
    assert!(app.actions.compose_failure_lines(&project.id).is_empty());

    // 2. Start project: Compose stops at the image that does not exist; its lines feed the rules.
    dom.in_scope(ScopeId::APP, || {
        let mut state = app.state;
        app.actions
            .project(&mut state, &project, ProjectOp::Start)
            .expect("Start project is allowed on a failing project");
    });
    until(
        &mut dom,
        &app,
        "Compose's failure lines are kept",
        120,
        |_| !app.actions.compose_failure_lines(&project.id).is_empty(),
    )
    .await;
    println!(
        "compose failure lines: {}",
        app.actions.compose_failure_lines(&project.id).len()
    );

    // 3. The panel's list: all seven problems of the set.
    until(
        &mut dom,
        &app,
        "the seven problems are listed",
        120,
        |state| missing(state).is_empty(),
    )
    .await;
    let listed: Vec<Problem> = dom.in_runtime(|| app.state.problems.peek().clone());
    for p in &listed {
        println!("{:?} | {:?} | {}", p.severity, p.kind, p.title);
        assert!(!p.fixes.is_empty(), "{} has no fix", p.title);
    }
    // Nothing is said about the healthy berthfx project.
    dom.in_runtime(|| {
        for p in app.state.problems.peek().iter() {
            if let Subject::Container(id) = &p.subject {
                let c = app.state.containers.peek();
                let c = c.iter().find(|c| c.id == *id).expect("its container");
                assert_ne!(c.compose_project.as_deref(), Some("berthfx"), "{}", p.title);
            }
        }
    });
    // The order is stable across a pass (AD31): the same problems, in the same places.
    app.problems.nudge();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    for _ in 0..10 {
        let _ = tokio::time::timeout(Duration::from_millis(50), dom.wait_for_work()).await;
        dom.render_immediate(&mut dioxus::core::NoOpMutations);
    }
    let again: Vec<ProblemId> = dom.in_runtime(|| {
        app.state
            .problems
            .peek()
            .iter()
            .map(|p| p.id.clone())
            .collect()
    });
    let before: Vec<ProblemId> = listed.iter().map(|p| p.id.clone()).collect();
    assert!(
        before.iter().all(|id| again.contains(id)),
        "no problem went away"
    );
    let kept: Vec<&ProblemId> = again.iter().filter(|id| before.contains(id)).collect();
    let expected: Vec<&ProblemId> = before.iter().collect();
    assert_eq!(kept, expected, "the listed problems kept their order");

    // 4. The port fix: a plan for compose.yaml that maps portclash to 56081 and writes nothing.
    let port = listed
        .iter()
        .find(|p| matches!(p.kind, ProblemKind::PortAllocated { port: 56080 }))
        .cloned()
        .expect("the port problem");
    let label = port.fixes[0].label.clone();
    println!("port fix: {label}");
    assert_eq!(label, "Map portclash to port 56081");
    let clicked = dom.in_runtime(|| {
        let mut state = app.state;
        fix_click(&mut state, Some(&app.plans), &port, 0)
    });
    let Clicked::Review(id) = clicked else {
        panic!("the port fix opens Review changes, not {clicked:?}");
    };
    let overlays = dom.in_runtime(|| app.state.overlays.peek().clone());
    assert_eq!(overlays.last(), Some(&Overlay::ReviewChanges(id)));
    let pending = app.plans.get(&id).expect("the plan is parked");
    assert_eq!(pending.plan.files.len(), 1);
    let file = &pending.plan.files[0];
    assert_eq!(file.path, std::path::PathBuf::from("compose.yaml"));
    let new_text = file.new_text.clone().expect("the new text");
    assert!(new_text.contains("56081"), "{new_text}");
    assert!(!new_text.contains("56080"), "{new_text}");
    assert!(file.old_sha256.is_some());
    assert_eq!(pending.plan.root, project.folder.clone().expect("folder"));
    assert!(
        pending.run.is_some(),
        "Write and restart knows where to run"
    );
    // The new text is valid Compose, and the folder's file was not touched.
    let compose = crate::services::compose::runner(false).await;
    compose
        .validate_text(&broken_dir(), &new_text, &[])
        .await
        .expect("the new compose text loads");
    let on_disk = std::fs::read_to_string(broken_dir().join("compose.yaml")).expect("compose.yaml");
    assert!(on_disk.contains("56080") && !on_disk.contains("56081"));

    // 5. An action fix: Unhealthy -> Restart runs through the dialog, and the problem clears.
    let unhealthy = listed
        .iter()
        .find(|p| p.kind == ProblemKind::Unhealthy)
        .cloned()
        .expect("the unhealthy problem");
    println!("unhealthy fix: {}", unhealthy.fixes[0].label);
    let clicked = dom.in_runtime(|| {
        let mut state = app.state;
        fix_click(&mut state, Some(&app.plans), &unhealthy, 0)
    });
    assert_eq!(clicked, Clicked::Confirm);
    let model = dom.in_runtime(|| match app.state.overlays.peek().last() {
        Some(Overlay::Confirm(model)) => model.clone(),
        other => panic!("a confirm dialog expected, got {other:?}"),
    });
    println!(
        "dialog: {} | {} | {:?}",
        model.title,
        confirm::button_label(&model.effect),
        model
            .effect
            .lines
            .iter()
            .map(|l| &l.text)
            .collect::<Vec<_>>()
    );
    let launched = dom.in_runtime(|| {
        let mut state = app.state;
        confirm::run(&mut state, &app.actions, &model, false)
    });
    assert!(launched.refused.is_empty(), "{:?}", launched.refused);
    assert_eq!(launched.keys.len(), 1);
    let key = launched.keys[0].clone();
    until(&mut dom, &app, "the restart finished", 60, |state| {
        !state.pending.peek().contains_key(&key)
    })
    .await;
    let failed = dom.in_runtime(|| app.state.pending.peek().get(&key).cloned());
    assert!(failed.is_none(), "the restart did not fail: {failed:?}");
    // Restarted: its health is `starting`, so the problem is gone (it comes back as a new one a
    // few seconds later, because the fixture's check always fails).
    let seen = unhealthy.first_seen;
    until(
        &mut dom,
        &app,
        "the unhealthy problem cleared",
        60,
        |state| {
            state
                .problems
                .peek()
                .iter()
                .find(|p| p.id == unhealthy.id)
                .is_none_or(|p| p.first_seen > seen)
        },
    )
    .await;
    println!("the unhealthy problem cleared after its restart");

    app.cancel.cancel();
    let _ = &app.services;
}
