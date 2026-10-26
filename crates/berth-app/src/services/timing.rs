//! Start-up timing marks (P6-T3, PLAN §9 row 6). `mark("name")` logs one line per name,
//! `timing name=<name> ms=<since main> epoch_ms=<wall clock>`, the first time it is called.
//! `scripts/perf.sh --app` reads the lines from the log: `epoch_ms` minus the moment it
//! launched the app is the cold start, which includes exec and dynamic linking that `ms`
//! (counted from [`init`]) cannot see.
//!
//! Marks used by the app:
//!
//! - `main`: logger up, in `main` (milliseconds count from [`init`], the first line of `main`);
//! - `config`: the window configuration is built, before the event loop starts;
//! - `app`: the root component rendered for the first time (window and webview exist);
//! - `projects_snapshot`: the Projects table has rows from the stored snapshot (before
//!   discovery or `colima list`);
//! - `projects_live`: rows from a live engine answer;
//! - `projects_painted`: two animation frames after the first rows, i.e. on screen.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// The marks seen so far and the instant they are counted from.
#[derive(Debug)]
pub struct Marks {
    start: Instant,
    seen: HashSet<&'static str>,
}

impl Marks {
    #[must_use]
    pub fn new(start: Instant) -> Self {
        Self {
            start,
            seen: HashSet::new(),
        }
    }

    /// Milliseconds since the start the first time `name` is marked; `None` after that.
    pub fn mark(&mut self, name: &'static str, now: Instant) -> Option<u128> {
        if !self.seen.insert(name) {
            return None;
        }
        Some(now.saturating_duration_since(self.start).as_millis())
    }
}

fn global() -> &'static Mutex<Marks> {
    static MARKS: OnceLock<Mutex<Marks>> = OnceLock::new();
    MARKS.get_or_init(|| Mutex::new(Marks::new(Instant::now())))
}

/// Start the clock. Call it first in `main`, before the logger exists; a mark made without
/// it counts from the first mark.
pub fn init() {
    let _ = global();
}

/// Log `timing name=… ms=… epoch_ms=…` once per name. Cheap: a lock and a set insert.
pub fn mark(name: &'static str) {
    let ms = global()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .mark(name, Instant::now());
    if let Some(ms) = ms {
        let epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        tracing::info!("timing name={name} ms={ms} epoch_ms={epoch_ms}");
    }
}

/// Resolves after the webview has painted what was just rendered: two animation frames.
/// Returns at once when there is no webview to ask (SSR scenes, tests).
pub async fn painted() {
    let mut eval = dioxus::document::eval(
        "requestAnimationFrame(() => requestAnimationFrame(() => dioxus.send(1)));",
    );
    let _ = eval.recv::<i32>().await;
}

/// [`painted`], but never longer than two seconds: a window that is not drawing (hidden at
/// launch) runs no animation frames, and the work that waits for the first paint must still
/// start.
pub async fn after_first_paint() {
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), painted()).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_name_is_marked_once_with_the_time_since_start() {
        let start = Instant::now();
        let mut marks = Marks::new(start);
        assert_eq!(
            marks.mark("shell", start + Duration::from_millis(120)),
            Some(120)
        );
        assert_eq!(
            marks.mark("shell", start + Duration::from_millis(900)),
            None
        );
        assert_eq!(
            marks.mark("live", start + Duration::from_millis(450)),
            Some(450)
        );
    }

    #[test]
    fn a_mark_before_the_start_reads_zero() {
        let start = Instant::now();
        let mut marks = Marks::new(start + Duration::from_secs(1));
        assert_eq!(marks.mark("early", start), Some(0));
    }
}
