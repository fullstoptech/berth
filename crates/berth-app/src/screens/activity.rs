//! Activity screen (P2-T10, D65): every Berth action and every engine
//! event as a live, virtual table. The feed model ([`Feed`], [`Filters`])
//! is plain data so pause and filtering are unit-tested without a DOM; the
//! component only wires it to the store (history), the activity tap (live
//! entries) and the shared UI primitives. No frame exists for this screen:
//! it reuses the Containers table layout (header, filters, full-bleed rows).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local};
use dioxus::prelude::*;

use crate::services::{activity, Services};
use crate::state::{AppState, EngineScope};
use crate::ui::{
    Badge, BadgeKind, Banner, BannerTone, Button, ButtonVariant, CellAlign, ControlSize,
    EmptyState, Icon, IconSize, SegmentedControl, SegmentedItem, Select, SelectOption,
    TableHeaderCell, VirtualTable,
};

const ACTIVITY_CSS: &str = include_str!("../../assets/css/activity.css");

/// How many entries the screen holds in memory (newest first). The store
/// keeps 30 days; a busier history than this is reached by narrowing the
/// time, not by scrolling the whole table.
pub const MAX_LOADED: usize = 5000;

// ---------------------------------------------------------------- filters

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OriginFilter {
    #[default]
    All,
    Engine,
    Berth,
    Plugin,
}

impl OriginFilter {
    fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Engine => "engine",
            Self::Berth => "berth",
            Self::Plugin => "plugin",
        }
    }

    fn from_id(id: &str) -> Self {
        match id {
            "engine" => Self::Engine,
            "berth" => Self::Berth,
            "plugin" => Self::Plugin,
            _ => Self::All,
        }
    }

    fn matches(self, origin: &berth_core::ActivityOrigin) -> bool {
        use berth_core::ActivityOrigin as O;
        matches!(
            (self, origin),
            (Self::All, _)
                | (Self::Engine, O::Engine)
                | (Self::Berth, O::Berth)
                | (Self::Plugin, O::Plugin(_))
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutcomeFilter {
    #[default]
    All,
    Ok,
    Failed,
}

impl OutcomeFilter {
    fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Ok => "ok",
            Self::Failed => "failed",
        }
    }

    fn from_id(id: &str) -> Self {
        match id {
            "ok" => Self::Ok,
            "failed" => Self::Failed,
            _ => Self::All,
        }
    }

    fn matches(self, outcome: &berth_core::Outcome) -> bool {
        matches!(
            (self, outcome),
            (Self::All, _)
                | (Self::Ok, berth_core::Outcome::Ok)
                | (Self::Failed, berth_core::Outcome::Failed { .. })
        )
    }
}

/// The four filters of the screen (engine, project, origin, outcome).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    pub engine: Option<berth_core::EngineId>,
    pub project: Option<berth_core::ProjectId>,
    pub origin: OriginFilter,
    pub outcome: OutcomeFilter,
}

/// Which project a container belongs to, from the live container list.
/// Engine events carry no project of their own, so the project filter
/// resolves it here at read time.
pub type ProjectOf = HashMap<berth_core::ContainerId, berth_core::ProjectId>;

pub fn project_map(containers: &[berth_core::Container]) -> ProjectOf {
    containers
        .iter()
        .filter_map(|c| c.project.clone().map(|p| (c.id.clone(), p)))
        .collect()
}

/// A gap marker, not an action: the engine's event stream was down.
pub fn is_gap(entry: &berth_core::ActivityEntry) -> bool {
    entry.origin == berth_core::ActivityOrigin::Engine && entry.action == activity::GAP_ACTION
}

fn project_of<'a>(
    entry: &'a berth_core::ActivityEntry,
    projects: &'a ProjectOf,
) -> Option<&'a berth_core::ProjectId> {
    entry.project.as_ref().or_else(|| match &entry.subject {
        berth_core::Subject::Container(id) => projects.get(id),
        berth_core::Subject::Project(id) => Some(id),
        _ => None,
    })
}

impl Filters {
    pub fn is_active(&self) -> bool {
        *self != Self::default()
    }

    /// A gap marker answers only to the engine filter: it says the feed of
    /// that engine is incomplete whatever else is being looked at.
    pub fn matches(&self, entry: &berth_core::ActivityEntry, projects: &ProjectOf) -> bool {
        let engine_ok = self.engine.as_ref().is_none_or(|e| *e == entry.engine);
        if is_gap(entry) {
            return engine_ok;
        }
        engine_ok
            && self
                .project
                .as_ref()
                .is_none_or(|p| project_of(entry, projects) == Some(p))
            && self.origin.matches(&entry.origin)
            && self.outcome.matches(&entry.outcome)
    }
}

// ------------------------------------------------------------------- feed

/// The entries on screen (newest first) plus, while paused, the entries
/// that arrived since. Pause freezes the table so a row under the pointer
/// does not move; the held entries are counted, not lost.
#[derive(Debug, Clone, Default)]
pub struct Feed {
    entries: Vec<berth_core::ActivityEntry>,
    held: Vec<berth_core::ActivityEntry>,
    ids: HashSet<uuid::Uuid>,
    paused: bool,
}

/// What a scene or test hands the screen instead of the store.
#[derive(Debug, Clone, Default)]
pub struct FeedSeed {
    pub entries: Vec<berth_core::ActivityEntry>,
    pub held: Vec<berth_core::ActivityEntry>,
    pub paused: bool,
}

impl Feed {
    pub fn from_seed(seed: FeedSeed) -> Self {
        let mut feed = Self::default();
        feed.extend(seed.entries);
        feed.paused = seed.paused;
        feed.extend(seed.held);
        feed
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Entries on screen, held ones excluded.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&berth_core::ActivityEntry> {
        self.entries.get(index)
    }

    /// Add one entry: on screen at its place in time order, or held while
    /// paused. An id already seen is ignored (history and live overlap).
    pub fn push(&mut self, entry: berth_core::ActivityEntry) {
        if !self.ids.insert(entry.id) {
            return;
        }
        if self.paused {
            self.held.push(entry);
        } else {
            self.insert_sorted(entry);
            self.trim();
        }
    }

    /// Add many entries (history, a batch of live ones) with one sort.
    pub fn extend(&mut self, entries: impl IntoIterator<Item = berth_core::ActivityEntry>) {
        let mut added = false;
        for entry in entries {
            if !self.ids.insert(entry.id) {
                continue;
            }
            if self.paused {
                self.held.push(entry);
            } else {
                self.entries.push(entry);
                added = true;
            }
        }
        if added {
            // Stable: entries of equal time keep their incoming order.
            self.entries.sort_by_key(|e| std::cmp::Reverse(e.ts));
            self.trim();
        }
    }

    fn insert_sorted(&mut self, entry: berth_core::ActivityEntry) {
        let at = self.entries.partition_point(|e| e.ts > entry.ts);
        self.entries.insert(at, entry);
    }

    fn trim(&mut self) {
        if self.entries.len() > MAX_LOADED {
            for dropped in self.entries.drain(MAX_LOADED..) {
                self.ids.remove(&dropped.id);
            }
        }
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Show everything held and follow the feed again. Returns how many
    /// entries joined the table.
    pub fn resume(&mut self) -> usize {
        self.paused = false;
        let held = std::mem::take(&mut self.held);
        let count = held.len();
        // Held entries are already in `ids`; add them without the check.
        self.entries.extend(held);
        self.entries.sort_by_key(|e| std::cmp::Reverse(e.ts));
        self.trim();
        count
    }

    /// New events waiting behind the pause that the current filters would
    /// show (a gap marker counts as an event).
    pub fn held_count(&self, filters: &Filters, projects: &ProjectOf) -> usize {
        self.held
            .iter()
            .filter(|e| filters.matches(e, projects))
            .count()
    }

    /// Indexes into the feed of the rows the filters keep, in table order.
    pub fn visible(&self, filters: &Filters, projects: &ProjectOf) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| filters.matches(e, projects))
            .map(|(i, _)| i)
            .collect()
    }
}

/// The pause pill: "23 new events".
pub fn new_events_label(count: usize) -> String {
    match count {
        0 => "No new events".to_string(),
        1 => "1 new event".to_string(),
        n => format!("{n} new events"),
    }
}

// ------------------------------------------------------------ row content

/// Time column: clock time today, with the date on older days.
pub fn format_time(ts: DateTime<Local>, now: DateTime<Local>) -> String {
    if ts.date_naive() == now.date_naive() {
        ts.format("%H:%M:%S").to_string()
    } else {
        ts.format("%b %-d %H:%M:%S").to_string()
    }
}

/// Action column: `container.die` reads "die"; other types keep their
/// type, `image.pull` reads "image pull"; a health status keeps its word
/// ("health status: unhealthy"). Rows stored before commands were dropped
/// may still carry one after a colon: it is never shown.
pub fn action_label(action: &str) -> String {
    let action = activity::clean_action(action).unwrap_or_default();
    let label = |rest: &str| rest.replace('_', " ");
    match action.split_once('.') {
        Some(("container", rest)) => label(rest),
        Some((ty, rest)) => format!("{ty} {}", label(rest)),
        None => label(&action),
    }
}

pub fn origin_label(origin: &berth_core::ActivityOrigin) -> String {
    match origin {
        berth_core::ActivityOrigin::Engine => "Engine".to_string(),
        berth_core::ActivityOrigin::Berth => "Berth".to_string(),
        berth_core::ActivityOrigin::Plugin(id) => format!("Plugin \u{00b7} {id}"),
    }
}

fn subject_icon(subject: &berth_core::Subject) -> &'static str {
    match subject {
        berth_core::Subject::Container(_) => "box",
        berth_core::Subject::Project(_) => "folder",
        berth_core::Subject::Engine(_) => "server",
        berth_core::Subject::Image(_) => "layers",
        berth_core::Subject::Volume(_) => "hard-drive",
        berth_core::Subject::Network(_) => "network",
        berth_core::Subject::Plugin(_) => "cloud",
    }
}

/// The engine as the user knows it: its name, else the id without its
/// kind prefix (`colima:default` reads "default").
pub fn engine_name(engines: &[berth_core::Engine], id: &berth_core::EngineId) -> String {
    engines.iter().find(|e| e.id == *id).map_or_else(
        || {
            id.0.split_once(':')
                .map_or_else(|| id.0.clone(), |(_, name)| name.to_string())
        },
        |e| e.name.clone(),
    )
}

fn entry_row(
    entry: &berth_core::ActivityEntry,
    engines: &[berth_core::Engine],
    now: DateTime<Local>,
) -> Element {
    let time = format_time(entry.ts.with_timezone(&Local), now);
    let engine = engine_name(engines, &entry.engine);
    if is_gap(entry) {
        return rsx! {
            div { class: "activity-gap", role: "cell",
                span { class: "activity-gap-time", "{time}" }
                Icon { name: "alert-triangle", size: IconSize::Sm }
                span { class: "activity-gap-text",
                    "{engine} disconnected. Events during the gap are missing."
                }
            }
        };
    }
    let origin = origin_label(&entry.origin);
    let action = action_label(&entry.action);
    let action_title = activity::clean_action(&entry.action).unwrap_or_default();
    let icon = subject_icon(&entry.subject);
    let failed_reason = match &entry.outcome {
        berth_core::Outcome::Ok => None,
        berth_core::Outcome::Failed { reason } => Some(reason.clone()),
    };
    rsx! {
        div { class: "activity-cols",
            span { class: "activity-cell activity-time", role: "cell", "{time}" }
            span { class: "activity-cell activity-engine", role: "cell", "{engine}" }
            span { class: "activity-cell activity-origin", role: "cell", "{origin}" }
            span {
                class: "activity-cell activity-action",
                role: "cell",
                title: "{action_title}",
                "{action}"
            }
            span { class: "activity-cell activity-subject", role: "cell",
                span { class: "activity-subject-icon", Icon { name: icon, size: IconSize::Sm } }
                span { class: "activity-subject-name", "{entry.subject_name}" }
            }
            span { class: "activity-cell activity-outcome", role: "cell",
                if let Some(reason) = failed_reason {
                    span { class: "activity-outcome-failed", title: "{reason}",
                        Icon { name: "alert-circle", size: IconSize::Sm }
                    }
                    span { class: "activity-outcome-failed", "Failed" }
                    span { class: "activity-outcome-reason activity-outcome-failed", "{reason}" }
                } else {
                    span { class: "activity-outcome-ok",
                        Icon { name: "check", size: IconSize::Sm }
                    }
                    span { class: "activity-outcome-ok", "OK" }
                }
            }
        }
    }
}

// ------------------------------------------------------------- scene seed

thread_local! {
    static SEED: std::cell::RefCell<Option<FeedSeed>> = const { std::cell::RefCell::new(None) };
}

/// Hand the next mounted Activity screen a feed instead of the store (frame
/// scenes and tests, D58). Consumed once, on the UI thread.
pub fn seed_feed(seed: FeedSeed) {
    SEED.with(|slot| *slot.borrow_mut() = Some(seed));
}

fn take_seed() -> Option<FeedSeed> {
    SEED.with(|slot| slot.borrow_mut().take())
}

// -------------------------------------------------------------- component

fn initial_filters(state: &AppState) -> Filters {
    // Honour the title bar's engine scope when the screen opens; the
    // select below changes it from there.
    match &*state.engine_scope.peek() {
        EngineScope::One(id) => Filters {
            engine: Some(id.clone()),
            ..Filters::default()
        },
        EngineScope::All => Filters::default(),
    }
}

/// Read the newest history from the store off the UI thread and merge it.
async fn load_history(store: std::sync::Arc<berth_store::Store>, mut feed: Signal<Feed>) {
    let history = tokio::task::spawn_blocking(move || activity::load_recent(&store, MAX_LOADED))
        .await
        .unwrap_or_default();
    // Rows stored before exec events were dropped carry commands: skip them.
    let history = history
        .into_iter()
        .filter(|e| activity::clean_action(&e.action).is_some());
    feed.write().extend(history);
}

async fn follow(services: Services, mut feed: Signal<Feed>, mut loaded: Signal<bool>) {
    use tokio::sync::broadcast::error::{RecvError, TryRecvError};
    // Subscribe first so nothing falls between history and live.
    let mut rx = activity::subscribe();
    load_history(std::sync::Arc::clone(&services.store), feed).await;
    loaded.set(true);
    loop {
        let mut batch = Vec::new();
        let mut lagged = false;
        match rx.recv().await {
            Ok(entry) => batch.push(entry),
            Err(RecvError::Lagged(_)) => lagged = true,
            Err(RecvError::Closed) => return,
        }
        loop {
            match rx.try_recv() {
                Ok(entry) => batch.push(entry),
                Err(TryRecvError::Lagged(_)) => lagged = true,
                Err(_) => break,
            }
        }
        if !batch.is_empty() {
            feed.write().extend(batch);
        }
        if lagged {
            load_history(std::sync::Arc::clone(&services.store), feed).await;
        }
    }
}

#[component]
pub fn ActivityScreen() -> Element {
    let state = use_context::<AppState>();
    let services = try_use_context::<Services>();
    let seed = use_hook(take_seed);
    let seeded = seed.is_some();
    let feed = use_signal(|| seed.clone().map(Feed::from_seed).unwrap_or_default());
    let mut filters = use_signal(|| initial_filters(&state));
    let mut loaded = use_signal(|| seeded);

    use_future(move || {
        let services = services.clone();
        async move {
            match services {
                Some(services) if !seeded => follow(services, feed, loaded).await,
                _ => loaded.set(true),
            }
        }
    });

    let rows = use_memo(move || {
        let projects = project_map(&state.containers.read());
        feed.read().visible(&filters.read(), &projects)
    });
    let held = use_memo(move || {
        let projects = project_map(&state.containers.read());
        feed.read().held_count(&filters.read(), &projects)
    });

    let paused = feed.read().is_paused();
    let shown = rows.read().len();
    let total = feed.read().len();
    let active = filters.read().is_active();
    let row_height_px = match state.settings.read().density {
        berth_store::Density::Compact => 24,
        berth_store::Density::Comfortable => 28,
    };

    let render_row = use_callback(move |i: usize| {
        let rows = rows.read();
        let feed = feed.read();
        let engines = state.engines.read();
        match rows.get(i).and_then(|&index| feed.get(index)) {
            Some(entry) => entry_row(entry, &engines, Local::now()),
            None => rsx! {},
        }
    });

    let engine_options = {
        let mut options = vec![SelectOption::new("", "All engines")];
        options.extend(
            state
                .engines
                .read()
                .iter()
                .map(|e| SelectOption::new(e.id.0.clone(), e.name.clone())),
        );
        options
    };
    let project_options = {
        let mut projects: Vec<(String, String)> = state
            .projects
            .read()
            .iter()
            .map(|p| (p.id.0.clone(), p.name.clone()))
            .collect();
        projects.sort_by(|a, b| a.1.cmp(&b.1));
        let mut options = vec![SelectOption::new("", "All projects")];
        options.extend(
            projects
                .into_iter()
                .map(|(id, name)| SelectOption::new(id, name)),
        );
        options
    };
    let origin_options = vec![
        SelectOption::new("all", "All origins"),
        SelectOption::new("engine", "Engine"),
        SelectOption::new("berth", "Berth"),
        SelectOption::new("plugin", "Plugins"),
    ];
    let engine_value = filters
        .read()
        .engine
        .as_ref()
        .map(|e| e.0.clone())
        .unwrap_or_default();
    let project_value = filters
        .read()
        .project
        .as_ref()
        .map(|p| p.0.clone())
        .unwrap_or_default();
    let origin_value = filters.read().origin.id().to_string();
    let outcome_value = filters.read().outcome.id().to_string();
    let pause_label = if paused { "Resume feed" } else { "Pause feed" };
    let pause_icon = if paused { "play" } else { "pause" };
    let banner = format!("Feed paused. {}", new_events_label(*held.read()));

    rsx! {
        style { "{ACTIVITY_CSS}" }
        section { class: "activity-screen", "aria-label": "Activity",
            div { class: "activity-head",
                h1 { class: "activity-title", "Activity" }
                Badge { kind: BadgeKind::Count, text: "{shown}" }
                span { class: "activity-spacer" }
                Button {
                    label: pause_label.to_string(),
                    icon: pause_icon,
                    variant: ButtonVariant::Secondary,
                    onclick: move |_| {
                        let mut feed = feed;
                        if feed.read().is_paused() {
                            feed.write().resume();
                        } else {
                            feed.write().pause();
                        }
                    },
                }
            }
            div { class: "activity-filters",
                div { class: "activity-filter-select",
                    Select {
                        aria_label: "Engine".to_string(),
                        value: engine_value,
                        options: engine_options,
                        onchange: move |e: FormEvent| {
                            let value = e.value();
                            filters.write().engine =
                                (!value.is_empty()).then_some(berth_core::EngineId(value));
                        },
                    }
                }
                div { class: "activity-filter-select",
                    Select {
                        aria_label: "Project".to_string(),
                        value: project_value,
                        options: project_options,
                        onchange: move |e: FormEvent| {
                            let value = e.value();
                            filters.write().project =
                                (!value.is_empty()).then_some(berth_core::ProjectId(value));
                        },
                    }
                }
                div { class: "activity-filter-select",
                    Select {
                        aria_label: "Origin".to_string(),
                        value: origin_value,
                        options: origin_options,
                        onchange: move |e: FormEvent| {
                            filters.write().origin = OriginFilter::from_id(&e.value());
                        },
                    }
                }
                SegmentedControl {
                    aria_label: "Outcome".to_string(),
                    selected: outcome_value,
                    items: vec![
                        SegmentedItem::new("all", "All"),
                        SegmentedItem::new("ok", "OK"),
                        SegmentedItem::new("failed", "Failed"),
                    ],
                    on_select: move |id: String| {
                        filters.write().outcome = OutcomeFilter::from_id(&id);
                    },
                }
                if active {
                    Button {
                        label: "Clear filters".to_string(),
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Md,
                        onclick: move |_| filters.set(Filters::default()),
                    }
                }
            }
            if paused {
                div { class: "activity-banner",
                    Banner {
                        message: banner,
                        tone: BannerTone::Info,
                        action_label: Some("Resume".to_string()),
                        onaction: move |()| {
                            let mut feed = feed;
                            feed.write().resume();
                        },
                    }
                }
            }
            div { class: "activity-table",
                div { class: "activity-thead", role: "row",
                    div { class: "activity-cols",
                        TableHeaderCell { label: "Time".to_string() }
                        TableHeaderCell { label: "Engine".to_string() }
                        TableHeaderCell { label: "Origin".to_string() }
                        TableHeaderCell { label: "Action".to_string() }
                        TableHeaderCell { label: "Subject".to_string() }
                        TableHeaderCell { label: "Outcome".to_string(), align: CellAlign::Left }
                    }
                }
                if shown > 0 {
                    VirtualTable {
                        total: shown,
                        render_row,
                        row_height_px,
                        viewport_height_px: 1400_u32,
                    }
                } else if *loaded.read() && total == 0 {
                    EmptyState {
                        title: "No activity yet".to_string(),
                        body: "What you do in Berth and what your engines report appears here, kept for 30 days."
                            .to_string(),
                        icon: "activity",
                    }
                } else if *loaded.read() {
                    EmptyState {
                        title: "No matching activity".to_string(),
                        body: "Nothing in the feed matches these filters.".to_string(),
                        icon: "filter",
                        action_label: Some("Clear filters".to_string()),
                        onaction: move |()| filters.set(Filters::default()),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
