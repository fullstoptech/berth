#![allow(clippy::expect_used)]
//! Live engines service against the berthfx fixture (P2-T1 done-when).
//! Run with `scripts/it.sh -p berth-app engines_it` (sets BERTH_IT=1 and
//! brings the fixtures up first). Stops `berthfx-web-1` through the
//! docker CLI, expects the signal to show Exited within 2 s, and starts
//! it again on every exit path (asserts run only after the restart).

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::engines::{run_engine, Applier, EngineUpdate};
use berth_app::state::{use_app_state, AppState};
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

fn web_state(dom: &VirtualDom, state: &AppState) -> Option<berth_core::ContainerState> {
    dom.in_runtime(|| {
        state
            .containers
            .peek()
            .iter()
            .find(|c| c.name == "berthfx-web-1")
            .map(|c| c.state)
    })
}

fn docker_bin(search: &berth_sys::SearchPath) -> std::path::PathBuf {
    search
        .find("docker")
        .unwrap_or_else(|| std::path::PathBuf::from("docker"))
}

fn run_docker(docker: &std::path::Path, host: &str, args: &[&str]) -> std::process::Output {
    std::process::Command::new(docker)
        .env("DOCKER_HOST", host)
        .args(args)
        .output()
        .expect("docker CLI runs")
}

async fn apply_until(
    dom: &VirtualDom,
    applier: &mut Applier,
    state: &mut AppState,
    rx: &mut tokio::sync::mpsc::Receiver<EngineUpdate>,
    want: berth_core::ContainerState,
    budget: std::time::Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if web_state(dom, state) == Some(want) {
            return true;
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return false;
        }
        match tokio::time::timeout(deadline - now, rx.recv()).await {
            Ok(Some(update)) => dom.in_runtime(|| applier.apply(state, update)),
            _ => return web_state(dom, state) == Some(want),
        }
    }
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1 via scripts/it.sh"]
async fn engines_it_web_turns_exited_in_the_signal_within_2s_of_docker_stop() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let (dom, mut state) = harness();
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima = berth_engine::ColimaCli::new(search.clone());
    let engines = berth_engine::discovery::discover(&colima, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default" && e.state == berth_core::EngineState::Running)
        .expect("colima:default running")
        .clone();
    let host = engine.docker_host.clone().expect("docker host");
    let docker = docker_bin(&search);
    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));

    let mut applier = Applier::new(Vec::new());
    dom.in_runtime(|| {
        applier.apply(&mut state, EngineUpdate::EnginesDiscovered(engines.clone()));
    });
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let cancel = CancellationToken::new();
    let task = tokio::spawn(run_engine(
        Arc::clone(&client),
        Arc::clone(&store),
        Vec::new(),
        false,
        tx,
        cancel.clone(),
        std::time::Duration::from_millis(200),
    ));

    // Initial lists, then make sure web is running before we stop it.
    let initial = apply_until(
        &dom,
        &mut applier,
        &mut state,
        &mut rx,
        berth_core::ContainerState::Running,
        std::time::Duration::from_secs(10),
    )
    .await;
    let mut notes: Vec<String> = Vec::new();
    if !initial {
        // Leftover stopped fixture: start it through the CLI and wait.
        let out = run_docker(&docker, &host, &["start", "berthfx-web-1"]);
        notes.push(format!(
            "pre-start: status={} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let running_first = apply_until(
        &dom,
        &mut applier,
        &mut state,
        &mut rx,
        berth_core::ContainerState::Running,
        std::time::Duration::from_secs(10),
    )
    .await;

    // The done-when measurement: docker stop -> signal shows Exited.
    let stop_started = std::time::Instant::now();
    let stop_out = run_docker(&docker, &host, &["stop", "berthfx-web-1"]);
    let exited = apply_until(
        &dom,
        &mut applier,
        &mut state,
        &mut rx,
        berth_core::ContainerState::Exited,
        std::time::Duration::from_secs(2),
    )
    .await;
    let elapsed = stop_started.elapsed();

    // Restart on every path from here, whatever was observed above.
    let start_out = run_docker(&docker, &host, &["start", "berthfx-web-1"]);
    let running_again = apply_until(
        &dom,
        &mut applier,
        &mut state,
        &mut rx,
        berth_core::ContainerState::Running,
        std::time::Duration::from_secs(5),
    )
    .await;
    cancel.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task).await;

    println!(
        "engines_it: running_first={running_first} exited={exited} in {elapsed:?} \
         stop_status={} start_status={} running_again={running_again} notes={notes:?}",
        stop_out.status, start_out.status
    );
    assert!(running_first, "berthfx-web-1 running in the signal first");
    assert!(stop_out.status.success(), "docker stop succeeded");
    assert!(exited, "signal showed Exited");
    assert!(
        elapsed <= std::time::Duration::from_secs(2),
        "Exited took {elapsed:?}, over the 2 s budget"
    );
    assert!(start_out.status.success(), "docker start succeeded");
    assert!(running_again, "berthfx-web-1 running again afterwards");
}
