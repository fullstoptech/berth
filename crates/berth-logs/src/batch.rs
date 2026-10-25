//! Many log streams in, one `Vec` per tick out.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use berth_core::LogLine;
use futures::stream::{select_all, BoxStream};
use futures::{FutureExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

/// Batches in flight between the collector and the UI service. Small on purpose: when the UI is
/// behind, the collector waits and lines pile up in the next batch instead of in the channel.
const CHANNEL_DEPTH: usize = 4;

/// Lines taken per pass without looking at the clock. Bounds how late a tick can run when a
/// stream never has to wait.
const DRAIN_STEP: usize = 1024;

/// Most lines in one emitted batch, so that the locked half of a push (`LogView::commit_batch`,
/// which holds the view lock the UI reads, D23) stays under the 4 ms budget of CLAUDE.md. The
/// other half, `PreparedBatch::new`, runs outside the lock and does not count against it.
/// Derivation: the lock hold is the worst batch, not the mean, and a batch that meets a cold
/// cache, first-touch memory or a preempted thread takes several times its average. A whole
/// `push_batch` of 2,048 lines took 6-18 ms under machine load, so the cap bounds how many lines
/// that worst case can multiply. At 1,024 lines one batch per 16 ms tick still carries 64,000
/// lines a second, 6 times the 10,000 lines a second requirement.
pub const MAX_BATCH_LINES: usize = 1024;

/// Lines held for later ticks by [`batch`]: the size of a view's ring (D25). Lines beyond that
/// would be pushed out of the ring as soon as they arrived, so the oldest are dropped here.
pub const DEFAULT_QUEUE_LINES: usize = 100_000;

const _: () = assert!(MAX_BATCH_LINES <= DEFAULT_QUEUE_LINES);

/// Lines [`batch_counted`] dropped because the queue was full. Hand the total to
/// [`LogView::add_dropped`](crate::LogView::add_dropped) so the "older lines dropped" notice
/// counts them.
#[derive(Debug, Clone, Default)]
pub struct BatchDrops(Arc<AtomicU64>);

impl BatchDrops {
    /// Lines dropped so far.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Collects lines from many streams and yields one `Vec` of at most [`MAX_BATCH_LINES`] per `tick`
/// (16 ms in the app), never an empty one. Ticks that find nothing send nothing. Lines over the
/// cap wait for later ticks, in order, in a queue of [`DEFAULT_QUEUE_LINES`]; when that overflows
/// the oldest are dropped (use [`batch_counted`] to learn how many). When every stream has ended
/// the remaining lines go out on the following ticks and the receiver then closes; cancelling the
/// token or dropping the receiver stops the task at once.
///
/// Must be called inside a tokio runtime.
#[must_use]
pub fn batch(
    streams: Vec<BoxStream<'static, LogLine>>,
    tick: Duration,
    cancel: CancellationToken,
) -> mpsc::Receiver<Vec<LogLine>> {
    batch_counted(streams, tick, cancel, DEFAULT_QUEUE_LINES).0
}

/// [`batch`] with a chosen queue size (at least [`MAX_BATCH_LINES`]), and the count of lines it
/// dropped on overflow.
#[must_use]
pub fn batch_counted(
    streams: Vec<BoxStream<'static, LogLine>>,
    tick: Duration,
    cancel: CancellationToken,
    queue_lines: usize,
) -> (mpsc::Receiver<Vec<LogLine>>, BatchDrops) {
    let (tx, rx) = mpsc::channel(CHANNEL_DEPTH);
    let drops = BatchDrops::default();
    let counter = drops.0.clone();
    let queue_lines = queue_lines.max(MAX_BATCH_LINES);
    tokio::spawn(async move {
        let mut merged = select_all(streams);
        let mut ticker = interval(tick.max(Duration::from_millis(1)));
        // A late tick is not made up for: two batches never sit closer than one tick.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut queue: VecDeque<LogLine> = VecDeque::new();
        let mut ended = false;
        let enqueue = |queue: &mut VecDeque<LogLine>, line: LogLine| {
            queue.push_back(line);
            if queue.len() > queue_lines {
                queue.pop_front();
                counter.fetch_add(1, Ordering::Relaxed);
            }
        };
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                () = tx.closed() => break,
                _ = ticker.tick() => {
                    if !queue.is_empty() {
                        let n = queue.len().min(MAX_BATCH_LINES);
                        let out: Vec<LogLine> = queue.drain(..n).collect();
                        if tx.send(out).await.is_err() {
                            break;
                        }
                    }
                    if ended && queue.is_empty() {
                        break;
                    }
                }
                next = merged.next(), if !ended => match next {
                    Some(line) => {
                        enqueue(&mut queue, line);
                        // Take what is already waiting without going back through the select.
                        for _ in 1..DRAIN_STEP {
                            let Some(more) = merged.next().now_or_never() else { break };
                            match more {
                                Some(line) => enqueue(&mut queue, line),
                                None => {
                                    ended = true;
                                    break;
                                }
                            }
                        }
                    }
                    None => ended = true,
                },
            }
        }
    });
    (rx, drops)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use berth_core::{ContainerId, Stream};
    use chrono::DateTime;
    use futures::stream;

    use super::*;

    fn line(container: &str, n: usize) -> LogLine {
        LogLine {
            container: ContainerId(container.to_string()),
            ts: DateTime::from_timestamp(1_761_297_240, 0).expect("ts"),
            stream: Stream::Stdout,
            text: format!("{container} {n}"),
        }
    }

    fn lines(container: &str, count: usize) -> BoxStream<'static, LogLine> {
        let container = container.to_string();
        stream::iter((0..count).map(move |n| line(&container, n))).boxed()
    }

    async fn drain(mut rx: mpsc::Receiver<Vec<LogLine>>) -> Vec<Vec<LogLine>> {
        let mut batches = Vec::new();
        while let Some(b) = rx.recv().await {
            batches.push(b);
        }
        batches
    }

    #[tokio::test]
    async fn merges_streams_without_losing_or_reordering_within_a_stream() {
        let rx = batch(
            vec![lines("a", 500), lines("b", 500)],
            Duration::from_millis(16),
            CancellationToken::new(),
        );
        let all: Vec<LogLine> = drain(rx).await.into_iter().flatten().collect();
        assert_eq!(all.len(), 1000);
        for id in ["a", "b"] {
            let texts: Vec<&str> = all
                .iter()
                .filter(|l| l.container.0 == id)
                .map(|l| l.text.as_str())
                .collect();
            let want: Vec<String> = (0..500).map(|n| format!("{id} {n}")).collect();
            assert_eq!(texts, want.iter().map(String::as_str).collect::<Vec<_>>());
        }
    }

    #[tokio::test]
    async fn never_yields_an_empty_batch_and_closes_when_streams_end() {
        let rx = batch(
            vec![lines("a", 3)],
            Duration::from_millis(5),
            CancellationToken::new(),
        );
        let batches = drain(rx).await;
        assert!(!batches.is_empty());
        assert!(batches.iter().all(|b| !b.is_empty()));
        assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), 3);
    }

    #[tokio::test]
    async fn no_streams_closes_the_receiver() {
        let rx = batch(
            Vec::new(),
            Duration::from_millis(5),
            CancellationToken::new(),
        );
        assert!(drain(rx).await.is_empty());
    }

    #[tokio::test]
    async fn slow_stream_arrives_in_tick_sized_batches() {
        let slow = stream::unfold(0_usize, |n| async move {
            tokio::time::sleep(Duration::from_millis(2)).await;
            (n < 60).then(|| (line("a", n), n + 1))
        })
        .boxed();
        let mut rx = batch(
            vec![slow],
            Duration::from_millis(20),
            CancellationToken::new(),
        );
        let started = Instant::now();
        let mut got = Vec::new();
        let mut stamps = Vec::new();
        while let Some(b) = rx.recv().await {
            stamps.push(started.elapsed());
            got.push(b.len());
        }
        assert_eq!(got.iter().sum::<usize>(), 60);
        assert!(got.len() >= 3, "expected several batches, got {got:?}");
        // Receive times jitter on a busy machine, so count instead of measuring gaps: no more
        // batches than ticks that fit in the run (the first tick fires at once and sends nothing).
        let ticks = stamps.last().map_or(0, |t| t.as_millis() / 20) + 1;
        assert!(
            got.len() as u128 <= ticks,
            "{} batches in {ticks} ticks: {stamps:?}",
            got.len()
        );
    }

    #[tokio::test]
    async fn cancel_stops_a_stream_that_never_ends() {
        let cancel = CancellationToken::new();
        let endless = stream::pending::<LogLine>().boxed();
        let mut rx = batch(vec![endless], Duration::from_millis(5), cancel.clone());
        cancel.cancel();
        let end = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
        assert_eq!(end.expect("task stopped"), None);
    }

    struct SetOnDrop(Arc<AtomicBool>);

    impl Drop for SetOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn dropping_the_receiver_releases_the_streams() {
        let released = Arc::new(AtomicBool::new(false));
        let guard = SetOnDrop(released.clone());
        let endless = stream::pending::<LogLine>()
            .map(move |l| {
                let _keep = &guard;
                l
            })
            .boxed();
        let rx = batch(
            vec![endless],
            Duration::from_millis(5),
            CancellationToken::new(),
        );
        drop(rx);
        for _ in 0..100 {
            if released.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the collector kept its streams after the receiver was dropped");
    }

    async fn drain_counted(
        (mut rx, drops): (mpsc::Receiver<Vec<LogLine>>, BatchDrops),
    ) -> (Vec<Vec<LogLine>>, u64) {
        let mut batches = Vec::new();
        while let Some(b) = rx.recv().await {
            batches.push(b);
        }
        (batches, drops.total())
    }

    #[tokio::test]
    async fn a_saturated_source_never_yields_a_batch_over_the_cap() {
        let total = MAX_BATCH_LINES * 5 + 123;
        let out = batch_counted(
            vec![lines("a", total)],
            Duration::from_millis(2),
            CancellationToken::new(),
            total,
        );
        let (batches, dropped) = drain_counted(out).await;
        assert_eq!(dropped, 0);
        assert!(batches.iter().all(|b| b.len() <= MAX_BATCH_LINES));
        assert!(
            batches.len() >= 6,
            "{total} lines need at least 6 batches, got {}",
            batches.len()
        );
        assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), total);
    }

    #[tokio::test]
    async fn carried_over_lines_arrive_in_order_on_later_ticks() {
        let total = MAX_BATCH_LINES * 3;
        let out = batch_counted(
            vec![lines("a", total)],
            Duration::from_millis(2),
            CancellationToken::new(),
            total,
        );
        let (batches, _) = drain_counted(out).await;
        assert!(batches.len() >= 3);
        assert_eq!(batches[0].len(), MAX_BATCH_LINES, "first batch is full");
        let got: Vec<String> = batches.into_iter().flatten().map(|l| l.text).collect();
        let want: Vec<String> = (0..total).map(|n| format!("a {n}")).collect();
        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn queue_overflow_drops_the_oldest_and_counts_them() {
        let total = 50_000;
        let queue = 5_000;
        let out = batch_counted(
            vec![lines("a", total)],
            Duration::from_millis(16),
            CancellationToken::new(),
            queue,
        );
        let (batches, dropped) = drain_counted(out).await;
        let got: Vec<usize> = batches
            .into_iter()
            .flatten()
            .map(|l| l.text.trim_start_matches("a ").parse().expect("number"))
            .collect();
        assert!(dropped > 0, "nothing was dropped");
        assert_eq!(got.len() as u64 + dropped, total as u64);
        assert!(got.windows(2).all(|w| w[0] < w[1]), "order kept");
        assert_eq!(got.last(), Some(&(total - 1)), "the newest line survives");
        assert!(got.len() >= queue, "a full queue is delivered");
    }
}
