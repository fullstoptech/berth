#![allow(clippy::expect_used)]
//! A log view over the live berthfx logger (P3-T2 done-when). Run directly:
//! `BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only logs_it --no-capture` (the
//! fixtures must be up; this test never touches them beyond reading `berthfx-logger-1`).
//!
//! The logger repeats, every 2 s at its default rate: an INFO line, a WARN line, a JSON line, an
//! `ERROR … simulated failure` line on stderr, a Python traceback of ten frames and a
//! `RuntimeError: simulated failure …` line. Following it for about five seconds must show that
//! lines arrive, error lines carry the mark, the trace folds under its head, F8 moves between
//! error rows, and searching "simulated failure" reads "N of M".

mod common;

use std::sync::Arc;
use std::time::Duration;

use berth_app::services::engines::client_map;
use berth_app::services::logs::{open_session, LogStatus, LogTarget};
use berth_app::services::Services;
use berth_app::ui::log_toolbar::search_count_label;
use berth_logs::{Fold, Query};
use tokio_util::sync::CancellationToken;

async fn live_client() -> (berth_core::Engine, Arc<dyn berth_engine::EngineClient>) {
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
    let client: Arc<dyn berth_engine::EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    (engine, client)
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn logs_it_follows_the_logger_marks_errors_folds_the_trace_and_searches() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    common::settle_fixture().await;
    let (engine, client) = live_client().await;
    let logger = client
        .containers()
        .await
        .expect("list")
        .into_iter()
        .find(|c| c.name == "berthfx-logger-1")
        .expect("berthfx-logger-1 listed");
    let clients = client_map();
    clients
        .lock()
        .expect("client map")
        .insert(engine.id.clone(), Arc::clone(&client));
    let services = Services {
        store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
        clients,
        cancel: CancellationToken::new(),
    };
    let target = LogTarget {
        engine: engine.id.clone(),
        id: logger.id.clone(),
        name: logger.name.clone(),
    };

    let core = open_session(&services, &[target]);
    // The first lines (the tail the logger already wrote) arrive at once.
    let started = std::time::Instant::now();
    while core.with_view(|v| v.len()).unwrap_or(0) == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "no line within 10 s"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let at_first = core.with_view(|v| v.len()).expect("view");
    let first_seq = core
        .with_view(|v| v.rows(0..v.len()).last().map(|r| r.seq))
        .expect("view");

    // Follow for about five seconds: at the default rate that is two or three more cycles.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let later = core.with_view(|v| v.len()).expect("view");
    let last_seq = core
        .with_view(|v| v.rows(0..v.len()).last().map(|r| r.seq))
        .expect("view");
    eprintln!(
        "lines at first {at_first}, after 5 s {later}; batches {}; slowest push_batch {:?}; status {:?}",
        core.batches(),
        core.slowest_push(),
        core.status()
    );
    assert!(
        last_seq > first_seq,
        "new lines kept arriving while following"
    );
    assert_eq!(core.status(), LogStatus::Following);

    // Error lines carry the mark; the traceback folds ten frames under its head.
    let (marked, head_fold, error_rows) = core
        .with_view(|v| {
            let rows = v.rows(0..v.len());
            let marked = rows
                .iter()
                .filter(|r| r.error_mark && r.text.contains("simulated failure"))
                .count();
            let head_fold = rows
                .iter()
                .find(|r| r.text.starts_with("Traceback"))
                .map(|r| r.fold);
            let error_rows = rows.iter().filter(|r| r.error_mark).count();
            (marked, head_fold, error_rows)
        })
        .expect("view");
    assert!(
        marked >= 2,
        "the ERROR and RuntimeError lines are marked: {marked}"
    );
    assert!(error_rows >= marked);
    assert_eq!(head_fold, Some(Fold::Head { hidden: 10 }));
    let counts = core.with_view(|v| v.counts().clone()).expect("view");
    assert!(counts.error as usize >= marked, "{counts:?}");
    assert!(counts.warn >= 1 && counts.info >= 1, "{counts:?}");

    // F8 moves from error row to error row; Shift+F8 goes back.
    let first_error = core
        .run(|v| v.next_error(usize::MAX))
        .await
        .flatten()
        .expect("an error row");
    let second_error = core
        .run(move |v| v.next_error(first_error))
        .await
        .flatten()
        .expect("another error row");
    assert_ne!(first_error, second_error, "F8 moved");
    let moved_to_error = core
        .with_view(|v| v.rows(second_error..second_error + 1)[0].error_mark)
        .expect("view");
    assert!(moved_to_error);
    let back = core
        .run(move |v| v.prev_error(second_error))
        .await
        .flatten();
    assert_eq!(back, Some(first_error), "Shift+F8 goes back");

    // Search "simulated failure": "N of M".
    let summary = core
        .set_search(Some(Query {
            text: "simulated failure".to_string(),
            regex: false,
            case_sensitive: false,
        }))
        .await
        .expect("searched");
    assert!(summary.matches >= 2, "{summary:?}");
    let _ = core.run(|v| v.next_match()).await;
    // The logger keeps writing while this runs, so the label and the match count it must agree
    // with are read under one lock; `summary.matches` is only the count at the time of the search.
    let (label, matches_now) = core
        .with_view(|v| {
            let now = v.search_summary();
            (search_count_label(&now), now.matches)
        })
        .expect("view");
    let label = label.expect("a label");
    eprintln!("search label: {label}");
    assert!(
        matches_now >= summary.matches,
        "matches only grow while following"
    );
    assert_eq!(label, format!("1 of {matches_now}"));

    // Save: the file's line count is the view's.
    let text = core
        .run(|v| v.text(None, false))
        .await
        .expect("text of the view");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("logger.log");
    let written = berth_app::services::logs::write_lines(&path, &text)
        .await
        .expect("saved");
    assert_eq!(written, text.lines().count());
    assert_eq!(
        std::fs::read_to_string(&path)
            .expect("read")
            .lines()
            .count(),
        written
    );
    core.close();
}

/// Delivery rate of the live logger: run it against `LOGGER_RATE=10000 scripts/fx-up.sh` with
/// `BERTH_LOGS_RATE=1 BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only logs_it_rate --no-capture`.
/// Prints, once a second for ten seconds, the lines the view has taken in (`seq` counts every
/// pushed line, folded or not), the lines dropped (queue overflow plus ring evictions) and the
/// slowest `push_batch`. It asserts only that lines kept flowing and the UI path stayed in budget.
#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1 BERTH_LOGS_RATE=1"]
async fn logs_it_rate_reports_delivered_lines_per_second_and_drops() {
    if std::env::var("BERTH_IT").is_err() || std::env::var("BERTH_LOGS_RATE").is_err() {
        eprintln!("BERTH_IT and BERTH_LOGS_RATE not both set, skipping");
        return;
    }
    common::settle_fixture().await;
    let (engine, client) = live_client().await;
    let logger = client
        .containers()
        .await
        .expect("list")
        .into_iter()
        .find(|c| c.name == "berthfx-logger-1")
        .expect("berthfx-logger-1 listed");
    let clients = client_map();
    clients
        .lock()
        .expect("client map")
        .insert(engine.id.clone(), Arc::clone(&client));
    let services = Services {
        store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
        clients,
        cancel: CancellationToken::new(),
    };
    let core = open_session(
        &services,
        &[LogTarget {
            engine: engine.id.clone(),
            id: logger.id.clone(),
            name: logger.name.clone(),
        }],
    );
    let pushed = |core: &berth_app::services::logs::LogCore| {
        core.with_view(|v| v.rows(0..v.len()).last().map_or(0, |r| r.seq + 1))
            .unwrap_or(0)
    };
    let started = std::time::Instant::now();
    while pushed(&core) == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "no line in 10 s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (t0, at0) = (std::time::Instant::now(), pushed(&core));
    let mut last = at0;
    for second in 1..=10 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let now = pushed(&core);
        let dropped = core.with_view(|v| v.dropped()).expect("view");
        eprintln!(
            "t={second:>2}s  +{:>6} lines  total {now:>7}  dropped {dropped}  slowest push {:?}",
            now - last,
            core.slowest_push()
        );
        last = now;
    }
    let seconds = t0.elapsed().as_secs_f64();
    #[allow(clippy::cast_precision_loss)]
    let rate = (last - at0) as f64 / seconds;
    let dropped = core.with_view(|v| v.dropped()).expect("view");
    eprintln!(
        "delivered {} lines in {seconds:.2} s = {rate:.0} lines/s; dropped {dropped}; batches {}; slowest push_batch {:?}; status {:?}",
        last - at0,
        core.batches(),
        core.slowest_push(),
        core.status()
    );
    assert!(last > at0, "lines kept flowing");
    core.close();
}
