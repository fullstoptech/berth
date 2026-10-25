//! PLAN §1.3 / D25: 10,000 lines a second through `batch` and `LogView::push_batch`, at most one
//! batch per 16 ms tick. Run with `cargo nextest run -p berth-logs --release --no-capture throughput`
//! to see the numbers; the assertions hold in a debug build too.
//!
//! Two measurements, because one number cannot say both things:
//! - saturated: the source never waits, so the rate is what the pipeline can do (CPU bound on the
//!   per-line work in `LogBuffer::push`: level scan, error-word scan, JSON parse for JSON lines,
//!   fold bookkeeping, filter check);
//! - paced: lines arrive at a steady rate above 10,000 a second, like a chatty container, and the
//!   batches must come once per tick with the lines of a whole tick in each.
//!
//! In both, the view that received the batches must hold exactly what an independent, synchronous
//! `push_batch` of the same lines holds, so the work provably happened.

#![allow(clippy::expect_used)] // clippy.toml allows expect in #[test] fns, not in helpers

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use berth_core::{ContainerId, LogLine, Stream};
use berth_logs::{batch_counted, LogView, MAX_BATCH_LINES};
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
    in_view: Duration,
    largest_batch: usize,
    /// Slowest single `push_batch`: what the UI path would wait for.
    max_push: Duration,
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
             largest batch {largest} lines); slowest push_batch {max_push:.2?}; view.push_batch alone = {view_rate:.0} lines/s \
             ({busy:.0}% of wall)",
            total = self.total,
            wall = self.wall,
            rate = self.lines_per_second(),
            batches = self.batches,
            ticks = self.ticks(),
            per_tick = self.batches as f64 / self.ticks() as f64,
            largest = self.largest_batch,
            max_push = self.max_push,
            view_rate = self.total as f64 / self.in_view.as_secs_f64(),
            busy = 100.0 * self.in_view.as_secs_f64() / self.wall.as_secs_f64(),
        );
    }
}

/// Feeds `source` through `batch` into a fresh view and times it.
async fn run(source: stream::BoxStream<'static, LogLine>, total: usize) -> Run {
    let mut view = LogView::new(RING, BTreeMap::new());
    let started = Instant::now();
    // The queue holds the whole run, so nothing is dropped and every line must reach the view.
    let (mut rx, drops) = batch_counted(vec![source], TICK, CancellationToken::new(), total);
    let mut max_push = Duration::ZERO;
    let (mut batches, mut seen, mut largest) = (0, 0, 0);
    let mut in_view = Duration::ZERO;
    while let Some(b) = rx.recv().await {
        assert!(!b.is_empty(), "batch yielded an empty Vec");
        batches += 1;
        seen += b.len();
        largest = largest.max(b.len());
        assert!(b.len() <= MAX_BATCH_LINES, "batch of {} lines", b.len());
        let t = Instant::now();
        view.push_batch(b);
        let took = t.elapsed();
        in_view += took;
        max_push = max_push.max(took);
    }
    let wall = started.elapsed();
    assert_eq!(drops.total(), 0);
    assert_eq!(seen, total, "lines lost between the source and the view");
    Run {
        view,
        total,
        batches,
        wall,
        in_view,
        largest_batch: largest,
        max_push,
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
    assert!(
        run.batches >= TOTAL.div_ceil(MAX_BATCH_LINES),
        "a saturated source must be spread over batches of at most {MAX_BATCH_LINES} lines"
    );
    // More lines than the ring holds: the oldest were dropped and the view says so.
    assert_eq!(run.view.dropped(), (TOTAL - RING) as u64);
    assert_same_content(&run.view, &want);
    run
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

/// CLAUDE.md: no work over 4 ms on the UI path.
const PUSH_BUDGET: Duration = Duration::from_millis(4);

/// Runs `attempt` and holds its slowest `push_batch` to [`PUSH_BUDGET`] in a release build (a debug
/// build is only checked for correctness). Other processes can only make a batch slower, never
/// faster, so a release run gets five tries and passes when one stays in budget; the cost of the
/// code itself is the lowest of the five.
async fn within_push_budget<F, Fut>(label: &str, attempt: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Run>,
{
    let tries = if cfg!(debug_assertions) { 1 } else { 5 };
    let mut slowest = Vec::new();
    for _ in 0..tries {
        let run = attempt().await;
        run.report(label);
        if cfg!(debug_assertions) || run.max_push <= PUSH_BUDGET {
            return;
        }
        slowest.push(run.max_push);
    }
    panic!("slowest push_batch per attempt {slowest:?}: over the 4 ms UI budget every time");
}

/// One test, not two: nextest runs tests in parallel, and a saturated run on the next core would
/// stretch the paced run's `push_batch` times.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn throughput() {
    within_push_budget("saturated", saturated_source).await;
    within_push_budget("paced 15,000 lines/s", paced_source_batches_once_per_tick).await;
}
