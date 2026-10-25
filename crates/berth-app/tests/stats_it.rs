#![allow(clippy::expect_used)]
//! Live stats against the berthfx fixture (P3-T4 done-when). Run directly:
//! `BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only stats_it
//! --no-capture` (the fixtures must be up).
//!
//! 1. Subscribing to `berthfx-db-1` yields at least three samples whose
//!    timestamps change.
//! 2. With every berthfx row "visible" (and then every running container on
//!    the engine, read-only), the number of open `docker.sock` connections
//!    of this process stays under 70 and the service never holds more than
//!    60 streams.
//!
//! Nothing is stopped, removed or restarted; the engine is only read.

mod common;

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use berth_app::services::engines::client_map;
use berth_app::services::stats::{StatsKey, StatsService, MAX_SUBSCRIPTIONS};
use berth_core::Container;
use berth_engine::EngineClient;

async fn live_client() -> (berth_core::Engine, Arc<dyn EngineClient>) {
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima_cli = berth_engine::ColimaCli::new(search);
    let engines = berth_engine::discovery::discover(&colima_cli, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default" && e.state == berth_core::EngineState::Running)
        .expect("colima:default running")
        .clone();
    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    (engine, client)
}

fn service_over(engine: &berth_core::Engine, client: &Arc<dyn EngineClient>) -> StatsService {
    let clients = client_map();
    clients
        .lock()
        .expect("client map")
        .insert(engine.id.clone(), Arc::clone(client));
    StatsService::new(clients, CancellationToken::new())
}

/// Unix-domain socket descriptors this process holds, by `lsof -U`. On
/// macOS the client end of a connection to `docker.sock` is listed as
/// `unix 0x… ->0x…` without the path, so `grep docker.sock` finds none; this
/// process talks to no other unix socket, so the count is its engine
/// connections.
fn docker_sock_connections() -> usize {
    let out = std::process::Command::new("lsof")
        .args(["-a", "-p", &std::process::id().to_string(), "-U"])
        .output()
        .expect("lsof runs");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .filter(|l| l.contains(" unix "))
        .count()
}

async fn until(timeout: Duration, mut check: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    check()
}

fn by_name<'a>(containers: &'a [Container], name: &str) -> &'a Container {
    containers
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} listed"))
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn stats_it_subscribing_to_db_yields_samples_with_changing_timestamps() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    common::settle_fixture().await;
    let (engine, client) = live_client().await;
    let containers = client.containers().await.expect("list");
    let db = by_name(&containers, "berthfx-db-1");
    let service = service_over(&engine, &client);
    let key = StatsKey::new(&engine.id, &db.id);
    let owner = service.new_owner();
    service.watch_row(owner, Some(key.clone()));
    service.reconcile();

    let got = until(Duration::from_secs(20), || service.history(&key).len() >= 3).await;
    let history = service.history(&key);
    let times: Vec<_> = history.points().map(|p| p.stats.at).collect();
    eprintln!("samples={} timestamps={times:?}", history.len());
    assert!(got, "fewer than 3 samples in 20 s: {}", history.len());
    assert!(
        times.windows(2).all(|w| w[1] > w[0]),
        "timestamps must change: {times:?}"
    );
    let latest = service.latest(&key, chrono::Utc::now()).expect("fresh");
    assert!(latest.mem_used.as_u64() > 0, "postgres uses memory");
    assert!(latest.cpu_pct.is_finite() && latest.cpu_pct >= 0.0);

    service.release(owner);
    service.reconcile();
    assert_eq!(service.subscription_count(), 0);
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn stats_it_every_visible_row_keeps_docker_sock_connections_bounded() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    common::settle_fixture().await;
    let (engine, client) = live_client().await;
    let containers = client.containers().await.expect("list");
    let service = service_over(&engine, &client);
    let idle = docker_sock_connections();

    // Phase A: the four berthfx rows.
    let mut owners = Vec::new();
    for container in containers.iter().filter(|c| c.name.starts_with("berthfx-")) {
        let owner = service.new_owner();
        service.watch_row(owner, Some(StatsKey::new(&engine.id, &container.id)));
        owners.push(owner);
    }
    assert_eq!(owners.len(), 4, "four berthfx rows");
    service.reconcile();
    let mut peak_a = 0;
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        service.reconcile();
        peak_a = peak_a.max(docker_sock_connections());
    }
    let open_a = service.subscription_count();

    // Phase B: every running container on the engine as a visible row (a
    // read-only stats stream each), capped by the service at 60.
    for container in containers
        .iter()
        .filter(|c| c.state == berth_core::ContainerState::Running)
        .filter(|c| !c.name.starts_with("berthfx-"))
    {
        let owner = service.new_owner();
        service.watch_row(owner, Some(StatsKey::new(&engine.id, &container.id)));
        owners.push(owner);
    }
    let mut peak_b = 0;
    let mut peak_streams = 0;
    for _ in 0..12 {
        service.reconcile();
        tokio::time::sleep(Duration::from_millis(500)).await;
        peak_b = peak_b.max(docker_sock_connections());
        peak_streams = peak_streams.max(service.subscription_count());
    }
    eprintln!(
        "docker.sock connections: idle={idle} berthfx_rows({open_a} streams)={peak_a} \
         all_{}_rows({peak_streams} streams)={peak_b}",
        owners.len()
    );
    assert!(
        peak_a >= 1,
        "lsof saw no engine connection: the count is broken"
    );
    assert!(peak_a < 70 && peak_b < 70, "a={peak_a} b={peak_b}");
    assert!(peak_streams <= MAX_SUBSCRIPTIONS);
    assert!(open_a == 4, "{open_a}");

    // Scroll everything away: the connections go with the streams.
    for owner in owners {
        service.release(owner);
    }
    service.reconcile();
    assert_eq!(service.subscription_count(), 0);
    let closed = until(Duration::from_secs(5), || {
        docker_sock_connections() <= idle + 1
    })
    .await;
    eprintln!("after release: {}", docker_sock_connections());
    assert!(closed, "connections did not close after release");
}
