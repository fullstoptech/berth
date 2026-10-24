//! Live streams and container actions against the berthfx fixtures
//! (P1-T5 done-when). Run with:
//!   scripts/it.sh -p berth-engine streams
//! One sequential test: it stops `berthfx-web-1` and always starts it again.

// Test helpers sit outside the #[test] fn, so clippy.toml's test allowance
// does not reach them.
#![allow(clippy::expect_used)]

use std::time::Duration;

use berth_engine::{BollardEngine, ColimaCli, EngineClient};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

async fn connect() -> BollardEngine {
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima = ColimaCli::new(search);
    let engines = berth_engine::discovery::discover(&colima, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default")
        .expect("colima:default discovered");
    BollardEngine::connect(engine).expect("connect")
}

async fn fixture_id(client: &BollardEngine, name: &str) -> berth_core::ContainerId {
    let containers = client.containers().await.expect("containers");
    containers
        .iter()
        .find(|c| c.name == name)
        .map(|c| c.id.clone())
        .expect("fixture container present")
}

async fn run_checks(client: &BollardEngine) -> Result<(), String> {
    let web = fixture_id(client, "berthfx-web-1").await;
    let logger = fixture_id(client, "berthfx-logger-1").await;
    let db = fixture_id(client, "berthfx-db-1").await;

    // Events: subscribe first, then stop web through act; a die/stop event
    // for web must arrive within 2 s.
    let events_cancel = CancellationToken::new();
    let mut events = client.events(events_cancel.clone());
    client
        .act(
            &web,
            berth_core::ContainerAction::Stop { timeout_s: Some(5) },
        )
        .await
        .map_err(|e| format!("stop berthfx-web-1: {e}"))?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut saw_stop = false;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match tokio::time::timeout(remaining, events.next()).await {
            Ok(Some(Ok(event))) => {
                if let berth_core::EventKind::Container { id, action } = &event.kind {
                    let named =
                        event.attributes.get("name").map(String::as_str) == Some("berthfx-web-1");
                    if (id == &web || named) && (action == "die" || action == "stop") {
                        saw_stop = true;
                        break;
                    }
                }
            }
            Ok(Some(Err(e))) => return Err(format!("events stream error: {e}")),
            Ok(None) => return Err("events stream ended before die/stop".to_string()),
            Err(_) => break,
        }
    }
    events_cancel.cancel();
    if !saw_stop {
        return Err("no die/stop event for berthfx-web-1 within 2s".to_string());
    }

    // Logs: tail 20 of the logger gives exactly 20 lines, both streams.
    let logs_cancel = CancellationToken::new();
    let query = berth_core::LogQuery {
        since: None,
        tail: Some(20),
        follow: false,
    };
    let lines = tokio::time::timeout(
        Duration::from_secs(10),
        client.logs(&logger, query, logs_cancel).collect::<Vec<_>>(),
    )
    .await
    .map_err(|_| "logs timed out".to_string())?;
    let lines: Vec<berth_core::LogLine> = lines
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("logs error: {e}"))?;
    if lines.len() != 20 {
        return Err(format!("logs tail 20 gave {} lines", lines.len()));
    }
    if !lines.iter().any(|l| l.stream == berth_core::Stream::Stdout)
        || !lines.iter().any(|l| l.stream == berth_core::Stream::Stderr)
    {
        return Err("logs tail 20 missing a stream".to_string());
    }
    if lines
        .iter()
        .any(|l| l.container != logger || l.ts.timestamp() <= 0)
    {
        return Err("log lines carry wrong container or timestamp".to_string());
    }

    // Stats: one sample of the db has memory in use.
    let stats_cancel = CancellationToken::new();
    let mut stats = client.stats(&db, stats_cancel.clone());
    let sample = tokio::time::timeout(Duration::from_secs(10), stats.next())
        .await
        .map_err(|_| "stats timed out".to_string())?
        .ok_or("stats stream ended empty")?
        .map_err(|e| format!("stats error: {e}"))?;
    stats_cancel.cancel();
    if sample.mem_used.0 == 0 {
        return Err(format!("stats mem_used is 0: {sample:?}"));
    }
    println!(
        "stats db: cpu={} mem_used={}",
        sample.cpu_pct, sample.mem_used
    );

    // Limits: 0.5 CPU on web is visible in a fresh container().
    let limits = berth_core::Limits {
        cpus: Some(0.5),
        ..Default::default()
    };
    client
        .update_limits(&web, &limits, &berth_core::RestartPolicy::No)
        .await
        .map_err(|e| format!("update_limits: {e}"))?;
    let detail = client
        .container(&web)
        .await
        .map_err(|e| format!("container after update_limits: {e}"))?;
    match detail.limits.cpus {
        Some(cpus) if (cpus - 0.5).abs() < 0.01 => {}
        other => return Err(format!("limits.cpus after update: {other:?}")),
    }

    // Pause/unpause round trip on a running web.
    client
        .act(&web, berth_core::ContainerAction::Start)
        .await
        .map_err(|e| format!("start berthfx-web-1: {e}"))?;
    client
        .act(&web, berth_core::ContainerAction::Pause)
        .await
        .map_err(|e| format!("pause berthfx-web-1: {e}"))?;
    let paused = client
        .container(&web)
        .await
        .map_err(|e| format!("container after pause: {e}"))?;
    if paused.container.state != berth_core::ContainerState::Paused {
        return Err(format!("state after pause: {:?}", paused.container.state));
    }
    client
        .act(&web, berth_core::ContainerAction::Unpause)
        .await
        .map_err(|e| format!("unpause berthfx-web-1: {e}"))?;
    let resumed = client
        .container(&web)
        .await
        .map_err(|e| format!("container after unpause: {e}"))?;
    if resumed.container.state != berth_core::ContainerState::Running {
        return Err(format!(
            "state after unpause: {:?}",
            resumed.container.state
        ));
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs the live engine and berthfx fixtures; run with BERTH_IT=1"]
async fn streams_and_actions() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let client = connect().await;
    let web = fixture_id(&client, "berthfx-web-1").await;

    let result = run_checks(&client).await;

    // Always leave berthfx-web-1 running, whatever the checks did. It may be
    // paused if a check failed between Pause and Unpause.
    if let Ok(detail) = client.container(&web).await {
        if detail.container.state == berth_core::ContainerState::Paused {
            let _ = client.act(&web, berth_core::ContainerAction::Unpause).await;
        }
    }
    if let Ok(detail) = client.container(&web).await {
        if detail.container.state != berth_core::ContainerState::Running {
            let _ = client.act(&web, berth_core::ContainerAction::Start).await;
        }
    }
    let final_detail = client.container(&web).await.expect("final web detail");
    assert_eq!(
        final_detail.container.state,
        berth_core::ContainerState::Running,
        "berthfx-web-1 must end running"
    );
    result.expect("streams and actions checks");
    println!("streams ok: events, logs(20, both streams), stats, limits, pause/unpause");
}
