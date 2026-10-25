//! Containers screen (P2-T7, brief §7.3): a grouped, sortable, virtual
//! table of every container in the engine scope, with filter chips, a
//! count-aware filter field, row and group buttons, J/K selection and the
//! 1024 px layout of D78.
//!
//! The view model below is pure (`build_list`, `step_selection`,
//! `list_status`, `port_url`, `open_command`): the screen reads signals,
//! asks the model what to show and sends clicks to the actions service.
//! It never calls an engine (D23).

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use dioxus::prelude::*;

use berth_core::{
    Container, ContainerAction, ContainerId, ContainerState, Engine, EngineKind, EngineState,
    PortMapping, Project, Proto, StatusKind, Subject,
};

use crate::actions::{action_by_id, mod_label, ActionId};
use crate::services::actions::{
    disabled_reason, dismiss, with_view, ActionsService, ContainerTarget, StateView,
};
use crate::services::confirm;
use crate::services::group_ops::{is_active, run as run_group_action, GroupIds};
use crate::services::stats::{use_row_stats, StatsKey, StatsService};
use crate::state::{AppState, EngineScope, PendingKey, PendingState, Section, Selection};
use crate::ui::bulk_bar::{applicable, bulk_counts};
use crate::ui::container_row::{
    ContainerRow, ContainerRowData, Meter, PortLink, RowAction, RowButton,
};
use crate::ui::{
    count_label, inspector_visible, scope_includes, Badge, BadgeKind, Banner, BannerTone,
    BulkAction, BulkBar, Button, ButtonVariant, CellAlign, ControlSize, EmptyState, GroupHeaderRow,
    IconButton, InlineMessage, InlineTone, Menu, MenuEntry, SearchField, Select, SelectOption,
    SkeletonRows, SortDirection, TableHeaderCell, VirtualTable,
};

/// The screen's CSS. Rendered with the screen (`style` element) so the
/// screen owns its styling; colours come only from `tokens.css`.
pub const CONTAINERS_CSS: &str = include_str!("../../assets/css/containers.css");

// ---------------------------------------------------------------------
// View state
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupBy {
    #[default]
    Project,
    Engine,
    None,
}

impl GroupBy {
    pub const ALL: [GroupBy; 3] = [GroupBy::Project, GroupBy::Engine, GroupBy::None];

    pub fn value(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Engine => "engine",
            Self::None => "none",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "engine" => Self::Engine,
            "none" => Self::None,
            _ => Self::Project,
        }
    }

    /// The text the select shows ("Group by: Project").
    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "Group by: Project",
            Self::Engine => "Group by: Engine",
            Self::None => "Group by: None",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Image,
    Ports,
    Cpu,
    Memory,
    Uptime,
    Status,
}

pub type Sort = Option<(SortColumn, SortDirection)>;

/// The state filter chip ("State: Stopped").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateFilter {
    Running,
    Stopped,
    Unhealthy,
}

impl StateFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Stopped => "Stopped",
            Self::Unhealthy => "Unhealthy",
        }
    }

    fn matches(self, c: &Container) -> bool {
        match self {
            Self::Running => c.state == ContainerState::Running,
            Self::Stopped => matches!(
                c.state,
                ContainerState::Exited | ContainerState::Created | ContainerState::Dead
            ),
            Self::Unhealthy => c.health == Some(berth_core::Health::Unhealthy),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filters {
    pub text: String,
    pub state: Option<StateFilter>,
    pub project: Option<String>,
}

impl Filters {
    pub fn is_active(&self) -> bool {
        !self.text.trim().is_empty() || self.state.is_some() || self.project.is_some()
    }
}

/// One removable chip under the header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chip {
    Engine(String),
    State(StateFilter),
    Project(String),
}

impl Chip {
    pub fn label(&self) -> String {
        match self {
            Self::Engine(name) => format!("Engine: {name}"),
            Self::State(state) => format!("State: {}", state.label()),
            Self::Project(name) => format!("Project: {name}"),
        }
    }
}

pub fn chips(engines: &[Engine], scope: &EngineScope, filters: &Filters) -> Vec<Chip> {
    let mut out = Vec::new();
    if let EngineScope::One(id) = scope {
        let name = engines
            .iter()
            .find(|e| e.id == *id)
            .map_or_else(|| id.0.clone(), |e| e.name.clone());
        out.push(Chip::Engine(name));
    }
    if let Some(state) = filters.state {
        out.push(Chip::State(state));
    }
    if let Some(project) = &filters.project {
        out.push(Chip::Project(project.clone()));
    }
    out
}

/// Everything the screen keeps besides the app state: filter, grouping,
/// sort, group collapse overrides, the inspector toggle (Space) and the
/// open ⋯ menu. One value is provided from the App root so the root key
/// handler can reach it (`dispatch` below); the screen creates its own
/// when none is provided (tests).
#[derive(Clone, Copy)]
pub struct ContainersView {
    pub filters: Signal<Filters>,
    pub group_by: Signal<GroupBy>,
    pub sort: Signal<Sort>,
    /// Group key to "collapsed". A group with no entry is collapsed when
    /// nothing in it is running.
    pub collapsed: Signal<BTreeMap<String, bool>>,
    /// Parity of Space presses. At 1200 px and wider it hides the docked
    /// inspector; below, it shows it as an overlay (`inspector-open`).
    pub inspector_toggled: Signal<bool>,
    /// Bumped to remount the (uncontrolled) filter field when cleared.
    pub filter_epoch: Signal<u32>,
    pub menu_open: Signal<Option<ContainerId>>,
    /// Key of the group whose ⋯ menu is open.
    pub group_menu: Signal<Option<String>>,
}

impl ContainersView {
    pub fn new() -> Self {
        Self {
            filters: Signal::new(Filters::default()),
            group_by: Signal::new(GroupBy::default()),
            sort: Signal::new(None),
            collapsed: Signal::new(BTreeMap::new()),
            inspector_toggled: Signal::new(false),
            filter_epoch: Signal::new(0),
            menu_open: Signal::new(None),
            group_menu: Signal::new(None),
        }
    }
}

impl Default for ContainersView {
    fn default() -> Self {
        Self::new()
    }
}

/// Provide the view from the App root (call once, in a component body).
pub fn provide_containers_view() -> ContainersView {
    use_hook(|| provide_context(ContainersView::new()))
}

/// Has Space toggled the inspector? Read by the shell; `false` without a
/// provided view.
pub fn inspector_toggled() -> bool {
    try_consume_context::<ContainersView>().is_some_and(|view| *view.inspector_toggled.read())
}

// ---------------------------------------------------------------------
// List status: loading, engine stopped, ready
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ListStatus {
    /// No engine has answered yet: skeleton rows.
    Loading,
    /// The scoped engine is stopped: banner and the Start engine empty state.
    EngineStopped(Box<Engine>),
    Ready,
}

pub fn list_status(
    engines: &[Engine],
    scope: &EngineScope,
    containers: &[Container],
    snapshot_stale: bool,
) -> ListStatus {
    if engines.is_empty() {
        return ListStatus::Loading;
    }
    let scoped: Vec<&Engine> = engines
        .iter()
        .filter(|e| scope_includes(scope, &e.id))
        .collect();
    if let EngineScope::One(_) = scope {
        if let Some(engine) = scoped.iter().find(|e| e.state == EngineState::Stopped) {
            return ListStatus::EngineStopped(Box::new((*engine).clone()));
        }
    }
    let has_rows = containers.iter().any(|c| scope_includes(scope, &c.engine));
    if has_rows || snapshot_stale {
        return ListStatus::Ready;
    }
    let waiting = scoped.iter().any(|e| match e.state {
        EngineState::Starting => true,
        // The first answer sets the Docker version (services::engines).
        EngineState::Running => e.docker_version.is_none(),
        _ => false,
    });
    if waiting {
        ListStatus::Loading
    } else {
        ListStatus::Ready
    }
}

/// One banner per engine in scope that is not running, worded for its state.
pub fn engine_banners(engines: &[Engine], scope: &EngineScope) -> Vec<(BannerTone, String)> {
    engines
        .iter()
        .filter(|e| scope_includes(scope, &e.id))
        .filter_map(|e| {
            let kind = match &e.kind {
                EngineKind::Colima { .. } => format!("Colima {}", e.name),
                _ => e.name.clone(),
            };
            match &e.state {
                EngineState::Running => None,
                EngineState::Stopped => Some((
                    BannerTone::Warning,
                    format!("{kind} is stopped. Containers on it are not running."),
                )),
                EngineState::Starting => {
                    Some((BannerTone::Info, format!("{kind} is starting\u{2026}")))
                }
                EngineState::Stopping => {
                    Some((BannerTone::Info, format!("{kind} is stopping\u{2026}")))
                }
                EngineState::Unreachable { reason, .. } => Some((
                    BannerTone::Danger,
                    format!("{kind} is unreachable. {reason}"),
                )),
                EngineState::Unsupported { reason } => Some((BannerTone::Warning, reason.clone())),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------
// The list model (pure)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct GroupInfo {
    pub key: String,
    pub name: String,
    /// "3 running · 1 restarting · 1 exited"
    pub counts: String,
    pub expanded: bool,
    /// Stopped members ("Start 1 stopped" starts these).
    pub start_ids: Vec<ContainerId>,
    /// Running, restarting or paused members ("Stop all", ↻ restart).
    pub stop_ids: Vec<ContainerId>,
}

impl GroupInfo {
    /// The group's one text button, as in the frame.
    pub fn action_label(&self) -> Option<String> {
        if !self.start_ids.is_empty() {
            Some(format!("Start {} stopped", self.start_ids.len()))
        } else if !self.stop_ids.is_empty() {
            Some("Stop all".to_string())
        } else {
            None
        }
    }

    /// Why the restart button cannot run: it restarts running members.
    pub fn restart_reason(&self) -> Option<&'static str> {
        self.stop_ids.is_empty().then_some("Nothing is running")
    }

    /// The group's ⋯ menu: the group actions the Actions service runs,
    /// each disabled with its reason when no member qualifies.
    pub fn menu_items(&self) -> Vec<MenuEntry> {
        let entry = |id: &str, label: String, icon: &'static str, reason: Option<&str>| {
            let entry = MenuEntry::new(id, label).icon(icon);
            match reason {
                Some(reason) => entry.disabled(reason),
                None => entry,
            }
        };
        let none_stopped = self.start_ids.is_empty().then_some("Nothing is stopped");
        let none_running = self.restart_reason();
        vec![
            entry(
                "start-stopped",
                format!("Start {} stopped", self.start_ids.len()),
                "play",
                none_stopped,
            ),
            entry(
                "stop-all",
                "Stop all running".to_string(),
                "square",
                none_running,
            ),
            entry(
                "restart-running",
                "Restart running".to_string(),
                "rotate-cw",
                none_running,
            ),
        ]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ListRow {
    Group(GroupInfo),
    Container(ContainerId),
    /// The failed-action message under a container row (D34).
    Failure(ContainerId),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListModel {
    /// Containers in the engine scope, before any filter.
    pub total: usize,
    /// Containers left after the filters.
    pub matched: usize,
    pub rows: Vec<ListRow>,
}

impl ListModel {
    /// The containers in table order, collapsed groups excluded: what J and
    /// K step through.
    pub fn selection_order(&self) -> Vec<ContainerId> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                ListRow::Container(id) => Some(id.clone()),
                _ => None,
            })
            .collect()
    }
}

pub struct ListInput<'a> {
    pub containers: &'a [Container],
    pub projects: &'a [Project],
    pub engines: &'a [Engine],
    pub scope: &'a EngineScope,
    pub filters: &'a Filters,
    pub group_by: GroupBy,
    pub sort: Sort,
    pub collapsed: &'a BTreeMap<String, bool>,
    pub pending: &'a BTreeMap<PendingKey, PendingState>,
}

fn status_of(c: &Container) -> StatusKind {
    berth_core::status_of(c.state, c.health, c.exit_code)
}

fn engine_name<'a>(engines: &'a [Engine], id: &'a berth_core::EngineId) -> &'a str {
    engines
        .iter()
        .find(|e| e.id == *id)
        .map_or(id.0.as_str(), |e| e.name.as_str())
}

/// The project a container belongs to: its own link, else the project
/// listing it as a service container.
fn project_of<'a>(
    c: &Container,
    by_container: &HashMap<&ContainerId, &'a Project>,
    projects: &'a [Project],
) -> Option<&'a Project> {
    if let Some(project) = by_container.get(&c.id) {
        return Some(project);
    }
    let id = c.project.as_ref()?;
    projects.iter().find(|p| p.id == *id)
}

fn matches_text(c: &Container, project: Option<&str>, engine: &str, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let mut hay = String::new();
    for field in [
        c.name.as_str(),
        c.image.reference.as_str(),
        project.unwrap_or(""),
        c.service.as_deref().unwrap_or(""),
        engine,
    ] {
        hay.push_str(&field.to_lowercase());
        hay.push('\n');
    }
    hay.push_str(&status_of(c).label().to_lowercase());
    hay.push('\n');
    for port in &c.ports {
        hay.push_str(&berth_core::fmt::port(port));
        hay.push('\n');
    }
    hay.push_str(&c.id.0.chars().take(12).collect::<String>());
    terms.iter().all(|term| hay.contains(term.as_str()))
}

fn cmp_opt<T: PartialOrd>(a: Option<T>, b: Option<T>, dir: SortDirection) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            let ord = a.partial_cmp(&b).unwrap_or(Ordering::Equal);
            if dir == SortDirection::Desc {
                ord.reverse()
            } else {
                ord
            }
        }
        // Rows without the value stay last in both directions.
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn first_host_port(c: &Container) -> Option<u16> {
    c.ports.iter().filter_map(|p| p.host_port).min()
}

fn compare(a: &Container, b: &Container, column: SortColumn, dir: SortDirection) -> Ordering {
    let ordered = |ord: Ordering| {
        if dir == SortDirection::Desc {
            ord.reverse()
        } else {
            ord
        }
    };
    match column {
        SortColumn::Name => ordered(a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        SortColumn::Image => ordered(
            a.image
                .reference
                .to_lowercase()
                .cmp(&b.image.reference.to_lowercase()),
        ),
        SortColumn::Ports => cmp_opt(first_host_port(a), first_host_port(b), dir),
        SortColumn::Cpu => cmp_opt(
            a.stats.as_ref().map(|s| s.cpu_pct),
            b.stats.as_ref().map(|s| s.cpu_pct),
            dir,
        ),
        SortColumn::Memory => cmp_opt(
            a.stats.as_ref().map(|s| s.mem_used.as_u64()),
            b.stats.as_ref().map(|s| s.mem_used.as_u64()),
            dir,
        ),
        // Ascending is the shortest uptime first, i.e. the latest start.
        SortColumn::Uptime => cmp_opt(
            a.started.map(|t| -t.timestamp()),
            b.started.map(|t| -t.timestamp()),
            dir,
        ),
        SortColumn::Status => ordered(status_of(a).label().cmp(&status_of(b).label())),
    }
}

/// The first failed action message under a container, with its key.
pub fn failure_of(
    pending: &BTreeMap<PendingKey, PendingState>,
    id: &ContainerId,
) -> Option<(PendingKey, String)> {
    pending
        .iter()
        .find_map(|(key, value)| match (&key.subject, value) {
            (Subject::Container(c), PendingState::Failed { message }) if c == id => {
                Some((key.clone(), message.clone()))
            }
            _ => None,
        })
}

fn group_info(
    key: String,
    name: String,
    members: &[&Container],
    collapsed: &BTreeMap<String, bool>,
) -> GroupInfo {
    let count = |f: &dyn Fn(&Container) -> bool| members.iter().filter(|c| f(c)).count();
    let counts = count_label(&[
        (count(&|c| c.state == ContainerState::Running), "running"),
        (
            count(&|c| c.state == ContainerState::Restarting),
            "restarting",
        ),
        (count(&|c| c.state == ContainerState::Paused), "paused"),
        (count(&|c| c.state == ContainerState::Created), "created"),
        (count(&|c| c.state == ContainerState::Exited), "exited"),
        (count(&|c| c.state == ContainerState::Dead), "dead"),
        (count(&|c| c.state == ContainerState::Removing), "removing"),
    ]);
    let ids = GroupIds::of(members.iter().copied());
    let default_collapsed = ids.stop_ids.is_empty();
    let expanded = !collapsed.get(&key).copied().unwrap_or(default_collapsed);
    GroupInfo {
        key,
        name,
        counts,
        expanded,
        start_ids: ids.start_ids,
        stop_ids: ids.stop_ids,
    }
}

pub fn build_list(input: &ListInput<'_>) -> ListModel {
    let by_container: HashMap<&ContainerId, &Project> = input
        .projects
        .iter()
        .flat_map(|p| {
            p.services
                .iter()
                .flat_map(|s| s.containers.iter())
                .map(move |id| (id, p))
        })
        .collect();
    let terms: Vec<String> = input
        .filters
        .text
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let scoped: Vec<&Container> = input
        .containers
        .iter()
        .filter(|c| scope_includes(input.scope, &c.engine))
        .collect();
    let total = scoped.len();
    let mut kept: Vec<(&Container, Option<&Project>)> = scoped
        .into_iter()
        .map(|c| (c, project_of(c, &by_container, input.projects)))
        .filter(|(c, project)| {
            input.filters.state.is_none_or(|s| s.matches(c))
                && input
                    .filters
                    .project
                    .as_deref()
                    .is_none_or(|name| project.is_some_and(|p| p.name == name))
                && matches_text(
                    c,
                    project.map(|p| p.name.as_str()),
                    engine_name(input.engines, &c.engine),
                    &terms,
                )
        })
        .collect();
    let matched = kept.len();
    if let Some((column, dir)) = input.sort {
        kept.sort_by(|a, b| compare(a.0, b.0, column, dir));
    }

    // Group, keeping first-seen order stable per the group order below.
    let mut groups: Vec<(String, String, Vec<&Container>)> = Vec::new();
    match input.group_by {
        GroupBy::None => {}
        GroupBy::Project => {
            for project in input.projects {
                groups.push((
                    format!("project:{}", project.id.0),
                    project.name.clone(),
                    vec![],
                ));
            }
            groups.push(("unassigned".to_string(), "Unassigned".to_string(), vec![]));
        }
        GroupBy::Engine => {
            for engine in input.engines {
                groups.push((
                    format!("engine:{}", engine.id.0),
                    engine.name.clone(),
                    vec![],
                ));
            }
        }
    }
    let mut rows = Vec::new();
    if input.group_by == GroupBy::None {
        for (c, _) in &kept {
            push_container(&mut rows, c, input.pending);
        }
    } else {
        for (c, project) in &kept {
            let key = match input.group_by {
                GroupBy::Project => project.map_or_else(
                    || "unassigned".to_string(),
                    |p| format!("project:{}", p.id.0),
                ),
                _ => format!("engine:{}", c.engine.0),
            };
            match groups.iter_mut().find(|g| g.0 == key) {
                Some(group) => group.2.push(c),
                None => groups.push((
                    key,
                    engine_name(input.engines, &c.engine).to_string(),
                    vec![*c],
                )),
            }
        }
        for (key, name, members) in groups {
            if members.is_empty() {
                continue;
            }
            let info = group_info(key, name, &members, input.collapsed);
            let expanded = info.expanded;
            rows.push(ListRow::Group(info));
            if expanded {
                for c in members {
                    push_container(&mut rows, c, input.pending);
                }
            }
        }
    }
    ListModel {
        total,
        matched,
        rows,
    }
}

fn push_container(
    rows: &mut Vec<ListRow>,
    c: &Container,
    pending: &BTreeMap<PendingKey, PendingState>,
) {
    rows.push(ListRow::Container(c.id.clone()));
    if failure_of(pending, &c.id).is_some() {
        rows.push(ListRow::Failure(c.id.clone()));
    }
}

// ---------------------------------------------------------------------
// Selection stepping, ports (pure)
// ---------------------------------------------------------------------

/// The container J (`forward`) or K selects next. At the ends the
/// selection stays; with no selection J picks the first row and K the
/// last. `None` when there is nothing to select.
pub fn step_selection(
    order: &[ContainerId],
    current: &Selection,
    forward: bool,
) -> Option<ContainerId> {
    if order.is_empty() {
        return None;
    }
    let anchor = match current {
        Selection::Containers(ids) => ids.last().and_then(|id| order.iter().position(|o| o == id)),
        _ => None,
    };
    let index = match (anchor, forward) {
        (Some(i), true) => (i + 1).min(order.len() - 1),
        (Some(i), false) => i.saturating_sub(1),
        (None, true) => 0,
        (None, false) => order.len() - 1,
    };
    order.get(index).cloned()
}

/// The URL a published port opens. Only TCP ports with a host port link.
pub fn port_url(port: &PortMapping) -> Option<String> {
    match (port.proto, port.host_port) {
        (Proto::Tcp, Some(host)) => Some(format!("http://localhost:{host}")),
        _ => None,
    }
}

/// The program and arguments that open `url` in the default browser.
pub fn open_command(url: &str) -> (&'static str, Vec<String>) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    (program, vec![url.to_string()])
}

fn open_in_browser(host_port: u16) {
    let url = format!("http://localhost:{host_port}");
    let (program, args) = open_command(&url);
    spawn(async move {
        let search = berth_sys::SearchPath::from_process();
        let Some(path) = search.find(program) else {
            tracing::warn!(program, "cannot open a link: program not found");
            return;
        };
        let args: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
        if let Err(err) = berth_sys::run_capture(
            &path,
            &args,
            berth_sys::SpawnOpts::default(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        {
            tracing::warn!(%err, "opening a link failed");
        }
    });
}

// ---------------------------------------------------------------------
// Row data (pure over borrowed state)
// ---------------------------------------------------------------------

const MAX_PORT_LINKS: usize = 1;

/// "Restart · ⌘R": the action's label and its shortcut, as the palette
/// and menus show them.
pub fn tooltip_for(label: &str, action: ActionId) -> String {
    match action_by_id(action).and_then(|a| a.shortcut) {
        Some(shortcut) => format!("{label} \u{00b7} {}", shortcut.replace('⌘', mod_label())),
        None => label.to_string(),
    }
}

/// Fraction of the CPU bar filled: square-root scaled so small loads show.
pub fn cpu_fraction(pct: f32) -> f32 {
    (pct / 100.0).clamp(0.0, 1.0).sqrt()
}

fn running_pending<'a>(
    pending: &'a BTreeMap<PendingKey, PendingState>,
    id: &ContainerId,
) -> Option<(&'a PendingKey, &'a str)> {
    pending
        .iter()
        .find_map(|(key, value)| match (&key.subject, value) {
            (Subject::Container(c), PendingState::Running { label }) if c == id => {
                Some((key, label.as_str()))
            }
            _ => None,
        })
}

fn row_buttons(c: &Container, view: &StateView<'_>) -> Vec<RowButton> {
    let subject = Subject::Container(c.id.clone());
    let active = is_active(c.state);
    let (toggle_label, toggle_icon, toggle_id) = if active {
        ("Stop", "square", "container.stop")
    } else {
        ("Start", "play", "container.start")
    };
    let pending_action = running_pending(view.pending, &c.id).map(|(key, _)| key.action);
    let button = |action: RowAction,
                  icon: &'static str,
                  label: String,
                  tooltip: String,
                  reason: Option<String>,
                  loading: bool| RowButton {
        action,
        icon,
        label,
        disabled: reason.is_some(),
        tooltip: reason.unwrap_or(tooltip),
        loading,
    };
    let toggle_reason = disabled_reason(&subject, toggle_id, view);
    let restart_reason = disabled_reason(&subject, "container.restart", view);
    let delete_reason = disabled_reason(&subject, "container.delete", view);
    vec![
        button(
            RowAction::StartStop,
            toggle_icon,
            format!("{toggle_label} {}", c.name),
            tooltip_for(toggle_label, "container.start_stop"),
            toggle_reason,
            matches!(pending_action, Some("container.start" | "container.stop")),
        ),
        button(
            RowAction::Restart,
            "rotate-cw",
            format!("Restart {}", c.name),
            tooltip_for("Restart", "container.restart"),
            restart_reason,
            pending_action == Some("container.restart"),
        ),
        button(
            RowAction::Logs,
            "logs",
            format!("Logs for {}", c.name),
            tooltip_for("Logs", "container.logs"),
            None,
            false,
        ),
        button(
            RowAction::Terminal,
            "terminal",
            format!("Terminal for {}", c.name),
            tooltip_for(
                if is_active(c.state) {
                    "Terminal"
                } else {
                    "Start and open terminal"
                },
                "container.terminal",
            ),
            crate::services::exec::terminal_disabled_reason(c.state),
            false,
        ),
        button(
            RowAction::Delete,
            "trash-2",
            format!("Delete {}", c.name),
            tooltip_for("Delete", "container.delete"),
            delete_reason,
            pending_action == Some("container.delete"),
        ),
        button(
            RowAction::More,
            "more-horizontal",
            format!("More actions for {}", c.name),
            "More".to_string(),
            None,
            false,
        ),
    ]
}

/// The ⋯ menu entries for a container.
pub fn row_menu(c: &Container, view: &StateView<'_>) -> Vec<MenuEntry> {
    let subject = Subject::Container(c.id.clone());
    let pause = if c.state == ContainerState::Paused {
        let entry = MenuEntry::new("resume", "Resume").icon("play");
        match disabled_reason(&subject, "container.resume", view) {
            Some(reason) => entry.disabled(reason),
            None => entry,
        }
    } else {
        let entry = MenuEntry::new("pause", "Pause").icon("pause");
        match disabled_reason(&subject, "container.pause", view) {
            Some(reason) => entry.disabled(reason),
            None => entry,
        }
    };
    vec![
        pause,
        MenuEntry::new("copy-id", "Copy container ID")
            .icon("copy")
            .separator_before(),
    ]
}

pub fn row_data(
    c: &Container,
    view: &StateView<'_>,
    engine: &str,
    now: chrono::DateTime<chrono::Utc>,
    menu_open: bool,
) -> ContainerRowData {
    let status = status_of(c);
    let status_text = running_pending(view.pending, &c.id)
        .map_or_else(|| status.label(), |(_, label)| label.to_string());
    // Ports are bound only while the container runs or is paused; an
    // exited, created or restarting row shows "—" like the frames.
    let bound = matches!(c.state, ContainerState::Running | ContainerState::Paused);
    let shown_ports: &[PortMapping] = if bound { &c.ports } else { &[] };
    let published: Vec<&PortMapping> = shown_ports
        .iter()
        .filter(|p| p.host_port.is_some())
        .collect();
    let unpublished = shown_ports.len() - published.len();
    let ports: Vec<PortLink> = published
        .iter()
        .take(MAX_PORT_LINKS)
        .map(|p| PortLink {
            label: berth_core::fmt::port(p),
            host_port: port_url(p).and(p.host_port),
        })
        .collect();
    let extra_ports = published.len().saturating_sub(MAX_PORT_LINKS) + unpublished;
    let cpu = c.stats.as_ref().map(|s| Meter {
        text: format!("{:.1}%", s.cpu_pct),
        fraction: cpu_fraction(s.cpu_pct),
    });
    let memory = c.stats.as_ref().map(|s| Meter {
        text: berth_core::fmt::bytes(s.mem_used),
        fraction: if s.mem_limit.as_u64() == 0 {
            0.0
        } else {
            #[allow(clippy::cast_precision_loss)]
            let f = s.mem_used.as_u64() as f32 / s.mem_limit.as_u64() as f32;
            f.clamp(0.0, 1.0)
        },
    });
    // A paused container still has an age (frame: api-worker "1 d").
    let uptime = if matches!(c.state, ContainerState::Running | ContainerState::Paused) {
        c.started.map(|t| berth_core::fmt::uptime(t, now))
    } else {
        None
    };
    ContainerRowData {
        id: c.id.clone(),
        name: c.name.clone(),
        image: c.image.reference.clone(),
        status,
        status_text,
        ports,
        extra_ports,
        cpu,
        memory,
        uptime,
        engine: engine.to_string(),
        buttons: row_buttons(c, view),
        menu: row_menu(c, view),
        menu_open,
    }
}

// ---------------------------------------------------------------------
// Effects (events -> state / actions service)
// ---------------------------------------------------------------------

/// Row click: this container alone is selected.
pub fn select_only(state: &mut AppState, id: &ContainerId) {
    state.selection.set(Selection::Containers(vec![id.clone()]));
}

/// Checkbox: add or remove this container from the selection.
pub fn toggle_selected(state: &mut AppState, id: &ContainerId) {
    let mut ids = match &*state.selection.peek() {
        Selection::Containers(ids) => ids.clone(),
        _ => Vec::new(),
    };
    if let Some(at) = ids.iter().position(|i| i == id) {
        ids.remove(at);
    } else {
        ids.push(id.clone());
    }
    state.selection.set(if ids.is_empty() {
        Selection::None
    } else {
        Selection::Containers(ids)
    });
}

fn target_of(state: &AppState, id: &ContainerId) -> Option<(ContainerTarget, ContainerState)> {
    state
        .containers
        .peek()
        .iter()
        .find(|c| c.id == *id)
        .map(|c| (ContainerTarget::from(c), c.state))
}

fn run_container_action(
    state: &mut AppState,
    actions: Option<&ActionsService>,
    id: &ContainerId,
    action: &RowAction,
) {
    if *action == RowAction::Delete {
        // Delete always asks first (D33); the dialog runs it.
        confirm::open_delete(state, std::slice::from_ref(id));
        return;
    }
    let Some(service) = actions else { return };
    let Some((target, container_state)) = target_of(state, id) else {
        return;
    };
    let command = match action {
        RowAction::StartStop if is_active(container_state) => {
            ContainerAction::Stop { timeout_s: None }
        }
        RowAction::StartStop => ContainerAction::Start,
        RowAction::Restart => ContainerAction::Restart { timeout_s: None },
        _ => return,
    };
    if let Err(reason) = service.container(state, &target, command) {
        tracing::debug!(%reason, "row action refused");
    }
}

fn copy_to_clipboard(text: &str) {
    let quoted = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
    let script = format!("navigator.clipboard.writeText({quoted});");
    spawn(async move {
        let _ = document::eval(&script).await;
    });
}

/// Keys that reach the screen: J/K move the selection through the visible
/// rows, Space toggles the inspector. Called from `keys::dispatch`; a
/// no-op without a provided `ContainersView`.
pub fn dispatch(state: &mut AppState, action: ActionId) {
    // A dialog over the table owns the selection (bulk delete reads it).
    if matches!(action, "global.select_next" | "global.select_prev")
        && !state.overlays.peek().is_empty()
    {
        return;
    }
    let Some(view) = try_consume_context::<ContainersView>() else {
        return;
    };
    apply_nav(state, &view, action);
}

fn apply_nav(state: &mut AppState, view: &ContainersView, action: ActionId) {
    match action {
        "global.toggle_inspector" => {
            let mut toggled = view.inspector_toggled;
            let next = !*toggled.peek();
            toggled.set(next);
        }
        "global.select_next" | "global.select_prev" => {
            if *state.section.peek() != Section::Containers {
                return;
            }
            let model = {
                let containers = state.containers.peek();
                let projects = state.projects.peek();
                let engines = state.engines.peek();
                let scope = state.engine_scope.peek();
                let filters = view.filters.peek();
                let collapsed = view.collapsed.peek();
                let pending = state.pending.peek();
                build_list(&ListInput {
                    containers: &containers,
                    projects: &projects,
                    engines: &engines,
                    scope: &scope,
                    filters: &filters,
                    group_by: *view.group_by.peek(),
                    sort: *view.sort.peek(),
                    collapsed: &collapsed,
                    pending: &pending,
                })
            };
            let next = step_selection(
                &model.selection_order(),
                &state.selection.peek(),
                action == "global.select_next",
            );
            if let Some(id) = next {
                select_only(state, &id);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------

fn state_view_from<'a>(
    state: &'a AppState,
    engines: &'a [Engine],
    containers: &'a [Container],
    projects: &'a [Project],
    pending: &'a BTreeMap<PendingKey, PendingState>,
) -> StateView<'a> {
    // Images, volumes and networks never change a container's reasons, so
    // they are peeked (no subscription) and left empty.
    let _ = state;
    StateView {
        engines,
        containers,
        projects,
        images: &[],
        volumes: &[],
        networks: &[],
        pending,
    }
}

/// The listed container with the service's newest fresh sample in place of
/// its own `stats` (the list never carries live ones). Without a service,
/// or without a fresh sample, the listed value stands: a scene's frame
/// values, or "—".
fn with_live_stats<'a>(
    listed: &'a Container,
    stats: Option<&StatsService>,
    key: Option<&StatsKey>,
) -> std::borrow::Cow<'a, Container> {
    let live = stats
        .zip(key)
        .and_then(|(service, key)| service.latest(key, chrono::Utc::now()));
    match live {
        Some(live) => std::borrow::Cow::Owned(Container {
            stats: Some(live),
            ..listed.clone()
        }),
        None => std::borrow::Cow::Borrowed(listed),
    }
}

#[component]
fn ContainerRowHost(id: ContainerId, show_uptime: bool, show_engine: bool) -> Element {
    let mut state = use_context::<AppState>();
    let view = use_context::<ContainersView>();
    let actions = try_consume_context::<ActionsService>();
    let stats = try_consume_context::<StatsService>();
    let containers = state.containers.read();
    let listed = containers.iter().find(|c| c.id == id);
    // D26: this row holds a stats stream open while it is rendered (inside
    // the table's visible range) and the container runs. The hook runs
    // before the early return so hook order never changes.
    let key = listed
        .filter(|c| matches!(c.state, ContainerState::Running | ContainerState::Paused))
        .map(|c| StatsKey::new(&c.engine, &c.id));
    use_row_stats(stats.as_ref(), key.clone());
    let Some(listed) = listed else {
        return rsx! {};
    };
    let container = &*with_live_stats(listed, stats.as_ref(), key.as_ref());
    let engines = state.engines.read();
    let projects = state.projects.read();
    let pending = state.pending.read();
    let selected =
        matches!(&*state.selection.read(), Selection::Containers(ids) if ids.contains(&id));
    let menu_open = view.menu_open.read().as_ref() == Some(&id);
    let data = {
        let view_state = state_view_from(&state, &engines, &containers, &projects, &pending);
        row_data(
            container,
            &view_state,
            engine_name(&engines, &container.engine),
            chrono::Utc::now(),
            menu_open,
        )
    };
    let pause_action = if container.state == ContainerState::Paused {
        ContainerAction::Unpause
    } else {
        ContainerAction::Pause
    };
    let menu_actions = actions.clone();
    rsx! {
        ContainerRow {
            data,
            selected,
            show_uptime,
            show_engine,
            onselect: move |id: ContainerId| {
                let mut menu = view.menu_open;
                menu.set(None);
                select_only(&mut state, &id);
            },
            ontoggle: move |id: ContainerId| toggle_selected(&mut state, &id),
            onaction: move |(id, action): (ContainerId, RowAction)| match action {
                RowAction::More => {
                    let mut group_menu = view.group_menu;
                    group_menu.set(None);
                    let mut menu = view.menu_open;
                    let open = menu.peek().as_ref() == Some(&id);
                    menu.set(if open { None } else { Some(id) });
                }
                RowAction::Logs => {
                    select_only(&mut state, &id);
                    crate::keys::dispatch(&mut state, "container.logs");
                }
                RowAction::Terminal => {
                    select_only(&mut state, &id);
                    crate::keys::dispatch(&mut state, "container.terminal");
                }
                other => run_container_action(&mut state, actions.as_ref(), &id, &other),
            },
            onport: move |port: u16| open_in_browser(port),
            onmenu: move |(id, item): (ContainerId, String)| {
                let mut menu = view.menu_open;
                menu.set(None);
                match item.as_str() {
                    "pause" | "resume" => {
                        let Some(service) = menu_actions.as_ref() else { return };
                        if let Some((target, _)) = target_of(&state, &id) {
                            let _ = service.container(&mut state, &target, pause_action.clone());
                        }
                    }
                    "copy-id" => copy_to_clipboard(&id.0),
                    _ => {}
                }
            },
        }
    }
}

#[component]
fn FailureRow(id: ContainerId) -> Element {
    let mut state = use_context::<AppState>();
    let failure = failure_of(&state.pending.read(), &id);
    let Some((key, message)) = failure else {
        return rsx! {};
    };
    rsx! {
        div { class: "ct-failure",
            InlineMessage { message, tone: InlineTone::Danger }
            button {
                class: "ct-failure-dismiss",
                r#type: "button",
                "aria-label": "Dismiss message",
                onclick: move |_| dismiss(&mut state, &key),
                "Dismiss"
            }
        }
    }
}

#[component]
fn GroupRowHost(info: GroupInfo) -> Element {
    let mut state = use_context::<AppState>();
    let view = use_context::<ContainersView>();
    let actions = try_consume_context::<ActionsService>();
    let toggle_key = info.key.clone();
    let expanded = info.expanded;
    let start_ids = info.start_ids.clone();
    let stop_ids = info.stop_ids.clone();
    let restart_ids = info.stop_ids.clone();
    let group_actions = actions.clone();
    let restart_actions = actions.clone();
    let menu_actions = actions;
    let restart_label = format!("Restart running containers in {}", info.name);
    let restart_reason = info.restart_reason();
    let menu_open = view.group_menu.read().as_deref() == Some(info.key.as_str());
    let menu_key = info.key.clone();
    let menu_items = info.menu_items();
    let more_label = format!("More actions for {}", info.name);
    let menu_label = more_label.clone();
    let menu_start = info.start_ids.clone();
    let menu_stop = info.stop_ids.clone();
    rsx! {
        GroupHeaderRow {
            name: info.name.clone(),
            counts: info.counts.clone(),
            expanded,
            action_label: info.action_label(),
            ontoggle: move |()| {
                let mut collapsed = view.collapsed;
                collapsed.write().insert(toggle_key.clone(), expanded);
            },
            onaction: move |()| {
                if start_ids.is_empty() {
                    run_group_action(
                        &mut state,
                        group_actions.as_ref(),
                        &stop_ids,
                        &ContainerAction::Stop { timeout_s: None },
                    );
                } else {
                    run_group_action(
                        &mut state,
                        group_actions.as_ref(),
                        &start_ids,
                        &ContainerAction::Start,
                    );
                }
            },
            IconButton {
                icon: "rotate-cw",
                label: restart_label,
                tooltip: restart_reason.unwrap_or("Restart running").to_string(),
                size: ControlSize::Sm,
                disabled: restart_reason.is_some(),
                onclick: move |_| {
                    run_group_action(
                        &mut state,
                        restart_actions.as_ref(),
                        &restart_ids,
                        &ContainerAction::Restart { timeout_s: None },
                    );
                },
            }
            span { class: "group-header-more",
                IconButton {
                    icon: "more-horizontal",
                    label: more_label,
                    tooltip: "More".to_string(),
                    size: ControlSize::Sm,
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        let mut open = view.group_menu;
                        let was_open = open.peek().as_deref() == Some(menu_key.as_str());
                        open.set(if was_open { None } else { Some(menu_key.clone()) });
                        let mut row_menu = view.menu_open;
                        row_menu.set(None);
                    },
                }
                if menu_open {
                    div {
                        class: "group-menu",
                        onclick: move |e| e.stop_propagation(),
                        Menu {
                            items: menu_items,
                            aria_label: menu_label,
                            on_select: move |item: String| {
                                let mut open = view.group_menu;
                                open.set(None);
                                let (ids, action) = match item.as_str() {
                                    "start-stopped" => (&menu_start, ContainerAction::Start),
                                    "stop-all" => {
                                        (&menu_stop, ContainerAction::Stop { timeout_s: None })
                                    }
                                    "restart-running" => {
                                        (&menu_stop, ContainerAction::Restart { timeout_s: None })
                                    }
                                    _ => return,
                                };
                                run_group_action(&mut state, menu_actions.as_ref(), ids, &action);
                            },
                        }
                    }
                }
            }
        }
    }
}

/// The bulk bar over the current selection (two or more rows): counts per
/// action from the same `disabled_reason` the row buttons use; Delete asks
/// first through the confirm dialog (D33).
#[component]
fn BulkBarHost(ids: Vec<ContainerId>) -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_consume_context::<ActionsService>();
    let counts = {
        let containers = state.containers.read();
        let engines = state.engines.read();
        let projects = state.projects.read();
        let pending = state.pending.read();
        let view = state_view_from(&state, &engines, &containers, &projects, &pending);
        bulk_counts(&ids, &view)
    };
    let selected = ids.len();
    let run_ids = ids.clone();
    rsx! {
        BulkBar {
            selected,
            counts,
            onaction: move |action: BulkAction| {
                let targets = with_view(&state, |view| applicable(&run_ids, action, view));
                let command = match action {
                    BulkAction::Delete => {
                        confirm::open_delete(&mut state, &targets);
                        return;
                    }
                    BulkAction::Start => ContainerAction::Start,
                    BulkAction::Stop => ContainerAction::Stop { timeout_s: None },
                    BulkAction::Restart => ContainerAction::Restart { timeout_s: None },
                    BulkAction::Pause => ContainerAction::Pause,
                };
                run_group_action(&mut state, actions.as_ref(), &targets, &command);
            },
            onclear: move |()| state.selection.set(Selection::None),
        }
    }
}

#[component]
fn FilterChip(chip: Chip, onremove: EventHandler<()>) -> Element {
    let label = chip.label();
    rsx! {
        span { class: "ct-chip",
            svg { class: "icon icon-sm", "aria-hidden": "true",
                r#use { href: "/icons.svg#filter" }
            }
            span { class: "ct-chip-label", "{label}" }
            button {
                class: "ct-chip-remove",
                r#type: "button",
                "aria-label": "Remove filter {label}",
                onclick: move |_| onremove.call(()),
                svg { class: "icon icon-sm", "aria-hidden": "true",
                    r#use { href: "/icons.svg#x" }
                }
            }
        }
    }
}

fn sort_direction(sort: Sort, column: SortColumn) -> SortDirection {
    match sort {
        Some((c, dir)) if c == column => dir,
        _ => SortDirection::None,
    }
}

/// What the table says when no row is left.
pub fn empty_copy(filters: &Filters) -> (String, String, Option<&'static str>) {
    if let Some(state) = filters.state {
        return (
            format!("No {} containers", state.label().to_lowercase()),
            "Nothing in the current view is in that state.".to_string(),
            None,
        );
    }
    if !filters.text.trim().is_empty() {
        return (
            "No containers match".to_string(),
            format!("Nothing matches \u{201c}{}\u{201d}.", filters.text.trim()),
            Some("Clear filter"),
        );
    }
    if filters.project.is_some() {
        return (
            "No containers in this project".to_string(),
            "Start the project to see its containers here.".to_string(),
            None,
        );
    }
    (
        "No containers yet".to_string(),
        "Start a project or run an image and its containers show up here.".to_string(),
        None,
    )
}

/// The command line the empty state shows under "Start engine".
fn start_hint(engine: &Engine) -> String {
    let profile = match &engine.kind {
        EngineKind::Colima { profile } => profile.clone(),
        _ => engine.name.clone(),
    };
    let mut parts = vec![format!("colima start {profile}")];
    if let Some(vm) = &engine.resources {
        parts.push(format!("{} CPU", vm.cpus));
        let yaml = berth_sys::paths::colima_dir()
            .join(&profile)
            .join("colima.yaml");
        let shown = berth_core::fmt::tilde(&yaml, &berth_sys::paths::home());
        parts.push(format!(
            "{} (from {shown})",
            berth_core::fmt::bytes(vm.memory)
        ));
    }
    parts.push("about 20 s".to_string());
    parts.join(" \u{00b7} ")
}

#[component]
pub fn ContainersScreen() -> Element {
    let mut state = use_context::<AppState>();
    let view = use_hook(|| {
        try_consume_context::<ContainersView>()
            .unwrap_or_else(|| provide_context(ContainersView::new()))
    });
    let actions = try_consume_context::<ActionsService>();
    let model = use_memo(move || {
        let containers = state.containers.read();
        let projects = state.projects.read();
        let engines = state.engines.read();
        let scope = state.engine_scope.read();
        let filters = view.filters.read();
        let collapsed = view.collapsed.read();
        let pending = state.pending.read();
        build_list(&ListInput {
            containers: &containers,
            projects: &projects,
            engines: &engines,
            scope: &scope,
            filters: &filters,
            group_by: *view.group_by.read(),
            sort: *view.sort.read(),
            collapsed: &collapsed,
            pending: &pending,
        })
    });

    let engines = state.engines.read().clone();
    let scope = state.engine_scope.read().clone();
    let filters = view.filters.read().clone();
    let group_by = *view.group_by.read();
    let sort = *view.sort.read();
    let status = list_status(
        &engines,
        &scope,
        &state.containers.read(),
        *state.snapshot_stale.read(),
    );
    let (total, matched, row_count) = {
        let m = model.read();
        (m.total, m.matched, m.rows.len())
    };
    let counts_total: usize = engines
        .iter()
        .filter(|e| scope_includes(&scope, &e.id))
        .map(|e| e.counts.containers_total as usize)
        .sum();
    let shown_total = match status {
        ListStatus::Ready => total,
        _ => counts_total,
    };
    let chip_list = chips(&engines, &scope, &filters);
    let banners = engine_banners(&engines, &scope);
    let toggled = *view.inspector_toggled.read();
    let docked_inspector = {
        let section = *state.section.read();
        inspector_visible(section, &state.selection.read()) && !toggled
    };
    let show_uptime = !docked_inspector;
    let show_engine = matches!(scope, EngineScope::All);
    let bulk_ids: Option<Vec<ContainerId>> = match &*state.selection.read() {
        Selection::Containers(ids) if ids.len() >= 2 => Some(ids.clone()),
        _ => None,
    };
    let grid = format!(
        "ct-grid{}{}{}",
        if show_uptime { " ct-has-uptime" } else { "" },
        if show_engine { " ct-has-engine" } else { "" },
        if bulk_ids.is_some() { " ct-bulk" } else { "" }
    );
    let filter_active = filters.is_active();
    let epoch = *view.filter_epoch.read();
    // The virtual table's maths must equal the stylesheet's `--size-row`.
    let row_height_px = {
        let settings = state.settings.read();
        crate::text_scale::row_height_px(settings.density, settings.text_scale)
    };

    let render_row = use_callback(move |i: usize| -> Element {
        let model = model.read();
        let Some(row) = model.rows.get(i) else {
            return rsx! {};
        };
        let engine_scope_all = matches!(&*state.engine_scope.read(), EngineScope::All);
        let toggled = *view.inspector_toggled.read();
        let docked = {
            let section = *state.section.read();
            inspector_visible(section, &state.selection.read()) && !toggled
        };
        match row {
            ListRow::Group(info) => rsx! { GroupRowHost { key: "{info.key}", info: info.clone() } },
            ListRow::Container(id) => rsx! {
                ContainerRowHost {
                    key: "{id.0}",
                    id: id.clone(),
                    show_uptime: !docked,
                    show_engine: engine_scope_all,
                }
            },
            ListRow::Failure(id) => rsx! { FailureRow { key: "{id.0}-failure", id: id.clone() } },
        }
    });

    let onsort = move |column: SortColumn| {
        let mut sort = view.sort;
        let next = match *sort.peek() {
            Some((c, dir)) if c == column => (column, dir.next()),
            _ => (column, SortDirection::Asc),
        };
        sort.set(Some(next));
    };
    let header_cell = move |column: SortColumn, label: &'static str, align: CellAlign| {
        rsx! {
            TableHeaderCell {
                label: label.to_string(),
                sortable: true,
                align,
                sort: sort_direction(sort, column),
                onsort: move |()| onsort(column),
            }
        }
    };

    let head = rsx! {
        div { class: "ct-head", role: "row",
            span {}
            span {}
            {header_cell(SortColumn::Name, "Name", CellAlign::Left)}
            span { class: "ct-image-head", {header_cell(SortColumn::Image, "Image", CellAlign::Left)} }
            {header_cell(SortColumn::Ports, "Ports", CellAlign::Left)}
            {header_cell(SortColumn::Cpu, "CPU", CellAlign::Right)}
            {header_cell(SortColumn::Memory, "Memory", CellAlign::Right)}
            if show_uptime {
                span { class: "ct-uptime", {header_cell(SortColumn::Uptime, "Uptime", CellAlign::Left)} }
            }
            if show_engine {
                span { class: "ct-engine", TableHeaderCell { label: "Engine".to_string() } }
            }
            {header_cell(SortColumn::Status, "Status", CellAlign::Left)}
        }
    };

    let group_options: Vec<SelectOption> = GroupBy::ALL
        .iter()
        .map(|g| SelectOption::new(g.value(), g.label()))
        .collect();
    let start_actions = actions.clone();
    let clear_filter = move || {
        let mut filters = view.filters;
        filters.write().text.clear();
        let mut epoch = view.filter_epoch;
        let next = *epoch.peek() + 1;
        epoch.set(next);
    };

    rsx! {
        style { "{CONTAINERS_CSS}" }
        section {
            class: "containers-screen",
            "aria-label": "Containers",
            onclick: move |_| {
                let mut menu = view.menu_open;
                if menu.peek().is_some() {
                    menu.set(None);
                }
                let mut group_menu = view.group_menu;
                if group_menu.peek().is_some() {
                    group_menu.set(None);
                }
            },
            if let Some(ids) = bulk_ids.clone() {
                BulkBarHost { ids }
            }
            if bulk_ids.is_none() {
            div { class: "ct-header",
                h1 { class: "ct-title", "Containers" }
                Badge { kind: BadgeKind::Count, text: "{shown_total}" }
                span { class: "ct-spacer" }
                div { class: "ct-filter",
                    for key in [epoch] {
                        SearchField {
                            key: "{key}",
                            initial_value: filters.text.clone(),
                            placeholder: "Filter containers".to_string(),
                            aria_label: "Filter containers".to_string(),
                            matched: if filters.text.trim().is_empty() { None } else { Some(matched) },
                            total: if filters.text.trim().is_empty() { None } else { Some(total) },
                            oninput: move |e: FormEvent| {
                                let mut filters = view.filters;
                                filters.write().text = e.value();
                            },
                            onclear: move |_| clear_filter(),
                        }
                    }
                }
                div { class: "ct-group",
                    Select {
                        options: group_options,
                        value: group_by.value().to_string(),
                        aria_label: "Group containers by".to_string(),
                        onchange: move |e: FormEvent| {
                            let mut group = view.group_by;
                            group.set(GroupBy::parse(&e.value()));
                        },
                    }
                }
                div { class: "ct-inspector-toggle",
                    Button {
                        label: "Inspector".to_string(),
                        icon: "file-text",
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Md,
                        onclick: move |_| crate::keys::dispatch(&mut state, "global.toggle_inspector"),
                    }
                }
            }
            }
            if bulk_ids.is_none() && !chip_list.is_empty() {
                div { class: "ct-chips", role: "group", "aria-label": "Active filters",
                    for chip in chip_list {
                        FilterChip {
                            key: "{chip.label()}",
                            chip: chip.clone(),
                            onremove: move |()| match &chip {
                                Chip::Engine(_) => state.engine_scope.set(EngineScope::All),
                                Chip::State(_) => {
                                    let mut filters = view.filters;
                                    filters.write().state = None;
                                }
                                Chip::Project(_) => {
                                    let mut filters = view.filters;
                                    filters.write().project = None;
                                }
                            },
                        }
                    }
                }
            }
            if !banners.is_empty() {
                div { class: "ct-banners",
                    for (index, (tone, message)) in banners.iter().enumerate() {
                        Banner { key: "{index}", message: message.clone(), tone: *tone }
                    }
                }
            }
            match status {
                ListStatus::Loading => rsx! {
                    div { class: "ct-table {grid}", role: "table",
                        {head.clone()}
                        div { class: "ct-skeleton", SkeletonRows { count: 8_usize } }
                    }
                },
                ListStatus::EngineStopped(engine) => {
                    let starting = state.pending.read().iter().find_map(|(key, value)| {
                        match (&key.subject, value) {
                            (Subject::Engine(id), PendingState::Running { label }) if *id == engine.id => {
                                Some(label.clone())
                            }
                            _ => None,
                        }
                    });
                    let failure = state.pending.read().iter().find_map(|(key, value)| {
                        match (&key.subject, value) {
                            (Subject::Engine(id), PendingState::Failed { message }) if *id == engine.id => {
                                Some(message.clone())
                            }
                            _ => None,
                        }
                    });
                    let counts = engine.counts.clone();
                    let body = format!(
                        "Start it to see its {} containers, {} images and {} volumes.",
                        counts.containers_total, counts.images, counts.volumes
                    );
                    let label = starting
                        .clone()
                        .unwrap_or_else(|| format!("Start engine {}", engine.name));
                    let hint = start_hint(&engine);
                    let target = (*engine).clone();
                    rsx! {
                        div { class: "ct-stopped",
                            if let Some(message) = failure {
                                InlineMessage { message, tone: InlineTone::Danger }
                            }
                            EmptyState {
                                title: "Start the engine to see its containers".to_string(),
                                body,
                                icon: "server",
                                action_label: Some(label),
                                hint: Some(hint),
                                onaction: move |()| {
                                    if let Some(service) = start_actions.as_ref() {
                                        if let Err(reason) = service.engine(
                                            &mut state,
                                            &target,
                                            berth_core::ColimaOp::Start,
                                        ) {
                                            tracing::debug!(%reason, "start engine refused");
                                        }
                                    }
                                },
                            }
                        }
                    }
                }
                ListStatus::Ready if row_count == 0 => {
                    let (title, body, action) = empty_copy(&filters);
                    rsx! {
                        div { class: "ct-empty",
                            EmptyState {
                                title,
                                body,
                                icon: "box",
                                action_label: if filter_active { action.map(str::to_string) } else { None },
                                onaction: move |()| clear_filter(),
                            }
                        }
                    }
                }
                ListStatus::Ready => rsx! {
                    div { class: "ct-table {grid}", role: "table",
                        {head.clone()}
                        VirtualTable {
                            total: row_count,
                            render_row,
                            row_height_px,
                            viewport_height_px: 1100_u32,
                            overscan: 10_usize,
                        }
                    }
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "containers_tests.rs"]
mod tests;
