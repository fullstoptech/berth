use std::sync::Arc;
use std::time::Duration;

use berth_core::Stream;
use berth_logs::{LogFilter, Query};
use chrono::{DateTime, TimeZone, Utc};
use futures::stream;
use tokio_util::sync::CancellationToken;

use super::*;

fn id(name: &str) -> ContainerId {
    ContainerId(name.to_string())
}

fn at(second: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_792_850_000 + second, 0)
        .single()
        .expect("ts")
}

fn line(container: &str, second: i64, stderr: bool, text: &str) -> LogLine {
    LogLine {
        container: id(container),
        ts: at(second),
        stream: if stderr {
            Stream::Stderr
        } else {
            Stream::Stdout
        },
        text: text.to_string(),
    }
}

fn names(list: &[&str]) -> BTreeMap<ContainerId, String> {
    list.iter().map(|n| (id(n), (*n).to_string())).collect()
}

/// Polls until `check` holds or five seconds pass.
async fn until(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..250 {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    check()
}

fn visible(core: &LogCore) -> usize {
    core.with_view(LogView::len).unwrap_or(0)
}

#[tokio::test]
async fn logs_session_feeds_batches_into_the_view_and_ends_with_its_streams() {
    let lines: Vec<LogLine> = (0..3000)
        .map(|i| {
            line(
                "web",
                i,
                i % 100 == 0,
                &if i % 100 == 0 {
                    format!("ERROR line {i}")
                } else {
                    format!("INFO line {i}")
                },
            )
        })
        .collect();
    let core = LogCore::start(
        names(&["web"]),
        move |_, _| vec![stream::iter(lines).boxed()],
        &CancellationToken::new(),
    );
    assert!(until(|| visible(&core) == 3000).await, "{}", visible(&core));
    // 3,000 lines cannot go through in one batch of at most 2,048.
    assert!(core.batches() >= 2, "{}", core.batches());
    let counts = core.with_view(|v| v.counts().clone()).expect("view");
    assert_eq!((counts.error, counts.info), (30, 2970));
    assert!(until(|| core.status() == LogStatus::Ended).await);
    core.close();
}

#[tokio::test]
async fn logs_first_lines_of_several_streams_are_merged_in_time_order() {
    // `api` answers at once with the later lines; `db` answers late with the earlier ones. Without
    // the merge window the db lines would land after the api lines.
    let api = vec![
        line("api", 5, false, "api 5"),
        line("api", 7, false, "api 7"),
    ];
    let db = vec![line("db", 4, false, "db 4"), line("db", 6, false, "db 6")];
    let late = stream::once(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        stream::iter(db)
    })
    .flatten()
    .boxed();
    // A window longer than the delay (it also closes when every stream has ended, so the test
    // does not wait for it): the answer cannot depend on how busy the machine is.
    let core = LogCore::start_merging(
        names(&["api", "db"]),
        move |_, _| vec![stream::iter(api).boxed(), late],
        &CancellationToken::new(),
        Duration::from_secs(30),
    );
    assert!(until(|| visible(&core) == 4).await, "{}", visible(&core));
    let texts: Vec<String> = core
        .with_view(|v| v.rows(0..4).into_iter().map(|r| r.text).collect())
        .expect("view");
    assert_eq!(texts, ["db 4", "api 5", "db 6", "api 7"]);
    core.close();
}

/// A stderr trace of `frames` frames for `container`, a stdout line landing after `noise_at`
/// trace lines (the engine hands over stdout and stderr in the order it read the two pipes).
fn trace_with_noise(container: &str, start: i64, noise_at: usize) -> Vec<LogLine> {
    let mut lines = vec![line(
        container,
        start,
        true,
        "Traceback (most recent call last):",
    )];
    for n in 1..=10 {
        lines.push(line(
            container,
            start,
            true,
            &format!("  File \"/app/worker.py\", line {}, in step_{n}", 40 + n),
        ));
    }
    lines.push(line(container, start, true, "RuntimeError: boom"));
    lines.insert(
        noise_at,
        line(container, start, false, "INFO logger cycle started"),
    );
    lines
}

fn hidden_of_tracebacks(core: &LogCore) -> Vec<berth_logs::Fold> {
    core.with_view(|v| {
        v.rows(0..v.len())
            .iter()
            .filter(|r| r.text.starts_with("Traceback"))
            .map(|r| r.fold)
            .collect()
    })
    .unwrap_or_default()
}

#[tokio::test]
async fn logs_a_trace_folds_whole_across_the_batch_cap_and_the_merge_window() {
    // One stream: the trace starts a few lines before the 2,048-line batch cap and runs past it.
    let mut lines: Vec<LogLine> = (0..2_044)
        .map(|i| line("web", i, false, &format!("INFO filler {i}")))
        .collect();
    lines.extend(trace_with_noise("web", 3_000, 6));
    let total = lines.len();
    let core = LogCore::start(
        names(&["web"]),
        move |_, _| vec![stream::iter(lines).boxed()],
        &CancellationToken::new(),
    );
    assert!(until(|| visible(&core) >= total - 10).await);
    assert!(core.batches() >= 2, "{}", core.batches());
    assert_eq!(
        hidden_of_tracebacks(&core),
        vec![berth_logs::Fold::Head { hidden: 10 }]
    );

    // Two streams merged in the window: each trace keeps its own frames.
    let a = trace_with_noise("a", 0, 4);
    let b = trace_with_noise("b", 0, 9);
    let core = LogCore::start_merging(
        names(&["a", "b"]),
        move |_, _| vec![stream::iter(a).boxed(), stream::iter(b).boxed()],
        &CancellationToken::new(),
        Duration::from_millis(100),
    );
    assert!(
        until(|| visible(&core) == 2 * 3).await,
        "{}",
        visible(&core)
    );
    assert_eq!(
        hidden_of_tracebacks(&core),
        vec![berth_logs::Fold::Head { hidden: 10 }; 2]
    );
}

#[tokio::test]
async fn logs_lines_that_arrive_while_paused_are_counted_for_the_pill() {
    let (sender, receiver) = futures::channel::mpsc::unbounded::<LogLine>();
    let core = LogCore::start(
        names(&["web"]),
        move |_, _| vec![receiver.boxed()],
        &CancellationToken::new(),
    );
    assert!(until(|| visible(&core) == 0 && core.with_view(|_| ()).is_some()).await);
    core.set_follow(false).await.expect("follow off");
    for i in 0..142 {
        sender
            .unbounded_send(line("web", i, false, &format!("INFO later {i}")))
            .expect("send");
    }
    assert!(until(|| visible(&core) == 142).await, "{}", visible(&core));
    assert_eq!(core.new_lines(), 142);
    // Following again forgets the count.
    core.set_follow(true).await.expect("follow on");
    assert_eq!(core.new_lines(), 0);
    sender
        .unbounded_send(line("web", 200, false, "INFO while following"))
        .expect("send");
    assert!(until(|| visible(&core) == 143).await);
    assert_eq!(core.new_lines(), 0, "lines while following do not count");
    core.close();
}

#[tokio::test]
async fn logs_lines_the_queue_dropped_reach_the_dropped_notice() {
    // More lines than the queue holds: the oldest never reach the view, and every line is either
    // in the view, pushed out of its ring, or counted by the queue.
    let total = DEFAULT_QUEUE_LINES + 50_000;
    let lines = (0..total).map(|i| {
        let second = i64::try_from(i).unwrap_or(0);
        line("web", second, false, "INFO flood")
    });
    let core = LogCore::start(
        names(&["web"]),
        move |_, _| vec![stream::iter(lines).boxed()],
        &CancellationToken::new(),
    );
    assert!(until(|| core.status() == LogStatus::Ended).await);
    let (shown, dropped) = core.with_view(|v| (v.len(), v.dropped())).expect("view");
    assert!(dropped > 0, "the queue must have overflowed");
    assert_eq!(shown as u64 + dropped, total as u64);
    core.close();
}

#[tokio::test]
async fn logs_commands_search_filter_and_report_a_bad_pattern() {
    let lines = vec![
        line("web", 1, false, "INFO ECONNREFUSED one"),
        line("web", 2, true, "ERROR boom"),
        line("web", 3, false, "INFO ECONNREFUSED two"),
    ];
    let mut view = LogView::new(100, names(&["web"]));
    view.push_batch(lines);
    let core = LogCore::seeded(view);

    let summary = core
        .set_search(Some(Query {
            text: "ECONNREFUSED".to_string(),
            regex: false,
            case_sensitive: false,
        }))
        .await
        .expect("answered");
    assert_eq!(summary.matches, 2);
    assert_eq!(core.search_error(), None);

    // A pattern that does not compile leaves the message for the field and no search.
    let bad = core
        .set_search(Some(Query {
            text: "(".to_string(),
            regex: true,
            case_sensitive: false,
        }))
        .await
        .expect("answered");
    assert_eq!(bad.matches, 0);
    assert!(core
        .search_error()
        .is_some_and(|e| e.contains("invalid pattern")));
    // A good one clears it again.
    let _ = core.set_search(None).await;
    assert_eq!(core.search_error(), None);

    // Filters run in the task and the view follows.
    let stderr_only = LogFilter {
        stdout: false,
        stderr: true,
        ..LogFilter::default()
    };
    core.run(move |v| v.set_filter(stderr_only))
        .await
        .expect("ran");
    assert_eq!(visible(&core), 1);
    core.close();
}

#[tokio::test]
async fn logs_only_the_newest_of_several_searches_runs() {
    let mut view = LogView::new(100, names(&["web"]));
    view.push_batch(vec![line("web", 1, false, "alpha beta")]);
    let core = LogCore::seeded(view);
    let query = |text: &str| {
        Some(Query {
            text: text.to_string(),
            regex: true,
            case_sensitive: false,
        })
    };
    // Three searches are queued before the task runs any: the bad one in the middle is stale.
    let first = core.set_search(query("alpha"));
    let stale = core.set_search(query("("));
    let last = core.set_search(query("beta"));
    let (_, _, last) = tokio::join!(first, stale, last);
    assert_eq!(last.expect("answered").matches, 1);
    assert_eq!(core.search_error(), None, "the stale bad pattern never ran");
    core.close();
}

#[test]
fn logs_series_follow_id_order_like_the_view() {
    let ids = [id("c"), id("a"), id("b")];
    let series = series_of(&ids);
    assert_eq!(series[&id("a")], series_for(0));
    assert_eq!(series[&id("b")], series_for(1));
    assert_eq!(series[&id("c")], series_for(2));
    // The view gives the same hues.
    let view = LogView::new(10, ids.iter().map(|i| (i.clone(), i.0.clone())).collect());
    for i in &ids {
        assert_eq!(view.series(i), series[i]);
    }
}

#[tokio::test]
async fn logs_save_writes_one_line_per_view_line() {
    // A trace folded under its head is written out in full: the file has what Copy would copy.
    let mut view = LogView::new(100, names(&["web"]));
    view.push_batch(vec![
        line("web", 1, false, "INFO started"),
        line("web", 2, true, "Error: connect ECONNREFUSED 10.0.0.1:5432"),
        line("web", 2, true, "    at Object.connect (node:net:1139:12)"),
        line("web", 2, true, "    at Pool.connect (/app/db.js:41:19)"),
        line("web", 3, false, "INFO done"),
    ]);
    let text = view.text(None, false);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("web.log");
    let written = write_lines(&path, &text).await.expect("written");
    let on_disk = std::fs::read_to_string(&path).expect("read back");
    assert_eq!(written, 5);
    assert_eq!(written, text.lines().count());
    assert_eq!(on_disk.lines().count(), written);
    assert_eq!(on_disk, format!("{text}\n"));
    // No half-written sibling is left behind.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .expect("dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["web.log"]);
    // Saving again replaces the file.
    let again = write_lines(&path, "one\ntwo").await.expect("rewritten");
    assert_eq!(again, 2);
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "one\ntwo\n");
    // Nothing to save: an empty file, zero lines.
    assert_eq!(write_lines(&path, "").await.expect("empty"), 0);
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "");
}

#[tokio::test]
async fn logs_save_failure_names_nothing_it_did_not_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("no-such-folder").join("x.log");
    assert!(write_lines(&missing, "a").await.is_err());
}

#[test]
fn logs_save_file_name_is_safe_and_dated() {
    let now = Utc
        .with_ymd_and_hms(2026, 10, 25, 9, 8, 7)
        .single()
        .expect("ts");
    assert_eq!(
        save_file_name("shop-api-worker-1", now),
        "shop-api-worker-1-20261025-090807.log"
    );
    assert_eq!(save_file_name("a/b c", now), "a-b-c-20261025-090807.log");
    assert_eq!(save_file_name("", now), "logs-20261025-090807.log");
}

#[tokio::test]
async fn logs_open_session_reads_the_engines_logs_stream() {
    let engine = berth_core::EngineId("colima:default".to_string());
    let mock = berth_engine::MockEngine::bundled(&engine);
    let worker = berth_engine::EngineClient::containers(&mock)
        .await
        .expect("containers")
        .into_iter()
        .find(|c| c.name == "shop-api-worker-1")
        .expect("worker in the mock world");
    let clients = crate::services::engines::client_map();
    clients
        .lock()
        .expect("clients")
        .insert(engine.clone(), Arc::new(mock));
    let services = Services {
        store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
        clients,
        cancel: CancellationToken::new(),
    };
    let target = LogTarget {
        engine,
        id: worker.id.clone(),
        name: worker.name.clone(),
    };
    let core = open_session(&services, &[target]);
    assert!(until(|| visible(&core) > 0).await);
    let rows = core.with_view(|v| v.rows(0..v.len())).expect("view");
    assert!(
        rows.iter()
            .any(|r| r.text.contains("ECONNREFUSED") && r.error_mark),
        "{rows:?}"
    );
    assert!(rows.iter().all(|r| r.container == "shop-api-worker-1"));
    core.close();
}

#[tokio::test]
async fn logs_a_container_on_an_engine_that_is_not_connected_marks_the_view_failed() {
    let services = Services {
        store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
        clients: crate::services::engines::client_map(),
        cancel: CancellationToken::new(),
    };
    let target = LogTarget {
        engine: berth_core::EngineId("ssh:gone".to_string()),
        id: id("c1"),
        name: "c1".to_string(),
    };
    let core = open_session(&services, &[target]);
    assert!(until(|| matches!(core.status(), LogStatus::Failed(_))).await);
    assert_eq!(
        core.status(),
        LogStatus::Failed("ssh:gone is not connected".to_string())
    );
    core.close();
}

#[tokio::test]
async fn logs_first_batch_costs_the_ui_path_under_the_budget() {
    // What the UI pays for the first batch: the window it reads (a screenful of rows) under the
    // lock, after the task built and warmed the view and applied 2,048 lines. Printed for the
    // record; the bound is generous because a debug build is an order of magnitude slower.
    let lines: Vec<LogLine> = (0..2048)
        .map(|i| line("web", i / 100, i % 7 == 0, &format!("INFO line {i}")))
        .collect();
    let core = LogCore::start(
        names(&["web"]),
        move |_, _| vec![stream::iter(lines).boxed()],
        &CancellationToken::new(),
    );
    assert!(until(|| visible(&core) == 2048).await);
    let started = std::time::Instant::now();
    let rows = core.with_view(|v| v.rows(0..60)).expect("view");
    let ui_cost = started.elapsed();
    println!(
        "first batch: push_batch {:?} in the task; the UI read of {} rows {ui_cost:?}",
        core.slowest_push(),
        rows.len()
    );
    assert_eq!(rows.len(), 60);
    assert!(ui_cost < Duration::from_millis(4), "{ui_cost:?}");
    core.close();
}
