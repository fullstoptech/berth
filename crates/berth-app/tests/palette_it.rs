#![allow(clippy::expect_used)]
//! The palette's by-hand check as a test (P2-T9 done-when): "type `restart
//! berthfx-web`, press Enter, it restarts". The test runs the same search
//! over the live engine's state, takes the top result and runs it the way
//! Enter does (`search::run` through the actions service), then checks the
//! container really restarted. Nobody needs Accessibility permission.
//!
//! Run with `scripts/it.sh -p berth-app palette_it` (sets BERTH_IT=1 and
//! brings the fixtures up first). Only `berthfx-web-1` is restarted, and
//! only after its fixture label is confirmed; Colima is never touched.

mod common;

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::actions::{apply_finished, with_view, ActionsService};
use berth_app::services::engines::client_map;
use berth_app::services::search::{run, Index, RunOutcome, Target};
use berth_app::state::{use_app_state, AppState, EngineScope, Selection};
use berth_core::{ContainerState, Subject};
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

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1 via scripts/it.sh"]
async fn palette_it_restart_berthfx_web_runs_the_top_result() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    common::settle_fixture().await;
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let mut state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");

    let search_path = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima_cli = berth_engine::ColimaCli::new(search_path.clone());
    let engines = berth_engine::discovery::discover(&colima_cli, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default" && e.state == berth_core::EngineState::Running)
        .expect("colima:default running")
        .clone();
    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let containers = client.containers().await.expect("containers");
    let web = containers
        .iter()
        .find(|c| c.name == "berthfx-web-1")
        .expect("berthfx-web-1 exists (run scripts/fx-up.sh)")
        .clone();
    assert_eq!(
        web.labels.get("dev.berth.fixture").map(String::as_str),
        Some("1"),
        "refusing to restart a container without the fixture label"
    );
    assert_eq!(web.state, ContainerState::Running, "fixture web is running");

    let images = client.images().await.expect("images");
    dom.in_runtime(|| {
        state.engines.set(engines.clone());
        state.containers.set(containers.clone());
        state.projects.set(Vec::new());
        state.images.set(images.clone());
        state.engine_scope.set(EngineScope::One(engine.id.clone()));
    });

    // 1. The search: what the palette lists for the typed text.
    let results = dom.in_runtime(|| {
        let index = with_view(&state, |view| {
            Index::build(view, &Selection::None, &EngineScope::One(engine.id.clone()))
        });
        index.search("restart berthfx-web", &[])
    });
    let top = results
        .flat()
        .first()
        .map(|item| (*item).clone())
        .expect("a result for the query");
    println!(
        "palette_it: top result = {:?} ({}); groups = {:?}",
        top.title,
        top.meta,
        results
            .groups
            .iter()
            .map(|g| (g.group.title(), g.items.len()))
            .collect::<Vec<_>>()
    );
    assert_eq!(top.title, "Restart berthfx-web-1");
    assert_eq!(
        top.target,
        Target::Run {
            action: "container.restart",
            subject: Some(Subject::Container(web.id.clone())),
        }
    );

    // 2. Enter: run it through the actions service.
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let clients = client_map();
    clients
        .lock()
        .expect("clients")
        .insert(engine.id.clone(), Arc::clone(&client));
    let (service, mut finished) = ActionsService::new(
        clients,
        Arc::clone(&store),
        Arc::new(berth_engine::MockCompose::bundled()),
        Arc::new(berth_engine::MockColima::bundled()),
        CancellationToken::new(),
    );
    let started_before = client
        .container(&web.id)
        .await
        .expect("detail")
        .container
        .started;
    assert!(
        started_before.is_some(),
        "a running container has a start time"
    );
    let outcome = dom.in_runtime(|| run(&mut state, Some(&service), &top));
    assert_eq!(outcome, RunOutcome::Done);
    let pending_right_after = dom.in_runtime(|| {
        state
            .pending
            .peek()
            .iter()
            .map(|(k, v)| (k.action, v.clone()))
            .collect::<Vec<_>>()
    });
    println!("palette_it: pending right after Enter = {pending_right_after:?}");
    assert_eq!(pending_right_after.len(), 1);
    assert_eq!(pending_right_after[0].0, "container.restart");

    let done = tokio::time::timeout(std::time::Duration::from_secs(40), finished.recv())
        .await
        .expect("the restart finishes within 40 s")
        .expect("finished channel open");
    let result = done.result.clone();
    dom.in_runtime(|| apply_finished(&mut state, done));
    assert_eq!(result, Ok(()));

    // 3. The container really restarted: running again, new start time.
    // (The list endpoint carries no start time, the detail does.)
    let after = client.container(&web.id).await.expect("detail").container;
    println!(
        "palette_it: started before={:?} after={:?} state={:?}",
        started_before, after.started, after.state
    );
    assert_eq!(after.state, ContainerState::Running);
    assert!(after.started.is_some());
    assert_ne!(
        after.started, started_before,
        "start time changed: it restarted"
    );
    let activity = store
        .activity_query(&berth_store::ActivityFilter::default(), None, 10)
        .expect("activity");
    assert!(
        activity
            .iter()
            .any(|e| e.action == "container.restart" && e.outcome == berth_core::Outcome::Ok),
        "a restart entry was recorded: {activity:?}"
    );
}
