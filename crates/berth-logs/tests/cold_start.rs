//! The first `push_batch` on a fresh `LogView` costs a little more than a later one (lazy regexes,
//! first-touch memory). The app builds its view and pushes a throwaway batch in the service task,
//! off the UI path. Run with `--release --no-capture cold_start` to see the times.

#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use berth_core::{ContainerId, LogLine, Stream};
use berth_logs::LogView;
use chrono::DateTime;

fn batch(n: usize, seed: usize) -> Vec<LogLine> {
    let texts = [
        "INFO logger cycle started",
        "WARN cache is nearly full",
        "{\"level\":\"info\",\"msg\":\"request ok\",\"cycle\":1,\"ms\":12}",
        "ERROR simulated failure",
        "Traceback (most recent call last):",
        "  File \"/app/worker.py\", line 41, in step_1",
        "level=info msg=\"GET /health\" status=200",
        "    at Object.connect (node:net:1139:12)",
    ];
    (0..n)
        .map(|i| LogLine {
            container: ContainerId("berthfx-logger-1".to_string()),
            ts: DateTime::from_timestamp(1_761_297_240 + (i / 100) as i64, 0).expect("ts"),
            stream: if i % 4 == 3 {
                Stream::Stderr
            } else {
                Stream::Stdout
            },
            text: format!("{} {}", texts[(i + seed) % texts.len()], i),
        })
        .collect()
}

/// (first batch, later batches) of 2,048 lines each.
fn run(label: &str, warm: bool) -> (Duration, Vec<Duration>) {
    if warm {
        // What the app's service does before the first real batch.
        let mut throwaway = LogView::new(1000, BTreeMap::new());
        throwaway.push_batch(batch(64, 0));
    }
    let t = Instant::now();
    let mut view = LogView::new(100_000, BTreeMap::new());
    let build = t.elapsed();
    let mut times = Vec::new();
    for k in 0..6 {
        let b = batch(2048, k);
        let t = Instant::now();
        view.push_batch(b);
        times.push(t.elapsed());
    }
    println!(
        "{label}: LogView::new {build:.2?}; first push_batch {:?}; later {:?}",
        times[0],
        &times[1..]
    );
    (times[0], times[1..].to_vec())
}

#[test]
fn cold_start_first_batch_cost() {
    let (first, _) = run("cold", false);
    // Generous: a debug build is 10x slower than release; this only guards against a pathological
    // up-front cost (the 47 ms once seen).
    assert!(first < Duration::from_millis(200), "{first:?}");
}

#[test]
fn cold_start_first_batch_cost_after_warm_up() {
    let (first, _) = run("warm", true);
    assert!(first < Duration::from_millis(200), "{first:?}");
}
