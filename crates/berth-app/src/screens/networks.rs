//! The Networks screen (P4-T5, brief §7.3, PLAN §7.2: no frame, built from
//! the Containers header and table). A short table of networks: name,
//! driver, subnet, how many containers are on it and the project it
//! belongs to. A row selects the network (its detail is in the inspector,
//! `screens/network_detail.rs`); Delete asks first (D33) and is absent on
//! the built-in networks, which show why instead of a button.
//!
//! The view model is pure ([`build_rows`]); the component only draws
//! signals and sends clicks to the confirm builder and the actions service.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use dioxus::prelude::*;

use berth_core::{Container, Engine, EngineState, Network, Project, Subject};

use crate::services::actions::{disabled_reason, dismiss, StateView};
use crate::services::confirm;
use crate::services::networks::{self as net, BUILT_IN_REASON};
use crate::state::{AppState, EngineScope, Overlay, PendingKey, PendingState, Section, Selection};
use crate::ui::{
    Badge, BadgeKind, Button, ButtonVariant, CellAlign, ControlSize, EmptyState, IconButton,
    IconButtonTone, InlineMessage, InlineTone, SearchField, SortDirection, TableHeaderCell,
};

/// The screen's CSS: the hand-written rules and the generated column tiers.
pub const NETWORKS_CSS: &str = concat!(
    include_str!("../../assets/css/networks.css"),
    include_str!("../../assets/css/networks_tiers.css")
);

// ---------------------------------------------------------------------
// The view model
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Driver,
    Subnet,
    Containers,
    Project,
}

/// What a row offers in its last cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteState {
    /// The Delete button.
    Allowed,
    /// The Delete button, disabled, with the reason as its tooltip.
    Disabled(String),
    /// No Delete at all: a built-in network cannot be removed. The text
    /// is the reason, shown where the button would be.
    Locked(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetworkRow {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub subnet: Option<String>,
    pub containers: usize,
    pub project: Option<String>,
    pub built_in: bool,
    /// The engine's name, when several engines are in view.
    pub engine_tag: Option<String>,
    pub delete: DeleteState,
    /// The label of an action running on this network ("Deleting…").
    pub busy: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListModel {
    pub rows: Vec<NetworkRow>,
    /// Networks in scope before the text filter.
    pub total: usize,
    pub matched: usize,
}

pub struct Input<'a> {
    pub networks: &'a [Network],
    pub containers: &'a [Container],
    pub projects: &'a [Project],
    pub engines: &'a [Engine],
    pub scope: &'a EngineScope,
    pub pending: &'a BTreeMap<PendingKey, PendingState>,
    pub filter: &'a str,
    pub sort: Option<(SortColumn, SortDirection)>,
}

fn matches_text(row: &NetworkRow, terms: &[String]) -> bool {
    let haystack = format!(
        "{} {} {} {}",
        row.name,
        row.driver,
        row.subnet.as_deref().unwrap_or_default(),
        row.project.as_deref().unwrap_or_default()
    )
    .to_lowercase();
    terms.iter().all(|term| haystack.contains(term))
}

fn cmp_opt<T: PartialOrd>(a: Option<T>, b: Option<T>, dir: SortDirection) -> Ordering {
    // Missing values sort last in either direction.
    match (a, b) {
        (Some(a), Some(b)) => {
            let ord = a.partial_cmp(&b).unwrap_or(Ordering::Equal);
            if dir == SortDirection::Desc {
                ord.reverse()
            } else {
                ord
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// The subnet as numbers: IPv4 before IPv6, then by address, so 172.9.0.0
/// sorts before 172.19.0.0.
fn subnet_key(row: &NetworkRow) -> Option<(u8, u128)> {
    let text = row.subnet.as_deref()?;
    let (address, _) = text.split_once('/')?;
    match address.parse::<std::net::IpAddr>().ok()? {
        std::net::IpAddr::V4(a) => Some((4, u128::from(u32::from(a)))),
        std::net::IpAddr::V6(a) => Some((6, u128::from(a))),
    }
}

fn compare(a: &NetworkRow, b: &NetworkRow, column: SortColumn, dir: SortDirection) -> Ordering {
    let by_name = || a.name.cmp(&b.name);
    let ord = match column {
        SortColumn::Name => {
            let ord = a.name.to_lowercase().cmp(&b.name.to_lowercase());
            if dir == SortDirection::Desc {
                ord.reverse()
            } else {
                ord
            }
        }
        SortColumn::Driver => {
            let ord = a.driver.cmp(&b.driver);
            if dir == SortDirection::Desc {
                ord.reverse()
            } else {
                ord
            }
        }
        SortColumn::Subnet => cmp_opt(subnet_key(a), subnet_key(b), dir),
        SortColumn::Containers => {
            let ord = a.containers.cmp(&b.containers);
            if dir == SortDirection::Desc {
                ord.reverse()
            } else {
                ord
            }
        }
        SortColumn::Project => cmp_opt(a.project.as_deref(), b.project.as_deref(), dir),
    };
    ord.then_with(by_name)
}

fn delete_state(network: &Network, view: &StateView<'_>) -> DeleteState {
    if network.built_in {
        return DeleteState::Locked(BUILT_IN_REASON.to_string());
    }
    if let Some(reason) = disabled_reason(
        &Subject::Network(network.id.clone()),
        "network.delete",
        view,
    ) {
        return DeleteState::Disabled(reason);
    }
    DeleteState::Allowed
}

fn busy_label(pending: &BTreeMap<PendingKey, PendingState>, id: &str) -> Option<String> {
    pending
        .iter()
        .find_map(|(key, value)| match (&key.subject, value) {
            (Subject::Network(n), PendingState::Running { label }) if n == id => {
                Some(label.clone())
            }
            _ => None,
        })
}

/// The rows of the table, filtered by engine and text and sorted. The
/// unused built-in networks (`host`, `none`) exist once per engine and say
/// nothing about which; one of each shows.
#[must_use]
pub fn build_rows(input: &Input<'_>) -> ListModel {
    let view = StateView {
        engines: input.engines,
        containers: input.containers,
        projects: input.projects,
        images: &[],
        volumes: &[],
        networks: input.networks,
        pending: input.pending,
        backing: crate::services::backing::Backing::default(),
    };
    let running = input
        .engines
        .iter()
        .filter(|e| e.state == EngineState::Running)
        .count();
    let tag_engines = matches!(input.scope, EngineScope::All) && running > 1;
    let mut seen_unused_built_in: Vec<&str> = Vec::new();
    let mut rows: Vec<NetworkRow> = Vec::new();
    for network in input.networks {
        if !crate::ui::scope_includes(input.scope, &network.engine) {
            continue;
        }
        if network.built_in && network.containers.is_empty() {
            if seen_unused_built_in.contains(&network.name.as_str()) {
                continue;
            }
            seen_unused_built_in.push(&network.name);
        }
        let engine_tag = tag_engines
            .then(|| crate::screens::activity::engine_name(input.engines, &network.engine));
        rows.push(NetworkRow {
            id: network.id.clone(),
            name: network.name.clone(),
            driver: network.driver.clone(),
            subnet: network.subnet.clone(),
            containers: network.containers.len(),
            project: net::project_of(network, input.containers, input.projects),
            built_in: network.built_in,
            engine_tag,
            delete: delete_state(network, &view),
            busy: busy_label(input.pending, &network.id),
        });
    }
    let total = rows.len();
    let terms: Vec<String> = input
        .filter
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    rows.retain(|row| matches_text(row, &terms));
    match input.sort {
        Some((column, dir)) => rows.sort_by(|a, b| compare(a, b, column, dir)),
        // Your networks first, Docker's own after them.
        None => rows.sort_by(|a, b| {
            a.built_in
                .cmp(&b.built_in)
                .then_with(|| compare(a, b, SortColumn::Name, SortDirection::Asc))
        }),
    }
    ListModel {
        matched: rows.len(),
        total,
        rows,
    }
}

/// What a network failure line says: "Could not delete web-net: …".
#[must_use]
pub fn failure_text(key: &PendingKey, message: &str, networks: &[Network]) -> String {
    let verb = match key.action {
        "network.create" => "create",
        "network.delete" => "delete",
        "network.connect_container" => "connect",
        "network.disconnect" => "disconnect",
        _ => "change",
    };
    let name = match &key.subject {
        Subject::Network(id) => networks
            .iter()
            .find(|n| n.id == *id)
            .map_or_else(|| id.clone(), |n| n.name.clone()),
        _ => String::new(),
    };
    format!("Could not {verb} network {name}: {message}")
}

/// The failures to show above the table, in key order.
#[must_use]
pub fn failures(
    pending: &BTreeMap<PendingKey, PendingState>,
    networks: &[Network],
) -> Vec<(PendingKey, String)> {
    pending
        .iter()
        .filter(|(key, _)| matches!(key.subject, Subject::Network(_)))
        .filter_map(|(key, value)| match value {
            PendingState::Failed { message } => {
                Some((key.clone(), failure_text(key, message, networks)))
            }
            PendingState::Running { .. } => None,
        })
        .collect()
}

// ---------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------

/// ⌫ on the selected network: the delete dialog, when the network can be
/// deleted. Only on the Networks screen and never over another overlay.
pub fn delete_selected(state: &mut AppState) {
    if *state.section.peek() != Section::Networks || !state.overlays.peek().is_empty() {
        return;
    }
    let Selection::Network(id) = state.selection.peek().clone() else {
        return;
    };
    let allowed = {
        let networks = state.networks.peek();
        let Some(network) = networks.iter().find(|n| n.id == id) else {
            return;
        };
        !network.built_in
            && crate::services::actions::with_view(state, |view| {
                disabled_reason(&Subject::Network(id.clone()), "network.delete", view)
            })
            .is_none()
    };
    if allowed {
        confirm::open_delete_network(state, &id);
    }
}

// ---------------------------------------------------------------------
// The component
// ---------------------------------------------------------------------

fn sort_direction(sort: Option<(SortColumn, SortDirection)>, column: SortColumn) -> SortDirection {
    match sort {
        Some((c, dir)) if c == column => dir,
        _ => SortDirection::None,
    }
}

#[component]
fn NetworkRowView(row: NetworkRow, selected: bool) -> Element {
    let mut state = use_context::<AppState>();
    let select_id = row.id.clone();
    let key_id = row.id.clone();
    let delete_id = row.id.clone();
    let class = if selected {
        "nw-row nw-selected"
    } else {
        "nw-row"
    };
    let subnet = row.subnet.clone().unwrap_or_default();
    let project = row.project.clone().unwrap_or_default();
    rsx! {
        div {
            class,
            role: "row",
            tabindex: "0",
            "aria-selected": selected.to_string(),
            onclick: move |_| state.selection.set(Selection::Network(select_id.clone())),
            onkeydown: move |e| {
                if matches!(e.key(), Key::Enter) || e.key() == Key::Character(" ".to_string()) {
                    e.prevent_default();
                    state.selection.set(Selection::Network(key_id.clone()));
                }
            },
            div { class: "nw-cell nw-name", role: "cell",
                span { class: "nw-name-text", title: "{row.name}", "{row.name}" }
                if let Some(engine) = &row.engine_tag {
                    span { class: "nw-engine-tag", "{engine}" }
                }
            }
            div { class: "nw-cell nw-driver nw-muted", role: "cell", "{row.driver}" }
            div { class: "nw-cell nw-mono", role: "cell",
                if subnet.is_empty() {
                    span { class: "nw-muted", "\u{2013}" }
                } else {
                    "{subnet}"
                }
            }
            div { class: "nw-cell nw-num", role: "cell", "{row.containers}" }
            div { class: "nw-cell nw-project", role: "cell",
                if project.is_empty() {
                    span { class: "nw-muted", "\u{2013}" }
                } else {
                    "{project}"
                }
            }
            div { class: "nw-cell nw-actions", role: "cell",
                if let Some(label) = &row.busy {
                    span { class: "nw-muted", "{label}" }
                } else {
                    match &row.delete {
                        DeleteState::Allowed => rsx! {
                            IconButton {
                                icon: "trash-2",
                                label: format!("Delete network {}", row.name),
                                tooltip: "Delete".to_string(),
                                tone: IconButtonTone::Danger,
                                size: ControlSize::Sm,
                                onclick: move |e: MouseEvent| {
                                    e.stop_propagation();
                                    confirm::open_delete_network(&mut state, &delete_id);
                                },
                            }
                        },
                        DeleteState::Disabled(reason) => rsx! {
                            IconButton {
                                icon: "trash-2",
                                label: format!("Delete network {}", row.name),
                                tooltip: reason.clone(),
                                tone: IconButtonTone::Danger,
                                size: ControlSize::Sm,
                                disabled: true,
                            }
                        },
                        DeleteState::Locked(reason) => rsx! {
                            span { class: "nw-locked", "data-tooltip": "{reason}", "Built-in" }
                        },
                    }
                }
            }
        }
    }
}

#[component]
pub fn NetworksScreen() -> Element {
    let mut state = use_context::<AppState>();
    let mut filter = use_signal(String::new);
    let mut epoch = use_signal(|| 0_u32);
    let mut sort = use_signal(|| None::<(SortColumn, SortDirection)>);

    let model = use_memo(move || {
        let networks = state.networks.read();
        let containers = state.containers.read();
        let projects = state.projects.read();
        let engines = state.engines.read();
        let scope = state.engine_scope.read();
        let pending = state.pending.read();
        build_rows(&Input {
            networks: &networks,
            containers: &containers,
            projects: &projects,
            engines: &engines,
            scope: &scope,
            pending: &pending,
            filter: &filter.read(),
            sort: *sort.read(),
        })
    });
    let failed = {
        let pending = state.pending.read();
        let networks = state.networks.read();
        failures(&pending, &networks)
    };
    let selected_id = match &*state.selection.read() {
        Selection::Network(id) => Some(id.clone()),
        _ => None,
    };
    let (rows, total, matched) = {
        let m = model.read();
        (m.rows.clone(), m.total, m.matched)
    };
    let filtering = !filter.read().trim().is_empty();
    let any_running = state
        .engines
        .read()
        .iter()
        .any(|e| e.state == EngineState::Running);

    let header_cell = move |column: SortColumn, label: &'static str, align: CellAlign| {
        rsx! {
            TableHeaderCell {
                label: label.to_string(),
                sortable: true,
                align,
                sort: sort_direction(*sort.read(), column),
                onsort: move |()| {
                    let next = match *sort.peek() {
                        Some((c, dir)) if c == column => (column, dir.next()),
                        _ => (column, SortDirection::Asc),
                    };
                    sort.set(Some(next));
                },
            }
        }
    };

    rsx! {
        style { "{NETWORKS_CSS}" }
        section { class: "networks-screen", "aria-label": "Networks",
            div { class: "nw-header",
                h1 { class: "nw-title", "Networks" }
                Badge { kind: BadgeKind::Count, text: "{total}" }
                span { class: "nw-spacer" }
                div { class: "nw-filter",
                    for key in [*epoch.read()] {
                        SearchField {
                            key: "{key}",
                            initial_value: filter.peek().clone(),
                            placeholder: "Filter networks".to_string(),
                            aria_label: "Filter networks".to_string(),
                            matched: if filtering { Some(matched) } else { None },
                            total: if filtering { Some(total) } else { None },
                            oninput: move |e: FormEvent| filter.set(e.value()),
                            onclear: move |_| {
                                filter.set(String::new());
                                let next = *epoch.peek() + 1;
                                epoch.set(next);
                            },
                        }
                    }
                }
                Button {
                    label: "Create network".to_string(),
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    icon: "plus",
                    onclick: move |_| state.overlays.write().push(Overlay::CreateNetwork),
                }
            }
            if !failed.is_empty() {
                div { class: "nw-notes",
                    for (key , text) in failed {
                        div { key: "{key.action}-{text}", class: "nw-failure",
                            InlineMessage { message: text, tone: InlineTone::Danger }
                            Button {
                                label: "Dismiss".to_string(),
                                variant: ButtonVariant::Ghost,
                                size: ControlSize::Sm,
                                onclick: move |_| dismiss(&mut state, &key),
                            }
                        }
                    }
                }
            }
            if rows.is_empty() {
                div { class: "nw-empty",
                    if filtering {
                        EmptyState {
                            title: "No network matches".to_string(),
                            body: "Nothing on this engine matches the filter.".to_string(),
                            icon: "network",
                            action_label: Some("Clear filter".to_string()),
                            onaction: move |()| {
                                filter.set(String::new());
                                let next = *epoch.peek() + 1;
                                epoch.set(next);
                            },
                        }
                    } else if any_running {
                        EmptyState {
                            title: "No networks yet".to_string(),
                            body: "Create one to put containers on a network of their own.".to_string(),
                            icon: "network",
                            action_label: Some("Create network".to_string()),
                            onaction: move |()| state.overlays.write().push(Overlay::CreateNetwork),
                        }
                    } else {
                        EmptyState {
                            title: "Start the engine to see its networks".to_string(),
                            body: "Networks are read from a running engine.".to_string(),
                            icon: "server",
                        }
                    }
                }
            } else {
                div { class: "nw-table", role: "table", "aria-label": "Networks",
                    div { class: "nw-head", role: "row",
                        {header_cell(SortColumn::Name, "Name", CellAlign::Left)}
                        span { class: "nw-driver-head", {header_cell(SortColumn::Driver, "Driver", CellAlign::Left)} }
                        {header_cell(SortColumn::Subnet, "Subnet", CellAlign::Left)}
                        {header_cell(SortColumn::Containers, "Containers", CellAlign::Right)}
                        span { class: "nw-project-head", {header_cell(SortColumn::Project, "Project", CellAlign::Left)} }
                        span {}
                    }
                    for row in rows {
                        NetworkRowView {
                            key: "{row.id}",
                            selected: selected_id.as_deref() == Some(row.id.as_str()),
                            row,
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "networks_tests.rs"]
mod tests;
