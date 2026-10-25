//! Projects screen (P2-T6, brief 7.3, frames "Projects / default / 1440",
//! its dark twin and "Projects / empty / 1440"): header with count,
//! filter, segmented status filter and Open folder; one `ProjectRow` per
//! project; "Needs attention" under the table. Reads signals only; every
//! button goes through the `ActionsService` (D23, D34).

use dioxus::prelude::*;

use super::projects_order::FrozenOrder;
use crate::services::actions::{ActionsService, StateView};
use crate::state::{AppState, Section, Selection};
use crate::ui::project_row::{row_model, ProjectRow};
use crate::ui::CellAlign;
use crate::ui::{
    Badge, BadgeKind, Banner, BannerTone, Button, ButtonVariant, ControlSize, EmptyState, Icon,
    IconSize, SearchField, SegmentedControl, SegmentedItem, SkeletonRows, TableHeaderCell,
};

pub(crate) const PROJECTS_CSS: &str = concat!(
    include_str!("../../assets/css/projects.css"),
    include_str!("../../assets/css/projects_tiers.css")
);

/// The skeleton shows only after this long, so a fast engine never
/// flashes it (brief 7.3 "loading: skeleton rows after 300 ms").
pub const SKELETON_DELAY_MS: u64 = 300;

/// Copy that the brief fixes for the empty state (7.3).
pub const EMPTY_LABEL: &str = "Open a folder to see its containers here";

/// The Open folder buttons: the folder browser sheet (⌘O does the same).
fn open_folder_browser(state: &mut AppState) {
    let open = state
        .overlays
        .peek()
        .iter()
        .any(|o| matches!(o, crate::state::Overlay::OpenFolder));
    if !open {
        state
            .overlays
            .write()
            .push(crate::state::Overlay::OpenFolder);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectFilter {
    All,
    Running,
    Failing,
    Stopped,
}

impl ProjectFilter {
    pub const ALL: [Self; 4] = [Self::All, Self::Running, Self::Failing, Self::Stopped];

    pub fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Running => "running",
            Self::Failing => "failing",
            Self::Stopped => "stopped",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Running => "Running",
            Self::Failing => "Failing",
            Self::Stopped => "Stopped",
        }
    }

    pub fn from_id(id: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|f| f.id() == id)
            .unwrap_or(Self::All)
    }

    /// Running means every service up; a partly running project shows
    /// under All only (frame: Running 3 · Failing 1 · Stopped 2 of 8).
    pub fn matches(self, status: berth_core::ProjectStatus) -> bool {
        use berth_core::ProjectStatus as P;
        match self {
            Self::All => true,
            Self::Running => status == P::AllRunning,
            Self::Failing => status == P::Failing,
            Self::Stopped => status == P::Stopped,
        }
    }
}

/// The Unassigned row holds the containers that belong to no project (AD5).
/// It is a holding row, not a project: it never counts anywhere.
pub fn is_unassigned(project: &berth_core::Project) -> bool {
    matches!(project.source, berth_core::ProjectSource::Loose)
}

/// How many real projects (everything but the Unassigned row) there are.
pub fn real_count(projects: &[&berth_core::Project]) -> usize {
    projects.iter().filter(|p| !is_unassigned(p)).count()
}

/// The four segment counts, in `ProjectFilter::ALL` order. Real projects
/// only, so All equals the header badge.
pub fn filter_counts(projects: &[&berth_core::Project]) -> [usize; 4] {
    ProjectFilter::ALL.map(|f| {
        projects
            .iter()
            .filter(|p| !is_unassigned(p) && f.matches(p.status))
            .count()
    })
}

/// Whether the status filter lists this row. The Unassigned row shows
/// under All only: Running, Failing and Stopped are about projects.
pub fn row_shown(project: &berth_core::Project, filter: ProjectFilter) -> bool {
    if is_unassigned(project) {
        filter == ProjectFilter::All
    } else {
        filter.matches(project.status)
    }
}

/// Case-insensitive match on name, folder, branch and engine name.
pub fn matches_query(project: &berth_core::Project, engine_name: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let branch = project
        .git
        .as_ref()
        .and_then(|g| g.branch.as_deref())
        .unwrap_or_default();
    let folder = project
        .folder
        .as_deref()
        .map(|f| f.display().to_string())
        .unwrap_or_default();
    [project.name.as_str(), &folder, branch, engine_name]
        .iter()
        .any(|field| field.to_lowercase().contains(&query))
}

/// The Unassigned row matches the query when it matches "Unassigned" or the
/// name of one of its containers (AD5). It has no folder, branch or engine
/// name worth matching.
pub fn unassigned_matches_query(
    project: &berth_core::Project,
    containers: &[berth_core::Container],
    query: &str,
) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let ids: Vec<&berth_core::ContainerId> = project
        .services
        .iter()
        .flat_map(|s| s.containers.iter())
        .collect();
    "unassigned".contains(&query)
        || containers
            .iter()
            .filter(|c| c.engine == project.engine && ids.contains(&&c.id))
            .any(|c| c.name.to_lowercase().contains(&query))
}

/// What the table lists for the filter text and the status filter.
#[derive(Debug)]
pub struct Listing<'a> {
    /// Real projects that match the text, whatever the status filter.
    pub matched: usize,
    /// The rows to draw, in the order given.
    pub visible: Vec<&'a berth_core::Project>,
}

/// Apply the filter text, then the status filter, to `ordered`.
pub fn list_projects<'a>(
    ordered: &[&'a berth_core::Project],
    engines: &[berth_core::Engine],
    containers: &[berth_core::Container],
    query: &str,
    filter: ProjectFilter,
) -> Listing<'a> {
    let engine_name = |p: &berth_core::Project| {
        engines
            .iter()
            .find(|e| e.id == p.engine)
            .map_or_else(|| p.engine.0.clone(), |e| e.name.clone())
    };
    let by_query: Vec<&berth_core::Project> = ordered
        .iter()
        .copied()
        .filter(|p| {
            if is_unassigned(p) {
                unassigned_matches_query(p, containers, query)
            } else {
                matches_query(p, &engine_name(p), query)
            }
        })
        .collect();
    Listing {
        // "N of M" counts projects: the Unassigned row is in neither number.
        matched: real_count(&by_query),
        visible: by_query
            .into_iter()
            .filter(|p| row_shown(p, filter))
            .collect(),
    }
}

/// Failing first, then partly running, running, stopped, not created;
/// projects on an engine that is not running after those; the Unassigned
/// project last of all, as on the Containers screen. Stable, so the
/// engine's own order holds inside a group.
pub fn sort_rank(
    project: &berth_core::Project,
    engines: &[berth_core::Engine],
) -> (bool, bool, u8) {
    use berth_core::ProjectStatus as P;
    let loose = matches!(project.source, berth_core::ProjectSource::Loose);
    let engine_down = engines
        .iter()
        .find(|e| e.id == project.engine)
        .is_some_and(|e| e.state != berth_core::EngineState::Running);
    let rank = match project.status {
        P::Failing => 0,
        P::Partial { .. } => 1,
        P::AllRunning => 2,
        P::Stopped => 3,
        P::NotCreated => 4,
    };
    (loose, engine_down, rank)
}

/// The projects in table order: [`sort_rank`], then the engine's place in
/// the engine list, then the order the engine itself listed them in (name,
/// then folder: `berth_engine::projects::group`). The engines' slices are
/// rewritten at the end of the list on every container event, so the slice
/// order is never part of the key.
pub fn ordered_projects<'a>(
    projects: &'a [berth_core::Project],
    engines: &[berth_core::Engine],
) -> Vec<&'a berth_core::Project> {
    let place = |p: &berth_core::Project| {
        engines
            .iter()
            .position(|e| e.id == p.engine)
            .unwrap_or(usize::MAX)
    };
    let mut ordered: Vec<&berth_core::Project> = projects.iter().collect();
    ordered.sort_by_key(|p| (sort_rank(p, engines), place(p)));
    ordered
}

/// What the main area shows (brief 7.3 states).
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenState {
    /// No engine has answered yet.
    Loading,
    /// An engine answered and knows no project.
    Empty,
    /// Nothing is running and a Colima engine is stopped: offer Start.
    EngineStopped(Box<berth_core::Engine>),
    /// Nothing is running and no engine can be reached.
    Unreachable,
    Rows,
}

pub fn screen_state(
    engines: &[berth_core::Engine],
    projects: &[berth_core::Project],
    snapshot_stale: bool,
) -> ScreenState {
    use berth_core::EngineState as E;
    if !projects.is_empty() {
        return ScreenState::Rows;
    }
    if engines.is_empty() {
        return ScreenState::Loading;
    }
    let running = engines.iter().any(|e| e.state == E::Running);
    if running {
        return if snapshot_stale {
            ScreenState::Loading
        } else {
            ScreenState::Empty
        };
    }
    if engines
        .iter()
        .any(|e| matches!(e.state, E::Starting | E::Stopping))
    {
        return ScreenState::Loading;
    }
    match engines.iter().find(|e| e.state == E::Stopped) {
        Some(engine) => ScreenState::EngineStopped(Box::new(engine.clone())),
        None => ScreenState::Unreachable,
    }
}

/// One "Needs attention" card.
#[derive(Debug, Clone, PartialEq)]
pub struct AttentionItem {
    pub severity: berth_core::Severity,
    pub title: String,
    pub subject_name: String,
    pub age: String,
    pub open: OpenTarget,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpenTarget {
    Container(berth_core::ContainerId),
    Project(berth_core::ProjectId),
    Engine(berth_core::EngineId),
}

/// Errors before warnings, newest first inside a severity. Info problems
/// are not "needs attention".
pub fn attention_items(
    problems: &[berth_core::Problem],
    containers: &[berth_core::Container],
    projects: &[berth_core::Project],
    engines: &[berth_core::Engine],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<AttentionItem> {
    use berth_core::{Severity, Subject};
    let mut shown: Vec<&berth_core::Problem> = problems
        .iter()
        .filter(|p| matches!(p.severity, Severity::Error | Severity::Warning))
        .collect();
    shown.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(b.first_seen.cmp(&a.first_seen))
    });
    shown
        .into_iter()
        .filter_map(|p| {
            let (subject_name, open) = match &p.subject {
                Subject::Container(id) => (
                    containers.iter().find(|c| c.id == *id)?.name.clone(),
                    OpenTarget::Container(id.clone()),
                ),
                Subject::Project(id) => (
                    projects.iter().find(|x| x.id == *id)?.name.clone(),
                    OpenTarget::Project(id.clone()),
                ),
                Subject::Engine(id) => (
                    engines.iter().find(|e| e.id == *id)?.name.clone(),
                    OpenTarget::Engine(id.clone()),
                ),
                _ => return None,
            };
            Some(AttentionItem {
                severity: p.severity,
                title: p.title.clone(),
                subject_name,
                age: berth_core::fmt::uptime(p.first_seen, now),
                open,
            })
        })
        .collect()
}

/// The JS the loading body waits on before it draws the skeleton.
pub fn delay_script() -> String {
    format!("await new Promise(r => setTimeout(r, {SKELETON_DELAY_MS}));")
}

/// Rows of skeleton, drawn only after [`SKELETON_DELAY_MS`]. `shown`
/// starts it visible (tests and scenes; SSR never runs effects).
#[component]
pub fn LoadingBody(#[props(default)] shown: bool) -> Element {
    let mut visible = use_signal(|| shown);
    use_effect(move || {
        if !*visible.peek() {
            spawn(async move {
                let _ = document::eval(&delay_script()).await;
                visible.set(true);
            });
        }
    });
    rsx! {
        div { class: "projects-loading",
            if *visible.read() {
                SkeletonRows { count: 8_usize }
            }
        }
    }
}

#[component]
fn AttentionCard(item: AttentionItem) -> Element {
    let mut state = use_context::<AppState>();
    let (tone, icon) = match item.severity {
        berth_core::Severity::Error => ("tone-danger", "alert-circle"),
        _ => ("tone-warning", "alert-triangle"),
    };
    let open = item.open.clone();
    rsx! {
        div { class: "attention-card",
            span { class: "{tone}", Icon { name: icon, size: IconSize::Md } }
            span { class: "attention-title", "{item.title}" }
            span { class: "attention-subject", "{item.subject_name}" }
            span { class: "attention-age", "{item.age}" }
            Button {
                label: "Open {item.subject_name}",
                variant: ButtonVariant::Ghost,
                size: ControlSize::Sm,
                onclick: move |_| match &open {
                    OpenTarget::Container(id) => {
                        state.selection.set(Selection::Containers(vec![id.clone()]));
                        state.section.set(Section::Containers);
                    }
                    OpenTarget::Project(id) => {
                        state.selection.set(Selection::Project(id.clone()));
                        state.section.set(Section::Project);
                    }
                    OpenTarget::Engine(id) => {
                        state.selection.set(Selection::Engine(id.clone()));
                        state.section.set(Section::Engines);
                    }
                },
            }
        }
    }
}

/// Start a stopped Colima engine through the service. A refusal lands in
/// the log; the banner keeps its button so the user can retry.
fn start_engine(
    state: &mut AppState,
    service: Option<&ActionsService>,
    engine: &berth_core::Engine,
) {
    let Some(service) = service else { return };
    if let Err(reason) = service.engine(state, engine, berth_core::ColimaOp::Start) {
        tracing::warn!(%reason, engine = %engine.name, "start engine refused");
    }
}

#[component]
fn EngineStoppedBanner(engine: berth_core::Engine) -> Element {
    let mut state = use_context::<AppState>();
    let service = try_use_context::<ActionsService>();
    let target = engine.clone();
    rsx! {
        Banner {
            message: format!(
                "Colima {} is stopped. Its projects are not running.",
                engine.name
            ),
            tone: BannerTone::Warning,
            action_label: Some("Start engine".to_string()),
            onaction: move |()| start_engine(&mut state, service.as_ref(), &target),
        }
    }
}

/// The empty body: the empty state and drop zone, centred in the main area.
/// With a first-run card in `first_run` (P6 fills the slot) the block moves
/// left and the card sits on its right, as in the frame; nothing fills the
/// slot today, so it renders centred.
#[component]
fn EmptyBody(#[props(default)] first_run: Option<Element>) -> Element {
    let state = try_use_context::<AppState>();
    let class = if first_run.is_some() {
        "projects-empty has-first-run"
    } else {
        "projects-empty"
    };
    rsx! {
        div { class, "aria-label": EMPTY_LABEL, role: "region",
            div { class: "projects-empty-column",
                EmptyState {
                    title: "No projects yet".to_string(),
                    body: "Open a folder and Berth finds its Compose file, or writes one for you."
                        .to_string(),
                }
                Button {
                    label: "Open folder",
                    variant: ButtonVariant::Primary,
                    onclick: move |_| {
                        if let Some(mut state) = state {
                            open_folder_browser(&mut state);
                        }
                    },
                }
                div { class: "drop-zone", role: "note",
                    span { "Drop a folder here" }
                    span { class: "drop-zone-hint", "or press {crate::actions::mod_label()}O" }
                }
            }
            if let Some(card) = first_run {
                div { class: "projects-first-run", {card} }
            }
        }
    }
}

#[component]
fn EngineStoppedBody(engine: berth_core::Engine) -> Element {
    let mut state = use_context::<AppState>();
    let service = try_use_context::<ActionsService>();
    let target = engine.clone();
    rsx! {
        div { class: "projects-empty",
            EmptyState {
                title: "Start the engine to see its projects".to_string(),
                body: format!("Colima {} is stopped.", engine.name),
                icon: "server",
                action_label: Some(format!("Start engine {}", engine.name)),
                onaction: move |()| start_engine(&mut state, service.as_ref(), &target),
            }
        }
    }
}

#[component]
fn Header(total: usize, show_controls: bool, counts: [usize; 4], matched: usize) -> Element {
    // The filter lives in the screen's signals so it survives a rebuild of
    // the header; `rev` remounts the uncontrolled field to clear it (D59).
    let mut state = use_context::<AppState>();
    let mut query = use_context::<Signal<String>>();
    let mut rev = use_context::<Signal<u32>>();
    let mut filter = use_context::<Signal<ProjectFilter>>();
    let has_query = !query.read().trim().is_empty();
    let items: Vec<SegmentedItem> = ProjectFilter::ALL
        .iter()
        .zip(counts)
        .map(|(f, n)| SegmentedItem::with_count(f.id(), f.label(), n))
        .collect();
    let selected = filter.read().id().to_string();
    let rev_key = *rev.read();
    rsx! {
        header { class: "projects-header",
            h1 { class: "projects-title", "Projects" }
            Badge { kind: BadgeKind::Count, text: total.to_string() }
            if show_controls {
                div { class: "projects-controls",
                    div { class: "projects-search",
                        for key in [rev_key] {
                            SearchField {
                                key: "{key}",
                                placeholder: "Filter projects".to_string(),
                                aria_label: "Filter projects".to_string(),
                                matched: has_query.then_some(matched),
                                total: has_query.then_some(total),
                                oninput: move |e: FormEvent| query.set(e.value()),
                                onclear: move |_| {
                                    query.set(String::new());
                                    rev.set(key + 1);
                                },
                            }
                        }
                    }
                    SegmentedControl {
                        items,
                        selected,
                        aria_label: "Project status".to_string(),
                        on_select: move |id: String| filter.set(ProjectFilter::from_id(&id)),
                    }
                    Button {
                        label: "Open folder",
                        variant: ButtonVariant::Primary,
                        kbd_letter: Some("O".to_string()),
                        onclick: move |_| open_folder_browser(&mut state),
                    }
                }
            }
        }
    }
}

/// The Projects section. Filter text and status live in this screen's own
/// signals; everything else is read from `AppState`.
#[component]
pub fn ProjectsScreen() -> Element {
    let state = use_context::<AppState>();
    let query = use_context_provider(|| Signal::new(String::new()));
    let _rev = use_context_provider(|| Signal::new(0_u32));
    let filter = use_context_provider(|| Signal::new(ProjectFilter::All));

    let engines = state.engines.read();
    let containers = state.containers.read();
    let projects = state.projects.read();
    let pending = state.pending.read();
    let problems = state.problems.read();
    let images = state.images.peek();
    let volumes = state.volumes.peek();
    let networks = state.networks.peek();
    let stale = *state.snapshot_stale.read();

    let screen = screen_state(&engines, &projects, stale);
    // Sorted once, when live data first shows; kept while the screen is open (AD31).
    let frozen = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(FrozenOrder::default())));
    let ordered = frozen.borrow_mut().arrange(&projects, &engines, !stale);
    let counts = filter_counts(&ordered);
    let listing = list_projects(
        &ordered,
        &engines,
        &containers,
        &query.read(),
        *filter.read(),
    );
    let visible = listing.visible;
    let matched = listing.matched;
    let home = berth_sys::paths::home();
    let view = StateView {
        engines: &engines,
        containers: &containers,
        projects: &projects,
        images: &images,
        volumes: &volumes,
        networks: &networks,
        pending: &pending,
        backing: *state.backing.peek(),
    };
    let mut sorted_problems: Vec<berth_core::Problem> = problems.clone();
    sorted_problems.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(b.first_seen.cmp(&a.first_seen))
    });
    let rows: Vec<_> = visible
        .iter()
        .map(|p| {
            let mut model = row_model(p, &view, &sorted_problems, &home);
            model.problem_slot = frozen
                .borrow_mut()
                .slot(&model.id, model.problem_summary.is_some());
            model
        })
        .collect();
    let attention = attention_items(
        &problems,
        &containers,
        &projects,
        &engines,
        chrono::Utc::now(),
    );
    let stopped_engines: Vec<berth_core::Engine> = engines
        .iter()
        .filter(|e| e.state == berth_core::EngineState::Stopped)
        .filter(|e| projects.iter().any(|p| p.engine == e.id))
        .cloned()
        .collect();
    let total = real_count(&ordered);
    let row_count = rows.len();
    let rows_state = screen == ScreenState::Rows;

    rsx! {
        section { class: "projects-screen", "aria-label": "Projects",
            style { "{PROJECTS_CSS}" }
            if screen != ScreenState::Loading {
                Header { total, show_controls: rows_state, counts, matched }
            } else {
                header { class: "projects-header",
                    h1 { class: "projects-title", "Projects" }
                }
            }
            match screen {
                ScreenState::Loading => rsx! { LoadingBody {} },
                ScreenState::Empty => rsx! { EmptyBody {} },
                ScreenState::EngineStopped(engine) => rsx! { EngineStoppedBody { engine: *engine } },
                ScreenState::Unreachable => rsx! {
                    div { class: "projects-empty",
                        EmptyState {
                            title: "Cannot reach any engine".to_string(),
                            body: "Berth could not connect to a Docker engine. Check that Colima is installed, or add an engine."
                                .to_string(),
                            icon: "server",
                        }
                    }
                },
                ScreenState::Rows => rsx! {
                    if !stopped_engines.is_empty() {
                        div { class: "projects-banners",
                            for engine in stopped_engines {
                                EngineStoppedBanner { key: "{engine.id.0}", engine }
                            }
                        }
                    }
                    div { class: "projects-table", role: "table", "aria-rowcount": "{row_count}",
                        div { class: "projects-head", role: "row",
                            div { class: "projects-head-name table-header-cell", role: "columnheader",
                                span { class: "table-header-label", "Project" }
                            }
                            TableHeaderCell { label: "Path" }
                            TableHeaderCell { label: "Engine" }
                            TableHeaderCell { label: "Branch" }
                            TableHeaderCell { label: "Services" }
                            TableHeaderCell { label: "Ports" }
                            TableHeaderCell { label: "CPU", align: CellAlign::Right }
                            TableHeaderCell { label: "Memory", align: CellAlign::Right }
                            div { class: "projects-head-actions table-header-cell", role: "columnheader" }
                        }
                        if row_count == 0 {
                            div { class: "projects-no-match",
                                EmptyState {
                                    title: "No projects match".to_string(),
                                    body: "Try a different filter or status.".to_string(),
                                }
                            }
                        }
                        for model in rows {
                            ProjectRow { key: "{model.id.0}", model }
                        }
                    }
                    if !attention.is_empty() {
                        div { class: "projects-attention",
                            h2 { class: "projects-section-title",
                                "Needs attention \u{00b7} {attention.len()}"
                            }
                            div { class: "attention-list",
                                for (index, item) in attention.into_iter().enumerate() {
                                    AttentionCard { key: "{index}", item }
                                }
                            }
                        }
                    }
                    crate::screens::recent_activity::RecentActivity {}
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "projects_tests.rs"]
mod tests;
