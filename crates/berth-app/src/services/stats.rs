//! Live stats (PLAN D26, D23). One `stats` stream per container row that is
//! rendered in the table's visible range (plus overscan) and per open Stats
//! panel, closed on scroll-out and while the window is hidden. Missing stats
//! read as `None` ("—" in the UI), never as zero.
//!
//! The service keeps three things:
//!
//! - a [`Registry`] of what the UI currently shows, one slot per owning
//!   component (a row or a panel), set from render and cleared on drop;
//! - the open streams, reconciled against the registry by [`StatsService::reconcile`]
//!   (every [`TICK`], and at once when the window is hidden or shown);
//! - a [`History`] per container, a ring buffer holding the last five
//!   minutes. A sample is appended by the stream's own task (O(1)); the UI
//!   thread only learns that something changed through one version signal
//!   per tick, so a burst of samples costs one re-render.
//!
//! Open streams never exceed [`MAX_SUBSCRIPTIONS`]: panels win, then the
//! rows that became visible most recently.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use berth_core::{ContainerId, EngineId, LiveStats};
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use futures::StreamExt;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::engines::ClientMap;

/// Most streams open at once. The Containers table renders at most about
/// 60 rows (1100 px of rows plus 10 rows of overscan each way), so this
/// only bites when a screen asks for more.
pub const MAX_SUBSCRIPTIONS: usize = 60;
/// How much history a container keeps (the sparklines' span).
pub const HISTORY_WINDOW_S: i64 = 300;
/// Hard bound on points per container, in case a stream is faster than 1 Hz.
pub const HISTORY_MAX_POINTS: usize = 600;
/// A latest value older than this is not shown: the row reads "—".
pub const FRESH_FOR_S: i64 = 5;
/// How often the open streams are reconciled with what is on screen.
pub const TICK: Duration = Duration::from_millis(500);
/// A stream that ended or failed is not reopened for this long.
pub const RETRY_AFTER: Duration = Duration::from_secs(3);

/// A container on one engine; the key of a stream and of a history.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatsKey {
    pub engine: EngineId,
    pub id: ContainerId,
}

impl StatsKey {
    pub fn new(engine: &EngineId, id: &ContainerId) -> Self {
        Self {
            engine: engine.clone(),
            id: id.clone(),
        }
    }
}

// ---------------------------------------------------------------------
// History
// ---------------------------------------------------------------------

/// One sample with the moment it arrived.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub at: DateTime<Utc>,
    pub stats: LiveStats,
}

/// The last five minutes of one container, oldest first.
#[derive(Debug, Default, Clone)]
pub struct History {
    points: VecDeque<Point>,
}

impl History {
    /// Append a sample. Amortised O(1): the front is trimmed only by what
    /// this push pushed out of the window.
    pub fn push(&mut self, at: DateTime<Utc>, stats: LiveStats) {
        self.points.push_back(Point { at, stats });
        let cutoff = at - chrono::Duration::seconds(HISTORY_WINDOW_S);
        while self.points.front().is_some_and(|p| p.at < cutoff) {
            self.points.pop_front();
        }
        while self.points.len() > HISTORY_MAX_POINTS {
            self.points.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn points(&self) -> impl Iterator<Item = &Point> {
        self.points.iter()
    }

    pub fn latest(&self) -> Option<&Point> {
        self.points.back()
    }

    /// The newest sample if it is recent enough to show.
    pub fn fresh(&self, now: DateTime<Utc>) -> Option<&LiveStats> {
        self.points
            .back()
            .filter(|p| now.signed_duration_since(p.at).num_seconds() <= FRESH_FOR_S)
            .map(|p| &p.stats)
    }

    /// One value per sample.
    pub fn series(&self, value: impl Fn(&LiveStats) -> f64) -> Vec<(DateTime<Utc>, f64)> {
        self.points
            .iter()
            .map(|p| (p.at, value(&p.stats)))
            .collect()
    }

    /// Bytes per second between consecutive samples of a cumulative
    /// counter. A counter that went down (container restarted) yields no
    /// point for that step.
    #[allow(clippy::cast_precision_loss)]
    pub fn rates(&self, counter: impl Fn(&LiveStats) -> u64) -> Vec<(DateTime<Utc>, f64)> {
        let mut out = Vec::with_capacity(self.points.len().saturating_sub(1));
        let mut prev: Option<(&Point, u64)> = None;
        for point in &self.points {
            let now = counter(&point.stats);
            if let Some((before, was)) = prev {
                let millis = point.at.signed_duration_since(before.at).num_milliseconds();
                if millis > 0 && now >= was {
                    out.push((point.at, (now - was) as f64 * 1000.0 / millis as f64));
                }
            }
            prev = Some((point, now));
        }
        out
    }
}

// ---------------------------------------------------------------------
// Registry: what the UI wants
// ---------------------------------------------------------------------

/// One component that shows stats (a row, a panel); minted by
/// [`StatsService::new_owner`].
pub type OwnerId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Want {
    Row(StatsKey),
    Panel(StatsKey),
}

#[derive(Debug)]
struct Entry {
    want: Want,
    /// Order in which the slot last changed; newer rows win a full house.
    seq: u64,
}

#[derive(Debug)]
struct Registry {
    owners: HashMap<OwnerId, Entry>,
    next_seq: u64,
    window_visible: bool,
}

impl Registry {
    fn new() -> Self {
        Self {
            owners: HashMap::new(),
            next_seq: 0,
            window_visible: true,
        }
    }

    fn set(&mut self, owner: OwnerId, want: Option<Want>) {
        match want {
            None => {
                self.owners.remove(&owner);
            }
            Some(want) => {
                if self.owners.get(&owner).is_some_and(|e| e.want == want) {
                    return;
                }
                self.next_seq += 1;
                self.owners.insert(
                    owner,
                    Entry {
                        want,
                        seq: self.next_seq,
                    },
                );
            }
        }
    }

    /// The streams that should be open, most important first: nothing
    /// while the window is hidden; else panels, then rows by recency;
    /// one stream per container; at most [`MAX_SUBSCRIPTIONS`].
    fn target(&self) -> Vec<StatsKey> {
        if !self.window_visible {
            return Vec::new();
        }
        let mut entries: Vec<&Entry> = self.owners.values().collect();
        entries.sort_by_key(|e| (matches!(e.want, Want::Row(_)), std::cmp::Reverse(e.seq)));
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for entry in entries {
            let (Want::Row(key) | Want::Panel(key)) = &entry.want;
            if seen.insert(key.clone()) {
                out.push(key.clone());
                if out.len() == MAX_SUBSCRIPTIONS {
                    break;
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------

/// Opens one container's stats stream, or `None` when its engine is not
/// connected. Called inside the stream's own task, so a bollard stream
/// that spawns its pump at call time finds a tokio runtime.
pub type Opener =
    Arc<dyn Fn(&StatsKey, CancellationToken) -> Option<berth_engine::Bs<LiveStats>> + Send + Sync>;

struct Open {
    generation: u64,
    cancel: CancellationToken,
}

struct State {
    registry: Registry,
    open: HashMap<StatsKey, Open>,
    cooldown: HashMap<StatsKey, Instant>,
    histories: HashMap<StatsKey, History>,
    next_generation: u64,
}

struct Inner {
    state: Mutex<State>,
    opener: Opener,
    root: CancellationToken,
    next_owner: AtomicU64,
    /// Set by a stream task when a sample landed or a stream ended; taken
    /// by the tick, which then bumps the UI's version signal once.
    changed: AtomicBool,
    wake_ui: tokio::sync::Notify,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, key: &StatsKey, at: DateTime<Utc>, stats: LiveStats) {
        self.lock()
            .histories
            .entry(key.clone())
            .or_default()
            .push(at, stats);
        self.changed.store(true, Ordering::Release);
    }

    /// A stream finished by itself. Only the current generation counts, so
    /// the end of a closed stream cannot unregister its replacement.
    fn ended(&self, key: &StatsKey, generation: u64) {
        let mut state = self.lock();
        if state
            .open
            .get(key)
            .is_some_and(|o| o.generation == generation)
        {
            state.open.remove(key);
            state
                .cooldown
                .insert(key.clone(), Instant::now() + RETRY_AFTER);
            drop(state);
            self.changed.store(true, Ordering::Release);
        }
    }

    fn reconcile(self: &Arc<Self>) -> Reconciled {
        let now = Instant::now();
        let mut report = Reconciled::default();
        let mut starts = Vec::new();
        {
            let mut state = self.lock();
            let target = state.registry.target();
            let wanted: BTreeSet<&StatsKey> = target.iter().collect();
            let stale: Vec<StatsKey> = state
                .open
                .keys()
                .filter(|k| !wanted.contains(k))
                .cloned()
                .collect();
            for key in stale {
                if let Some(open) = state.open.remove(&key) {
                    open.cancel.cancel();
                }
                report.closed.push(key);
            }
            state.cooldown.retain(|_, until| *until > now);
            for key in target {
                if state.open.contains_key(&key) || state.cooldown.contains_key(&key) {
                    continue;
                }
                state.next_generation += 1;
                let generation = state.next_generation;
                let cancel = self.root.child_token();
                state.open.insert(
                    key.clone(),
                    Open {
                        generation,
                        cancel: cancel.clone(),
                    },
                );
                starts.push((key.clone(), generation, cancel));
                report.opened.push(key);
            }
        }
        for (key, generation, cancel) in starts {
            let inner = Arc::clone(self);
            spawn_io(pump(inner, key, generation, cancel));
        }
        report
    }

    /// Drop histories nothing has fed for a full window.
    fn prune(&self, now: DateTime<Utc>) {
        let cutoff = now - chrono::Duration::seconds(HISTORY_WINDOW_S);
        let mut state = self.lock();
        let State {
            histories, open, ..
        } = &mut *state;
        histories
            .retain(|key, h| open.contains_key(key) || h.latest().is_some_and(|p| p.at >= cutoff));
    }
}

/// What one reconcile pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reconciled {
    pub opened: Vec<StatsKey>,
    pub closed: Vec<StatsKey>,
}

/// The stats service. Cheap to clone; provided as context from the App
/// root next to `ActionsService`.
#[derive(Clone)]
pub struct StatsService {
    inner: Arc<Inner>,
    /// Bumped (once per tick with news) so the components reading stats
    /// re-render. `None` for a detached service (tests, SSR scenes).
    version: Option<Signal<u64>>,
}

fn spawn_io(future: impl std::future::Future<Output = ()> + Send + 'static) {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(future);
    } else {
        spawn(future);
    }
}

impl StatsService {
    /// A service over a custom opener, with no UI wiring and no tick:
    /// tests call [`reconcile`](Self::reconcile) themselves.
    pub fn with_opener(opener: Opener, cancel: CancellationToken) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    registry: Registry::new(),
                    open: HashMap::new(),
                    cooldown: HashMap::new(),
                    histories: HashMap::new(),
                    next_generation: 0,
                }),
                opener,
                root: cancel,
                next_owner: AtomicU64::new(1),
                changed: AtomicBool::new(false),
                wake_ui: tokio::sync::Notify::new(),
            }),
            version: None,
        }
    }

    /// A detached service over the engine clients.
    pub fn new(clients: ClientMap, cancel: CancellationToken) -> Self {
        Self::with_opener(client_opener(clients), cancel)
    }

    /// The running service: ticks every [`TICK`] on the tokio runtime and
    /// bumps its version signal on the UI side. Call from the App root.
    pub fn start(clients: ClientMap, cancel: CancellationToken) -> Self {
        let mut service = Self::new(clients, cancel.clone());
        let mut version = Signal::new(0_u64);
        service.version = Some(version);
        let inner = Arc::clone(&service.inner);
        spawn_io(async move {
            let mut interval = tokio::time::interval(TICK);
            let mut count = 0_u32;
            loop {
                tokio::select! {
                    () = cancel.cancelled() => break,
                    _ = interval.tick() => {}
                }
                inner.reconcile();
                count = count.wrapping_add(1);
                if count.is_multiple_of(20) {
                    inner.prune(Utc::now());
                }
                if inner.changed.swap(false, Ordering::AcqRel) {
                    inner.wake_ui.notify_one();
                }
            }
        });
        let wake = Arc::clone(&service.inner);
        spawn(async move {
            loop {
                wake.wake_ui.notified().await;
                version += 1;
            }
        });
        service
    }

    /// A fresh owner id for one component.
    pub fn new_owner(&self) -> OwnerId {
        self.inner.next_owner.fetch_add(1, Ordering::Relaxed)
    }

    /// This row shows (or, with `None`, stops showing) the container's
    /// stats. Cheap and idempotent: call it on every render.
    pub fn watch_row(&self, owner: OwnerId, key: Option<StatsKey>) {
        self.inner.lock().registry.set(owner, key.map(Want::Row));
    }

    /// An open Stats panel for the container (or none).
    pub fn watch_panel(&self, owner: OwnerId, key: Option<StatsKey>) {
        self.inner.lock().registry.set(owner, key.map(Want::Panel));
    }

    /// The owner is gone (component dropped, row scrolled out).
    pub fn release(&self, owner: OwnerId) {
        self.inner.lock().registry.set(owner, None);
    }

    /// Hidden windows hold no streams; showing the window reopens what is
    /// still on screen. Reconciles at once.
    pub fn set_window_visible(&self, visible: bool) {
        self.inner.lock().registry.window_visible = visible;
        self.reconcile();
    }

    /// Close streams nothing shows any more and open the ones that are
    /// wanted, never more than [`MAX_SUBSCRIPTIONS`] at a time.
    pub fn reconcile(&self) -> Reconciled {
        self.inner.reconcile()
    }

    /// Drop histories nothing has fed for a full window.
    pub fn prune(&self, now: DateTime<Utc>) {
        self.inner.prune(now);
    }

    /// Streams open right now.
    pub fn subscription_count(&self) -> usize {
        self.inner.lock().open.len()
    }

    /// Keys of the streams open right now.
    pub fn subscriptions(&self) -> BTreeSet<StatsKey> {
        self.inner.lock().open.keys().cloned().collect()
    }

    /// The newest sample if it is fresh. Reading it subscribes the calling
    /// component to the version signal.
    pub fn latest(&self, key: &StatsKey, now: DateTime<Utc>) -> Option<LiveStats> {
        self.track();
        self.inner
            .lock()
            .histories
            .get(key)
            .and_then(|h| h.fresh(now))
            .cloned()
    }

    /// A copy of the container's history (at most five minutes). Reading it
    /// subscribes the calling component to the version signal.
    pub fn history(&self, key: &StatsKey) -> History {
        self.track();
        self.inner
            .lock()
            .histories
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    /// Put samples into a container's history as if they had streamed in
    /// (dev scenes and tests).
    pub fn seed(
        &self,
        key: &StatsKey,
        samples: impl IntoIterator<Item = (DateTime<Utc>, LiveStats)>,
    ) {
        let mut state = self.inner.lock();
        let history = state.histories.entry(key.clone()).or_default();
        for (at, stats) in samples {
            history.push(at, stats);
        }
        drop(state);
        self.inner.changed.store(true, Ordering::Release);
    }

    fn track(&self) {
        if let Some(version) = self.version {
            let _ = version.read();
        }
    }
}

/// The opener over the live engine clients.
fn client_opener(clients: ClientMap) -> Opener {
    Arc::new(move |key, cancel| {
        let client = match clients.lock() {
            Ok(map) => map.get(&key.engine).cloned(),
            Err(poisoned) => poisoned.into_inner().get(&key.engine).cloned(),
        }?;
        Some(client.stats(&key.id, cancel))
    })
}

/// One stream's task: append samples until the stream ends or is closed.
async fn pump(inner: Arc<Inner>, key: StatsKey, generation: u64, cancel: CancellationToken) {
    let Some(mut stream) = (inner.opener)(&key, cancel.clone()) else {
        inner.ended(&key, generation);
        return;
    };
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            item = stream.next() => match item {
                Some(Ok(sample)) => inner.record(&key, Utc::now(), sample),
                Some(Err(err)) => {
                    tracing::debug!(%err, "stats stream failed");
                    break;
                }
                None => break,
            },
        }
    }
    inner.ended(&key, generation);
}

/// What the native window reports about itself. The webview's own
/// `document.visibilityState` is deliberately not an input: WebKit turns it
/// to `hidden` when another window merely covers this one, and a covered
/// window still shows the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowFacts {
    /// Minimised to the Dock.
    pub minimized: bool,
    /// Ordered in: false once the app is hidden (Cmd-H).
    pub shown: bool,
}

/// D26: streams run while the window is shown and not minimised. Another
/// app being frontmost (inactive), or covering the window, changes
/// neither.
#[must_use]
pub fn streams_allowed(facts: WindowFacts) -> bool {
    facts.shown && !facts.minimized
}

/// How often the native window state is read; a hide or minimise closes
/// the streams within this time.
const WINDOW_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// The UI side of D26's "closed when the window is hidden": the native
/// window's state (minimised, hidden) drives the service.
pub fn use_window_visibility(service: StatsService) {
    let window = dioxus::desktop::use_window();
    use_hook(move || {
        spawn(async move {
            let mut last = None;
            loop {
                let allowed = streams_allowed(WindowFacts {
                    minimized: window.is_minimized(),
                    shown: window.is_visible(),
                });
                if last != Some(allowed) {
                    last = Some(allowed);
                    service.set_window_visible(allowed);
                }
                tokio::time::sleep(WINDOW_POLL).await;
            }
        });
    });
}

/// The row-side hook: register `key` as shown by this component while it
/// is rendered, release it on drop. Pass `None` when the row has nothing
/// to stream (container not running).
pub fn use_row_stats(service: Option<&StatsService>, key: Option<StatsKey>) {
    let owner = use_hook(|| service.map(StatsService::new_owner));
    if let (Some(service), Some(owner)) = (service, owner) {
        service.watch_row(owner, key);
    }
    let service = service.cloned();
    use_drop(move || {
        if let (Some(service), Some(owner)) = (service, owner) {
            service.release(owner);
        }
    });
}

#[cfg(test)]
#[path = "stats_tests.rs"]
mod tests;
