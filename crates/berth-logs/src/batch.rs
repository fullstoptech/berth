//! Many log streams in, one `Vec` per tick out.

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

/// Collects lines from many streams and yields one `Vec` per `tick` (16 ms in the app), never an
/// empty one. Ticks that find nothing send nothing. When every stream has ended the remaining lines
/// go out on the next tick and the receiver then closes; cancelling the token or dropping the
/// receiver stops the task at once.
///
/// Must be called inside a tokio runtime.
#[must_use]
pub fn batch(
    streams: Vec<BoxStream<'static, LogLine>>,
    tick: Duration,
    cancel: CancellationToken,
) -> mpsc::Receiver<Vec<LogLine>> {
    let (tx, rx) = mpsc::channel(CHANNEL_DEPTH);
    tokio::spawn(async move {
        let mut merged = select_all(streams);
        let mut ticker = interval(tick.max(Duration::from_millis(1)));
        // A late tick is not made up for: two batches never sit closer than one tick.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut pending: Vec<LogLine> = Vec::new();
        let mut ended = false;
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                () = tx.closed() => break,
                _ = ticker.tick() => {
                    if !pending.is_empty() && tx.send(std::mem::take(&mut pending)).await.is_err() {
                        break;
                    }
                    if ended && pending.is_empty() {
                        break;
                    }
                }
                next = merged.next(), if !ended => match next {
                    Some(line) => {
                        pending.push(line);
                        // Take what is already waiting without going back through the select.
                        for _ in 1..DRAIN_STEP {
                            let Some(more) = merged.next().now_or_never() else { break };
                            match more {
                                Some(line) => pending.push(line),
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
    rx
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
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
        for pair in stamps.windows(2) {
            assert!(
                pair[1] - pair[0] >= Duration::from_millis(18),
                "batches closer than a tick: {stamps:?}"
            );
        }
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
}
