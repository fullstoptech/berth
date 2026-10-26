//! The one visibility signal (AD32, P6-T3). "Visible" means the native window is shown and
//! not minimised; a window merely covered by another app is still visible, because the
//! webview's own `visibilitychange` fires for a covered window (PLAN D26, amended).
//!
//! Every timer that only matters for what is on screen takes a [`Pacer`] instead of
//! sleeping: it ticks every period while the window is visible, parks while it is not, and
//! ticks at once when the window comes back, so a closed window costs no wake-ups.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

/// The window's visible flag, shared by every service and screen timer.
#[derive(Debug, Clone)]
pub struct Visibility {
    tx: Arc<watch::Sender<bool>>,
}

impl Default for Visibility {
    fn default() -> Self {
        Self::new(true)
    }
}

impl Visibility {
    #[must_use]
    pub fn new(visible: bool) -> Self {
        Self {
            tx: Arc::new(watch::channel(visible).0),
        }
    }

    /// Record what the window reports; waiters wake only when the value changes.
    pub fn set(&self, visible: bool) {
        self.tx.send_if_modified(|current| {
            let changed = *current != visible;
            *current = visible;
            changed
        });
    }

    #[must_use]
    pub fn is_visible(&self) -> bool {
        *self.tx.borrow()
    }

    /// A timer that ticks every `period` while the window is visible.
    #[must_use]
    pub fn pacer(&self, period: Duration) -> Pacer {
        Pacer {
            rx: Some(self.tx.subscribe()),
            period,
        }
    }
}

/// A repeating timer gated on [`Visibility`].
#[derive(Debug)]
pub struct Pacer {
    /// `None` is a pacer with no visibility signal behind it (SSR scenes, tests): it never
    /// parks.
    rx: Option<watch::Receiver<bool>>,
    period: Duration,
}

impl Pacer {
    /// A plain timer, for code that runs where no window exists.
    #[must_use]
    pub fn always(period: Duration) -> Self {
        Self { rx: None, period }
    }

    /// `visibility`'s pacer, or a plain one when the app has no visibility signal.
    #[must_use]
    pub fn of(visibility: Option<&Visibility>, period: Duration) -> Self {
        visibility.map_or_else(|| Self::always(period), |v| v.pacer(period))
    }

    /// Wait for the next tick: one period while visible. While hidden this waits without a
    /// timer until the window shows, then returns at once (the screen is stale). A window
    /// that hides during the wait parks it too. Returns `false` when the signal is gone.
    pub async fn tick(&mut self) -> bool {
        let Some(rx) = self.rx.as_mut() else {
            tokio::time::sleep(self.period).await;
            return true;
        };
        loop {
            if !*rx.borrow_and_update() {
                return rx.wait_for(|visible| *visible).await.is_ok();
            }
            tokio::select! {
                () = tokio::time::sleep(self.period) => return true,
                changed = rx.changed() => {
                    if changed.is_err() {
                        return false;
                    }
                    if *rx.borrow_and_update() {
                        return true;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const PERIOD: Duration = Duration::from_millis(20);

    /// Count ticks for `span`.
    async fn count_ticks(pacer: &mut Pacer, span: Duration) -> usize {
        let deadline = Instant::now() + span;
        let mut ticks = 0;
        while Instant::now() < deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(left, pacer.tick()).await {
                Ok(true) => ticks += 1,
                _ => break,
            }
        }
        ticks
    }

    #[tokio::test]
    async fn ticks_while_visible() {
        let visibility = Visibility::new(true);
        let mut pacer = visibility.pacer(PERIOD);
        assert!(count_ticks(&mut pacer, Duration::from_millis(210)).await >= 5);
    }

    #[tokio::test]
    async fn a_hidden_window_gets_no_ticks() {
        let visibility = Visibility::new(true);
        let mut pacer = visibility.pacer(PERIOD);
        visibility.set(false);
        assert_eq!(count_ticks(&mut pacer, Duration::from_millis(210)).await, 0);
    }

    #[tokio::test]
    async fn a_window_that_hides_mid_wait_parks_the_timer() {
        let visibility = Visibility::new(true);
        let mut pacer = visibility.pacer(Duration::from_millis(150));
        let hide = visibility.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            hide.set(false);
        });
        assert_eq!(count_ticks(&mut pacer, Duration::from_millis(400)).await, 0);
    }

    #[tokio::test]
    async fn showing_the_window_ticks_at_once_then_resumes() {
        let visibility = Visibility::new(false);
        let mut pacer = visibility.pacer(Duration::from_secs(30));
        let show = visibility.clone();
        let shown_at = Instant::now();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            show.set(true);
        });
        // The 30 s period is not waited out: the show itself is the tick.
        assert!(pacer.tick().await);
        assert!(shown_at.elapsed() < Duration::from_secs(5));
        assert!(visibility.is_visible());
    }

    #[tokio::test]
    async fn a_repeated_value_does_not_wake_a_waiter() {
        let visibility = Visibility::new(true);
        let mut pacer = visibility.pacer(Duration::from_millis(100));
        visibility.set(true);
        visibility.set(true);
        let start = Instant::now();
        assert!(pacer.tick().await);
        // Still a full period: setting the same value was not a tick.
        assert!(start.elapsed() >= Duration::from_millis(90));
    }

    #[tokio::test]
    async fn a_pacer_without_a_signal_never_parks() {
        let mut pacer = Pacer::of(None, PERIOD);
        assert!(count_ticks(&mut pacer, Duration::from_millis(210)).await >= 5);
    }
}
