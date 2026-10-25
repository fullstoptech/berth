//! PLAN §1.3 / D23 / D25: 10,000 lines a second through `batch`, `PreparedBatch::new` (outside the
//! view lock) and `LogView::commit_batch` (under it), at most one batch per 16 ms tick, and no
//! commit holding the lock for more than 4 ms. The default suite checks the counts (every line
//! reaches the view, in batches of at most `MAX_BATCH_LINES`, the same content as a synchronous
//! push); the rates and the lock hold are `perf_throughput`, which `scripts/perf.sh` runs in a
//! release build (AD26). `scripts/perf.sh` prints the numbers.
//!
//! Two measurements, because one number cannot say both things:
//! - saturated: the source never waits, so the rate is what the pipeline can do (CPU bound on the
//!   per-line work in `PreparedBatch::new`: level scan, error-word scan, JSON parse for JSON
//!   lines, fold facts; and in `commit_batch`: fold link, counts, filter check, eviction);
//! - paced: lines arrive at a steady rate above 10,000 a second, like a chatty container, and the
//!   batches must come once per tick with the lines of a whole tick in each.
//!
//! In both, the view that received the batches must hold exactly what an independent, synchronous
//! `push_batch` of the same lines holds, so the work provably happened.

#![allow(clippy::expect_used)] // clippy.toml allows expect in #[test] fns, not in helpers

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use berth_core::{ContainerId, LogLine, Stream};
use berth_logs::{batch_counted, LogView, PreparedBatch, MAX_BATCH_LINES};
use chrono::DateTime;
use futures::stream::{self, StreamExt};
use tokio_util::sync::CancellationToken;

const TICK: Duration = Duration::from_millis(16);
const RING: usize = 100_000;

/// Every sample in `fixtures/logs/`, in order: plain levels, logfmt, JSON, four stack-trace kinds,
/// nginx and Postgres lines.
fn corpus() -> Vec<String> {
    [
        include_str!("../../../fixtures/logs/plain_levels.txt"),
        include_str!("../../../fixtures/logs/logfmt.txt"),
        include_str!("../../../fixtures/logs/json.txt"),
        include_str!("../../../fixtures/logs/node_trace.txt"),
        include_str!("../../../fixtures/logs/python_trace.txt"),
        include_str!("../../../fixtures/logs/java_trace.txt"),
        include_str!("../../../fixtures/logs/go_panic.txt"),
        include_str!("../../../fixtures/logs/nginx_access.txt"),
        include_str!("../../../fixtures/logs/postgres.txt"),
    ]
    .iter()
    .flat_map(|sample| sample.lines().map(str::to_string))
    .collect()
}

fn lines(count: usize) -> Vec<LogLine> {
    let corpus = corpus();
    (0..count)
        .map(|i| LogLine {
            container: ContainerId("berthfx-logger-1".to_string()),
            ts: DateTime::from_timestamp(1_761_297_240 + (i / 1000) as i64, 0).expect("ts"),
            stream: if i % 7 == 0 {
                Stream::Stderr
            } else {
                Stream::Stdout
            },
            text: corpus[i % corpus.len()].clone(),
        })
        .collect()
}

struct Run {
    view: LogView,
    total: usize,
    batches: usize,
    wall: Duration,
    /// Total time in `PreparedBatch::new`, which the app runs outside the view lock.
    prepare: Duration,
    /// Total time in `LogView::commit_batch`, which the app runs under the view lock.
    commit: Duration,
    largest_batch: usize,
    /// Longest single `commit_batch`: how long the UI can be kept from reading the view.
    max_commit: Duration,
}

impl Run {
    fn lines_per_second(&self) -> f64 {
        self.total as f64 / self.wall.as_secs_f64()
    }

    /// Ticks that fit in the run, counting the first (an interval fires at once).
    fn ticks(&self) -> usize {
        (self.wall.as_millis() / TICK.as_millis()) as usize + 1
    }

    fn report(&self, label: &str) {
        println!(
            "{label}: {total} lines in {wall:.1?} = {rate:.0} lines/s; \
             {batches} batches in {ticks} ticks ({per_tick:.2} batches per 16 ms tick, \
             largest batch {largest} lines); lock hold (max commit) {max_commit:.2?}; \
             prepare (unlocked) {prepare:.1?} = {prepare_rate:.0} lines/s, commit (locked) {commit:.1?} = {commit_rate:.0} lines/s \
             ({busy:.0}% of wall under the lock)",
            total = self.total,
            wall = self.wall,
            rate = self.lines_per_second(),
            batches = self.batches,
            ticks = self.ticks(),
            per_tick = self.batches as f64 / self.ticks() as f64,
            largest = self.largest_batch,
            max_commit = self.max_commit,
            prepare = self.prepare,
            prepare_rate = self.total as f64 / self.prepare.as_secs_f64(),
            commit = self.commit,
            commit_rate = self.total as f64 / self.commit.as_secs_f64(),
            busy = 100.0 * self.commit.as_secs_f64() / self.wall.as_secs_f64(),
        );
    }
}

/// Feeds `source` through `batch` into a fresh view and times it.
async fn run(source: stream::BoxStream<'static, LogLine>, total: usize) -> Run {
    let mut view = LogView::new(RING, BTreeMap::new());
    let started = Instant::now();
    // The queue holds the whole run, so nothing is dropped and every line must reach the view.
    let (mut rx, drops) = batch_counted(vec![source], TICK, CancellationToken::new(), total);
    let mut max_commit = Duration::ZERO;
    let (mut batches, mut seen, mut largest) = (0, 0, 0);
    let (mut prepare, mut commit) = (Duration::ZERO, Duration::ZERO);
    while let Some(b) = rx.recv().await {
        assert!(!b.is_empty(), "batch yielded an empty Vec");
        batches += 1;
        seen += b.len();
        largest = largest.max(b.len());
        assert!(b.len() <= MAX_BATCH_LINES, "batch of {} lines", b.len());
        // The two halves the app runs: prepare with no lock held, then the commit under it.
        let t = Instant::now();
        let prepared = PreparedBatch::new(b);
        prepare += t.elapsed();
        let t = Instant::now();
        view.commit_batch(prepared);
        let took = t.elapsed();
        commit += took;
        max_commit = max_commit.max(took);
    }
    let wall = started.elapsed();
    assert_eq!(drops.total(), 0);
    assert_eq!(seen, total, "lines lost between the source and the view");
    Run {
        view,
        total,
        batches,
        wall,
        prepare,
        commit,
        largest_batch: largest,
        max_commit,
    }
}

/// The same lines pushed synchronously in one batch: the answer the pipeline must agree with.
fn reference(all: Vec<LogLine>) -> LogView {
    let mut view = LogView::new(RING, BTreeMap::new());
    view.push_batch(all);
    view
}

fn assert_same_content(got: &LogView, want: &LogView) {
    assert_eq!(got.len(), want.len(), "visible rows");
    assert_eq!(got.dropped(), want.dropped(), "dropped lines");
    assert_eq!(got.counts(), want.counts(), "level counts");
    assert_eq!(
        got.text(None, true),
        want.text(None, true),
        "view content differs from a synchronous push of the same lines"
    );
}

async fn saturated_source() -> Run {
    const TOTAL: usize = 120_000;
    let all = lines(TOTAL);
    let want = reference(all.clone());
    let run = run(stream::iter(all).boxed(), TOTAL).await;

    assert!(
        run.batches >= TOTAL.div_ceil(MAX_BATCH_LINES),
        "a saturated source must be spread over batches of at most {MAX_BATCH_LINES} lines"
    );
    // More lines than the ring holds: the oldest were dropped and the view says so.
    assert_eq!(run.view.dropped(), (TOTAL - RING) as u64);
    assert_same_content(&run.view, &want);
    run
}

/// The saturated run's rate and tick count: wall-clock, so perf only.
fn assert_saturated_timing(run: &Run) {
    assert!(
        run.lines_per_second() >= 10_000.0,
        "{:.0} lines/s is under the 10,000 lines/s budget",
        run.lines_per_second()
    );
    assert!(
        run.batches <= run.ticks(),
        "{} batches in {} ticks: more than one per tick",
        run.batches,
        run.ticks()
    );
}

async fn paced_source_batches_once_per_tick() -> Run {
    // 15 lines every millisecond for 1.2 s = 15,000 lines/s, above the 10,000 lines/s budget.
    const PER_MS: usize = 15;
    const MS: usize = 1200;
    let total = PER_MS * MS;
    let all = lines(total);
    let want = reference(all.clone());

    let source = stream::unfold(
        (
            all.into_iter(),
            tokio::time::interval(Duration::from_millis(1)),
            0_usize,
        ),
        |(mut pending, mut ticker, n)| async move {
            ticker.tick().await;
            let chunk: Vec<LogLine> = pending.by_ref().take(PER_MS).collect();
            (n < MS && !chunk.is_empty()).then(|| (chunk, (pending, ticker, n + 1)))
        },
    )
    .flat_map(stream::iter)
    .boxed();

    let run = run(source, total).await;

    assert!(
        run.lines_per_second() >= 10_000.0,
        "{:.0} lines/s is under the 10,000 lines/s budget",
        run.lines_per_second()
    );
    assert!(
        run.batches <= run.ticks(),
        "{} batches in {} ticks: more than one per tick",
        run.batches,
        run.ticks()
    );
    let per_batch = total / run.batches;
    assert!(
        per_batch >= 20,
        "{per_batch} lines per batch: lines are not being gathered per tick"
    );
    assert_same_content(&run.view, &want);
    run
}

/// CLAUDE.md: no work over 4 ms on the UI path. AD23: what the UI can wait for is the locked
/// commit, so that is the time held to the budget (the unlocked prepare is only reported).
const LOCK_HOLD_BUDGET: Duration = Duration::from_millis(4);

/// Prints the run and holds its longest lock hold to [`LOCK_HOLD_BUDGET`] (a debug build is an
/// order of magnitude slower, so there the bound is ten times wider).
fn report_within_lock_budget(label: &str, run: &Run) {
    run.report(label);
    let budget = if cfg!(debug_assertions) {
        LOCK_HOLD_BUDGET * 10
    } else {
        LOCK_HOLD_BUDGET
    };
    assert!(
        run.max_commit <= budget,
        "{label}: the view lock was held for {:?} by one commit_batch of at most {MAX_BATCH_LINES} lines, over the {budget:?} budget",
        run.max_commit
    );
}

/// Counts only: no line is lost or reordered between the source and the view, however fast the
/// source is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn throughput_saturated_source_reaches_the_view_in_bounded_batches() {
    let run = saturated_source().await;
    assert!(run.largest_batch <= MAX_BATCH_LINES);
}

/// One test, not two: a saturated run on the next core would stretch the paced run's commit
/// times. Wall-clock, so `scripts/perf.sh` only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "timing; run by scripts/perf.sh"]
async fn perf_throughput() {
    let saturated = saturated_source().await;
    assert_saturated_timing(&saturated);
    report_within_lock_budget("saturated", &saturated);
    report_within_lock_budget(
        "paced 15,000 lines/s",
        &paced_source_batches_once_per_tick().await,
    );
}
