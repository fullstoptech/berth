#![allow(clippy::expect_used)]
//! Live Activity feed against the berthfx fixture (P2-T10 done-when).
//! Run with `scripts/it.sh -p berth-app activity_it` (sets BERTH_IT=1 and
//! brings the fixtures up first).
//!
//! Restarting `berthfx-web-1` through the actions service must put a Berth
//! entry (`container.restart`) and the engine's own entries (`kill`, `die`,
//! `start`) on the activity tap within 2 s, and into the store. The tap is
//! exactly what the Activity screen follows, so this is the by-hand check
//! "restart from the row, see the rows appear" without a window.
//!
//! Colima itself is never touched (the Colima runner is the mock) and the
//! fixture container is started again on every exit path.

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::actions::{ActionsService, ContainerTarget};
use berth_app::services::activity;
use berth_app::services::engines::{client_map, run_engine};
use berth_app::state::{use_app_state, AppState};
use berth_core::{ActivityOrigin, ContainerAction};
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

fn run_docker(docker: &std::path::Path, host: &str, args: &[&str]) -> std::process::Output {
    std::process::Command::new(docker)
        .env("DOCKER_HOST", host)
        .args(args)
        .output()
        .expect("docker CLI runs")
}

const BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1 via scripts/it.sh"]
async fn activity_it_restart_adds_a_berth_entry_and_engine_entries_within_2s() {
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
    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));

    let clients = client_map();
    clients
        .lock()
        .expect("clients")
        .insert(engine.id.clone(), Arc::clone(&client));
    let cancel = CancellationToken::new();
    let (service, _finished) = ActionsService::new(
        clients,
        Arc::clone(&store),
        Arc::new(berth_engine::CliCompose::new(search.clone())),
        Arc::new(berth_engine::MockColima::bundled()),
        cancel.clone(),
    );

    // The engine events loop writes entries as the app does; drain its
    // updates so it never blocks on a full channel.
    let (tx, mut updates) = tokio::sync::mpsc::channel(256);
    let engine_task = tokio::spawn(run_engine(
        Arc::clone(&client),
        Arc::clone(&store),
        Vec::new(),
        false,
        tx,
        cancel.clone(),
        std::time::Duration::from_millis(200),
    ));
    let drain = tokio::spawn(async move { while updates.recv().await.is_some() {} });

    // Fixture up and the engine stream connected before we act.
    let _ = run_docker(&docker, &host, &["start", "berthfx-web-1"]);
    tokio::time::sleep(std::time::Duration::from_millis(1_000)).await;
    // The service checks the row against the signals, as the app does.
    let containers = client.containers().await.expect("containers");
    let web = containers
        .iter()
        .find(|c| c.name == "berthfx-web-1")
        .map(ContainerTarget::from)
        .expect("berthfx-web-1 exists");
    dom.in_runtime(|| {
        state.engines.set(engines.clone());
        state.containers.set(containers.clone());
    });

    let mut feed = activity::subscribe();
    let t0 = std::time::Instant::now();
    let accepted = dom.in_runtime(|| {
        service.container(
            &mut state,
            &web,
            ContainerAction::Restart { timeout_s: Some(5) },
        )
    });

    let mut seen: Vec<(ActivityOrigin, String)> = Vec::new();
    let want = |seen: &[(ActivityOrigin, String)]| {
        [
            (ActivityOrigin::Berth, "container.restart"),
            (ActivityOrigin::Engine, "container.kill"),
            (ActivityOrigin::Engine, "container.die"),
            (ActivityOrigin::Engine, "container.start"),
        ]
        .iter()
        .all(|(origin, action)| seen.iter().any(|(o, a)| o == origin && a == action))
    };
    while !want(&seen) {
        let left = BUDGET.saturating_sub(t0.elapsed());
        if left.is_zero() {
            break;
        }
        match tokio::time::timeout(left, feed.recv()).await {
            Ok(Ok(entry)) if entry.subject_name == "berthfx-web-1" => {
                seen.push((entry.origin, entry.action));
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let elapsed = t0.elapsed();
    let stored: Vec<(ActivityOrigin, String)> = activity::load_recent(&store, 100)
        .into_iter()
        .filter(|e| e.subject_name == "berthfx-web-1")
        .map(|e| (e.origin, e.action))
        .collect();

    // Leave the fixture running whatever happened above.
    let restored = run_docker(&docker, &host, &["start", "berthfx-web-1"])
        .status
        .success();
    cancel.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), engine_task).await;
    drain.abort();

    println!(
        "activity_it: accepted={accepted:?} elapsed={elapsed:?} on_tap={seen:?} in_store={stored:?}"
    );
    assert_eq!(accepted, Ok(()));
    assert!(
        want(&seen),
        "restart should show restart + kill + die + start within {BUDGET:?}, saw {seen:?}"
    );
    assert!(elapsed <= BUDGET, "took {elapsed:?}");
    assert!(
        want(&stored),
        "the same entries are in the store: {stored:?}"
    );
    assert!(
        !stored.iter().any(|(_, a)| a.contains("exec")),
        "exec events (health checks) never reach the store: {stored:?}"
    );
    assert!(restored, "fixture left running");
}
