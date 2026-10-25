//! Log sessions (PLAN §6.6, §7.6, D23, D25). One [`LogCore`] per open log view: a task that
//! builds the [`LogView`], feeds it from `berth_logs::batch_counted` over the engines' `logs`
//! streams and applies the UI's commands in order. The UI never touches an engine and never runs
//! more than a window of rows on its own thread:
//!
//! - the view is built, warmed and filled in the session task (the first allocation and the
//!   lazily compiled patterns cost a few milliseconds the UI must not pay);
//! - the UI reads a screenful of rows under a short lock ([`LogHandle::read`]);
//! - anything that walks the whole ring (filter, search, Copy, Save) runs as a command in the
//!   session task ([`LogHandle::run`]) and the UI awaits the answer;
//! - after each applied batch (or command) the task wakes the UI once; [`LogHandle`] turns the
//!   wake into one bump of a version signal, so a burst of lines costs one re-render.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use berth_core::{ContainerId, EngineId, LogLine, LogQuery};
use berth_logs::{
    batch_counted, series_for, LogView, PreparedBatch, SearchSummary, DEFAULT_QUEUE_LINES,
    MAX_BATCH_LINES,
};
use dioxus::prelude::*;
use futures::stream::BoxStream;
use futures::StreamExt;
use tokio::sync::{mpsc, oneshot, Notify};
use tokio_util::sync::CancellationToken;

use super::Services;

/// Lines one view keeps (D25).
pub const RING_LINES: usize = 100_000;
/// One batch per tick at most (PLAN §1.3).
pub const TICK: Duration = Duration::from_millis(16);
/// Lines asked of each container before following.
pub const INITIAL_TAIL: u32 = 2_000;
/// With several containers, lines are held this long so the first lines of every stream can be
/// merged in time order before any is shown.
pub const MERGE_WINDOW: Duration = Duration::from_millis(250);

/// One container a view follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogTarget {
    pub engine: EngineId,
    pub id: ContainerId,
    pub name: String,
}

/// Where a view stands, for the one line of text under the toolbar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogStatus {
    /// Streams are being opened; no line has arrived.
    Connecting,
    Following,
    /// Every stream ended (the containers stopped, or the engines went away).
    Ended,
    /// A stream failed or an engine is not connected.
    Failed(String),
}

type Command = Box<dyn FnOnce(&mut LogView) + Send>;
/// Makes the streams of a session. It runs in the session task and may mark the session failed
/// through the weak reference.
type Opener =
    Box<dyn FnOnce(CancellationToken, Weak<LogCore>) -> Vec<BoxStream<'static, LogLine>> + Send>;

/// The series hue each container of a view gets: the `LogView` gives the n-th id (in id order)
/// `series_for(n)`, so the picker chips colour themselves the same way before any line arrives.
#[must_use]
pub fn series_of(ids: &[ContainerId]) -> BTreeMap<ContainerId, u8> {
    ids.iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(n, id)| (id.clone(), series_for(n)))
        .collect()
}

/// What the session task shares with the UI.
pub struct LogCore {
    /// `None` until the task has built it.
    view: Mutex<Option<LogView>>,
    commands: mpsc::UnboundedSender<Command>,
    wake: Notify,
    /// Rows appended since Follow went off: the "142 new lines" pill.
    new_lines: AtomicU64,
    /// Batches applied so far.
    batches: AtomicU64,
    /// Longest hold of the view lock by one batch commit, in microseconds (measurement, logged at
    /// debug).
    slowest_push_us: AtomicU64,
    status: Mutex<LogStatus>,
    search_error: Mutex<Option<String>>,
    /// The newest search asked for; an older one that reaches the task later does nothing.
    search_ticket: AtomicU64,
    cancel: CancellationToken,
    id: u64,
}

impl Drop for LogCore {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

static NEXT_CORE_ID: AtomicU64 = AtomicU64::new(1);

fn spawn_io(future: impl Future<Output = ()> + Send + 'static) {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(future);
    } else {
        spawn(future);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// A throwaway view that touches every lazily built pattern and the first memory pages, so the
/// first real batch costs what a later one does (`berth-logs/tests/cold_start.rs`).
fn warm_up() {
    let mut throwaway = LogView::new(1_000, BTreeMap::new());
    let id = ContainerId("warm-up".to_string());
    let line = |stream, text: &str| LogLine {
        container: id.clone(),
        ts: chrono::DateTime::UNIX_EPOCH,
        stream,
        text: text.to_string(),
    };
    let lines = [
        "INFO started",
        "WARN cache is nearly full",
        "{\"level\":\"info\",\"msg\":\"request ok\",\"ms\":12}",
        "level=info msg=\"GET /health\" status=200",
        "ERROR simulated failure",
        "Traceback (most recent call last):",
        "  File \"/app/worker.py\", line 41, in step_1",
        "RuntimeError: simulated failure",
        "Error: connect ECONNREFUSED 127.0.0.1:5432",
        "    at Object.connect (node:net:1139:12)",
        "panic: runtime error",
        "goroutine 1 [running]:",
    ]
    .iter()
    .enumerate()
    .map(|(i, text)| {
        line(
            if i % 2 == 0 {
                berth_core::Stream::Stdout
            } else {
                berth_core::Stream::Stderr
            },
            text,
        )
    })
    .collect();
    throwaway.push_batch(lines);
}

impl LogCore {
    fn new(cancel: CancellationToken) -> (Arc<Self>, mpsc::UnboundedReceiver<Command>) {
        let (commands, receiver) = mpsc::unbounded_channel();
        let core = Arc::new(Self {
            view: Mutex::new(None),
            commands,
            wake: Notify::new(),
            new_lines: AtomicU64::new(0),
            batches: AtomicU64::new(0),
            slowest_push_us: AtomicU64::new(0),
            status: Mutex::new(LogStatus::Connecting),
            search_error: Mutex::new(None),
            search_ticket: AtomicU64::new(0),
            cancel,
            id: NEXT_CORE_ID.fetch_add(1, Ordering::Relaxed),
        });
        (core, receiver)
    }

    /// A session over streams made by `open` (called inside the session task, so a stream that
    /// starts its own pump finds a tokio runtime). The view is built there too.
    pub fn start(
        names: BTreeMap<ContainerId, String>,
        open: impl FnOnce(CancellationToken, Weak<LogCore>) -> Vec<BoxStream<'static, LogLine>>
            + Send
            + 'static,
        cancel: &CancellationToken,
    ) -> Arc<Self> {
        Self::start_merging(names, open, cancel, MERGE_WINDOW)
    }

    /// [`start`](Self::start) with the merge window chosen (tests).
    pub fn start_merging(
        names: BTreeMap<ContainerId, String>,
        open: impl FnOnce(CancellationToken, Weak<LogCore>) -> Vec<BoxStream<'static, LogLine>>
            + Send
            + 'static,
        cancel: &CancellationToken,
        merge_window: Duration,
    ) -> Arc<Self> {
        let cancel = cancel.child_token();
        let (core, commands) = Self::new(cancel.clone());
        let task_core = Arc::clone(&core);
        spawn_io(session(
            task_core,
            names,
            Box::new(open),
            merge_window,
            cancel,
            commands,
        ));
        core
    }

    /// A session over a view that is already filled and has no stream (dev scenes, tests). It
    /// still answers commands.
    pub fn seeded(view: LogView) -> Arc<Self> {
        let cancel = CancellationToken::new();
        let (core, commands) = Self::new(cancel.clone());
        *lock(&core.view) = Some(view);
        *lock(&core.status) = LogStatus::Following;
        let task_core = Arc::clone(&core);
        spawn_io(commands_only(task_core, cancel, commands));
        core
    }

    /// Stops the session task and its streams.
    pub fn close(&self) {
        self.cancel.cancel();
    }

    /// Unique for the life of the process; names the scroll element of the view.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Reads the view, or `None` while the task is still building it. Keep `f` to a window of
    /// rows or a counter: it holds the lock the session task needs.
    pub fn with_view<R>(&self, f: impl FnOnce(&LogView) -> R) -> Option<R> {
        lock(&self.view).as_ref().map(f)
    }

    /// Runs `f` on the view inside the session task and returns what it answered; `None` when
    /// the session is closed. The UI awaits this instead of walking the ring itself.
    pub fn run<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut LogView) -> R + Send + 'static,
    ) -> impl Future<Output = Option<R>> {
        let (reply, answer) = oneshot::channel();
        let sent = self
            .commands
            .send(Box::new(move |view: &mut LogView| {
                let _ = reply.send(f(view));
            }))
            .is_ok();
        async move {
            if sent {
                answer.await.ok()
            } else {
                None
            }
        }
    }

    /// Rows appended while Follow was off.
    #[must_use]
    pub fn new_lines(&self) -> u64 {
        self.new_lines.load(Ordering::Relaxed)
    }

    /// Sets the pill's count (dev scenes and tests).
    pub fn seed_new_lines(&self, count: u64) {
        self.new_lines.store(count, Ordering::Relaxed);
    }

    /// Follow on or off. Turning it on forgets the pill's count.
    pub fn set_follow(&self, on: bool) -> impl Future<Output = Option<()>> {
        if on {
            self.new_lines.store(0, Ordering::Relaxed);
        }
        self.run(move |view| view.set_follow(on))
    }

    #[must_use]
    pub fn status(&self) -> LogStatus {
        lock(&self.status).clone()
    }

    /// The regex error of the current search, for the inline message under the field.
    #[must_use]
    pub fn search_error(&self) -> Option<String> {
        lock(&self.search_error).clone()
    }

    /// Starts, replaces or ends the search; a bad pattern lands in [`search_error`](Self::search_error).
    pub fn set_search(
        self: &Arc<Self>,
        query: Option<berth_logs::Query>,
    ) -> impl Future<Output = Option<SearchSummary>> {
        let core = Arc::clone(self);
        let ticket = self.search_ticket.fetch_add(1, Ordering::SeqCst) + 1;
        self.run(move |view| {
            if core.search_ticket.load(Ordering::SeqCst) != ticket {
                // Typed over before it ran: the newer search does the work.
                return view.search_summary();
            }
            match view.set_search(query) {
                Ok(summary) => {
                    *lock(&core.search_error) = None;
                    summary
                }
                Err(err) => {
                    *lock(&core.search_error) = Some(err.to_string());
                    SearchSummary::default()
                }
            }
        })
    }

    /// Batches applied so far.
    #[must_use]
    pub fn batches(&self) -> u64 {
        self.batches.load(Ordering::Relaxed)
    }

    /// The longest the view lock was held by one batch commit so far.
    #[must_use]
    pub fn slowest_push(&self) -> Duration {
        Duration::from_micros(self.slowest_push_us.load(Ordering::Relaxed))
    }

    /// Resolves when the session task has changed something the UI shows. Coalesces: wakes
    /// that nobody awaited collapse into one.
    pub async fn changed(&self) {
        self.wake.notified().await;
    }

    fn set_status(&self, status: LogStatus) {
        let mut guard = lock(&self.status);
        // A failure is not undone by the streams that still run.
        if matches!(*guard, LogStatus::Failed(_)) && !matches!(status, LogStatus::Failed(_)) {
            return;
        }
        *guard = status;
    }
}

/// Runs one of the UI's commands on the view and wakes the UI.
fn run_command(core: &LogCore, command: Command) {
    if let Some(view) = lock(&core.view).as_mut() {
        command(view);
    }
    core.wake.notify_one();
}

/// The task of a seeded session: no streams, only commands.
async fn commands_only(
    core: Arc<LogCore>,
    cancel: CancellationToken,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            Some(command) = commands.recv() => run_command(&core, command),
        }
    }
}

/// Lines of one batch in time order; ties keep arrival order (the sort is stable).
fn sort_by_time(lines: &mut [LogLine]) {
    lines.sort_by_key(|line| line.ts);
}

async fn session(
    core: Arc<LogCore>,
    names: BTreeMap<ContainerId, String>,
    open: Opener,
    merge_window: Duration,
    cancel: CancellationToken,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    // Built here, not on the UI path: the first allocation and the lazily compiled patterns.
    warm_up();
    *lock(&core.view) = Some(LogView::new(RING_LINES, names));
    core.wake.notify_one();
    let streams = open(cancel.clone(), Arc::downgrade(&core));
    let merge = streams.len() > 1;
    let (mut batches, drops) = batch_counted(streams, TICK, cancel.clone(), DEFAULT_QUEUE_LINES);
    let mut counted_drops = 0_u64;
    let mut held: Vec<LogLine> = Vec::new();
    let merge_until = tokio::time::Instant::now() + merge_window;
    let mut merging = merge;
    let mut open_batches = true;
    loop {
        // While the merge window is open the next wake-up is its end.
        let window = async {
            if merging {
                tokio::time::sleep_until(merge_until).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            Some(command) = commands.recv() => run_command(&core, command),
            () = window => {
                merging = false;
                let mut lines = std::mem::take(&mut held);
                sort_by_time(&mut lines);
                apply(&core, &drops, &mut counted_drops, lines);
            }
            received = batches.recv(), if open_batches => match received {
                Some(lines) if merging => held.extend(lines),
                Some(mut lines) => {
                    if merge {
                        sort_by_time(&mut lines);
                    }
                    apply(&core, &drops, &mut counted_drops, lines);
                }
                None => {
                    open_batches = false;
                    if merging {
                        merging = false;
                        let mut lines = std::mem::take(&mut held);
                        sort_by_time(&mut lines);
                        apply(&core, &drops, &mut counted_drops, lines);
                    }
                    core.set_status(LogStatus::Ended);
                    core.wake.notify_one();
                }
            },
        }
    }
}

/// Pushes `lines` into the view in chunks the UI path could also take (at most
/// [`MAX_BATCH_LINES`]), then wakes the UI once.
fn apply(
    core: &LogCore,
    drops: &berth_logs::BatchDrops,
    counted_drops: &mut u64,
    lines: Vec<LogLine>,
) {
    if lines.is_empty() {
        return;
    }
    let mut rest = lines;
    while !rest.is_empty() {
        let tail = rest.split_off(rest.len().min(MAX_BATCH_LINES));
        let chunk = std::mem::replace(&mut rest, tail);
        // Classification, JSON parsing and fold facts run before the lock is taken (AD23): the UI
        // reads the view while this chunk is prepared, and waits only for the commit.
        let prepare_started = Instant::now();
        let prepared = PreparedBatch::new(chunk);
        let prepare_us = micros(prepare_started.elapsed());
        // Only error-marked lines feed the problem rules (D35, D82); outside the lock too.
        crate::services::problems::tap_errors(prepared.error_lines());
        let mut guard = lock(&core.view);
        let Some(view) = guard.as_mut() else {
            return;
        };
        let commit_started = Instant::now();
        let delta = view.commit_batch(prepared);
        let commit_us = micros(commit_started.elapsed());
        drop(guard);
        core.new_lines
            .fetch_add(u64::from(delta.new_while_paused), Ordering::Relaxed);
        core.batches.fetch_add(1, Ordering::Relaxed);
        core.slowest_push_us.fetch_max(commit_us, Ordering::Relaxed);
        tracing::debug!(
            prepare_us,
            commit_us,
            appended = delta.appended,
            "log batch applied"
        );
    }
    let total = drops.total();
    if total > *counted_drops {
        if let Some(view) = lock(&core.view).as_mut() {
            view.add_dropped(total - *counted_drops);
        }
        *counted_drops = total;
    }
    core.set_status(LogStatus::Following);
    core.wake.notify_one();
}

// ---------------------------------------------------------------------
// Opening a live session
// ---------------------------------------------------------------------

/// What a view asks of the engines.
#[must_use]
pub fn log_query() -> LogQuery {
    LogQuery {
        since: None,
        tail: Some(INITIAL_TAIL),
        follow: true,
    }
}

/// Opens a session over the engines' `logs` streams. A target whose engine is not connected
/// marks the session [`LogStatus::Failed`] but the others still run.
pub fn open_session(services: &Services, targets: &[LogTarget]) -> Arc<LogCore> {
    let names = targets
        .iter()
        .map(|t| (t.id.clone(), t.name.clone()))
        .collect();
    let clients = services.clients.clone();
    let targets = targets.to_vec();
    LogCore::start(
        names,
        move |cancel, weak| {
            let mut streams: Vec<BoxStream<'static, LogLine>> = Vec::new();
            for target in &targets {
                let client = match clients.lock() {
                    Ok(map) => map.get(&target.engine).cloned(),
                    Err(poisoned) => poisoned.into_inner().get(&target.engine).cloned(),
                };
                let Some(client) = client else {
                    if let Some(core) = weak.upgrade() {
                        core.set_status(LogStatus::Failed(format!(
                            "{} is not connected",
                            target.engine.0
                        )));
                    }
                    continue;
                };
                let (name, failed) = (target.name.clone(), weak.clone());
                let stream = client
                    .logs(&target.id, log_query(), cancel.child_token())
                    .inspect(move |item| {
                        if let (Err(err), Some(core)) = (item, failed.upgrade()) {
                            tracing::debug!(%err, container = %name, "log stream failed");
                            core.set_status(LogStatus::Failed(format!(
                                "Lost the log stream of {name}"
                            )));
                        }
                    })
                    .take_while(|item| std::future::ready(item.is_ok()))
                    .filter_map(|item| std::future::ready(item.ok()))
                    .boxed();
                streams.push(stream);
            }
            streams
        },
        &services.cancel,
    )
}

// ---------------------------------------------------------------------
// The UI's handle
// ---------------------------------------------------------------------

/// What a scene or test provides so a view shows a ready session instead of opening streams.
#[derive(Clone)]
pub struct SeededLogs(pub Arc<LogCore>);

/// A [`LogCore`] plus the version signal that re-renders whoever reads it.
#[derive(Clone)]
pub struct LogHandle {
    pub core: Arc<LogCore>,
    version: Signal<u64>,
}

impl PartialEq for LogHandle {
    fn eq(&self, other: &Self) -> bool {
        self.core.id == other.core.id
    }
}

impl LogHandle {
    /// Reads the view and subscribes the calling component to every change.
    pub fn read<R>(&self, f: impl FnOnce(&LogView) -> R) -> Option<R> {
        let _ = self.version.read();
        self.core.with_view(f)
    }

    /// Subscribes the calling component to every change.
    pub fn track(&self) {
        let _ = self.version.read();
    }
}

/// The session for a view, opened once per mounted component (remount to reopen) and closed when
/// the component goes. Under a scene, the seeded session is used instead and left open.
pub fn use_log_session(targets: Vec<LogTarget>) -> LogHandle {
    let seeded = try_consume_context::<SeededLogs>();
    let services = try_consume_context::<Services>();
    let was_seeded = seeded.is_some();
    let core = use_hook(move || match (seeded, services) {
        (Some(seeded), _) => seeded.0,
        (None, Some(services)) => open_session(&services, &targets),
        (None, None) => {
            // No engines at all (a bare render): an empty, ended session.
            let core = LogCore::start(
                BTreeMap::new(),
                |_, _| Vec::new(),
                &CancellationToken::new(),
            );
            core.set_status(LogStatus::Ended);
            core
        }
    });
    let mut version = use_signal(|| 0_u64);
    let waker = Arc::clone(&core);
    use_hook(move || {
        spawn(async move {
            loop {
                waker.changed().await;
                version += 1;
            }
        })
    });
    let closer = Arc::clone(&core);
    use_drop(move || {
        if !was_seeded {
            closer.close();
        }
    });
    LogHandle { core, version }
}

// ---------------------------------------------------------------------
// Save
// ---------------------------------------------------------------------

/// Writes `text` (lines joined by `\n`) to `path` with a closing newline and returns the number
/// of lines written, which equals `text.lines().count()`. The file is written beside its final
/// name and renamed, so a failed save never leaves half a log under the chosen name.
///
/// # Errors
/// Any I/O error from creating, writing or renaming the file.
pub async fn write_lines(path: &std::path::Path, text: &str) -> std::io::Result<usize> {
    let lines = text.lines().count();
    let mut body = String::with_capacity(text.len() + 1);
    body.push_str(text);
    if !text.is_empty() {
        body.push('\n');
    }
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let name = path
        .file_name()
        .map_or_else(|| "log".into(), |n| n.to_string_lossy().into_owned());
    let temp = dir.join(format!(".{name}.berth-save"));
    tokio::fs::write(&temp, body.as_bytes()).await?;
    if let Err(err) = tokio::fs::rename(&temp, path).await {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(err);
    }
    Ok(lines)
}

/// The suggested file name for a Save: `<view name>-<UTC time>.log`.
#[must_use]
pub fn save_file_name(view_name: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let clean: String = view_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let clean = clean.trim_matches('-');
    let clean = if clean.is_empty() { "logs" } else { clean };
    format!("{clean}-{}.log", now.format("%Y%m%d-%H%M%S"))
}

#[cfg(test)]
#[path = "logs_tests.rs"]
mod tests;
