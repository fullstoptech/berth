//! A network in the inspector, and a container's networks in its Overview
//! (P4-T5). Both end in the same two actions: Connect a container to a
//! network and Disconnect one from it.
//!
//! [`NetworkDetailPanel`] is self-contained: `app.rs` mounts it in the
//! inspector for `Selection::Network`. [`ContainerNetworks`] is the
//! "Networks" row of a container's Overview, with the buttons.
//!
//! The rows are pure ([`detail_rows`], [`container_rows`]); the components
//! send clicks to the confirm builder and the actions service. The engine
//! of a network comes from `services::networks::engine_of`; the engine of a
//! container is its own.

use dioxus::prelude::*;

use berth_core::{Container, ContainerId, Engine, EngineId, Network, Project, Subject};

use crate::screens::containers::select_only;
use crate::screens::networks::{failure_text, DeleteState};
use crate::services::actions::{disabled_reason, dismiss, with_view, ActionsService};
use crate::services::confirm;
use crate::services::networks::{self as net, NetOp, BUILT_IN_REASON};
use crate::state::{AppState, PendingKey, PendingState, Section, Toast, ToastTone};
use crate::ui::{
    Badge, BadgeKind, Button, ButtonVariant, ControlSize, InlineMessage, InlineTone, KeyValue,
    KeyValueList, Menu, MenuEntry, StatusDot,
};

/// The hand-written rules only (the table's generated tiers are not needed
/// here), plus the key-value list the panel borrows from the container's.
const DETAIL_CSS: &str = include_str!("../../assets/css/networks.css");
const KV_CSS: &str = include_str!("../../assets/css/container_detail.css");

// ---------------------------------------------------------------------
// Pure rows
// ---------------------------------------------------------------------

/// The key-value rows of a network, in the order the detail shows them.
#[must_use]
pub fn detail_rows(
    network: &Network,
    project: Option<&str>,
    engine: Option<&str>,
) -> Vec<KeyValue> {
    let mut rows = vec![
        KeyValue::mono("ID", short_id(&network.id)),
        KeyValue::text("Driver", network.driver.clone()),
        KeyValue::mono("Subnet", network.subnet.clone().unwrap_or_default()).empty_text("\u{2013}"),
        KeyValue::mono("Gateway", network.gateway.clone().unwrap_or_default())
            .empty_text("\u{2013}"),
        KeyValue::text("Internal", if network.internal { "Yes" } else { "No" }),
        KeyValue::text("Project", project.unwrap_or_default()).empty_text("\u{2013}"),
    ];
    if let Some(engine) = engine {
        rows.push(KeyValue::text("Engine", engine));
    }
    rows
}

fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

/// One container on the network and what Disconnect would do to it.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberRow {
    pub id: ContainerId,
    pub name: String,
    pub status: berth_core::StatusKind,
}

/// The containers on a network, by name.
#[must_use]
pub fn members(network: &Network, containers: &[Container]) -> Vec<MemberRow> {
    let mut list: Vec<MemberRow> = network
        .containers
        .iter()
        .filter_map(|id| containers.iter().find(|c| c.id == *id))
        .map(|c| MemberRow {
            id: c.id.clone(),
            name: c.name.clone(),
            status: berth_core::status_of(c.state, c.health, c.exit_code),
        })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

/// The network detail's Delete: absent on a built-in network, disabled with
/// its reason while containers are on it.
#[must_use]
pub fn delete_state(network: &Network, view_reason: Option<String>) -> DeleteState {
    if network.built_in {
        DeleteState::Locked(BUILT_IN_REASON.to_string())
    } else if let Some(reason) = view_reason {
        DeleteState::Disabled(reason)
    } else {
        DeleteState::Allowed
    }
}

/// The running label of an action on this network, if one runs.
fn running_label(
    pending: &std::collections::BTreeMap<PendingKey, PendingState>,
    id: &str,
) -> Option<String> {
    pending
        .iter()
        .find_map(|(key, value)| match (&key.subject, value) {
            (Subject::Network(n), PendingState::Running { label }) if n == id => {
                Some(label.clone())
            }
            _ => None,
        })
}

fn toast(state: &mut AppState, text: String) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone: ToastTone::Danger,
        action: None,
    });
}

/// Send one network action; a refusal becomes a toast.
fn send(
    state: &mut AppState,
    actions: Option<&ActionsService>,
    engine: &EngineId,
    network: &Network,
    op: NetOp,
) {
    let Some(service) = actions else { return };
    if let Err(reason) = service.network(state, engine, &network.id, &network.name, op) {
        toast(state, reason);
    }
}

// ---------------------------------------------------------------------
// The network in the inspector
// ---------------------------------------------------------------------

/// The failures of this network's actions, as the text to show and the key
/// that dismisses each.
fn failures_of(
    pending: &std::collections::BTreeMap<PendingKey, PendingState>,
    network: &Network,
    networks: &[Network],
) -> Vec<(PendingKey, String)> {
    pending
        .iter()
        .filter(|(key, _)| key.subject == Subject::Network(network.id.clone()))
        .filter_map(|(key, value)| match value {
            PendingState::Failed { message } => {
                Some((key.clone(), failure_text(key, message, networks)))
            }
            PendingState::Running { .. } => None,
        })
        .collect()
}

#[component]
pub fn NetworkDetailPanel(id: String) -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_consume_context::<ActionsService>();
    let mut menu_open = use_signal(|| false);

    let networks = state.networks.read().clone();
    let Some(network) = networks.iter().find(|n| n.id == id).cloned() else {
        return rsx! {
            style { "{DETAIL_CSS}" }
            p { class: "nd-note", "This network no longer exists." }
        };
    };
    let containers = state.containers.read().clone();
    let projects: Vec<Project> = state.projects.read().clone();
    let engines: Vec<Engine> = state.engines.read().clone();
    let pending = state.pending.read().clone();

    let engine = network.engine.clone();
    let project = net::project_of(&network, &containers, &projects);
    let engine_name =
        (engines.len() > 1).then(|| crate::screens::activity::engine_name(&engines, &engine));
    let rows = detail_rows(&network, project.as_deref(), engine_name.as_deref());
    let on_it = members(&network, &containers);
    let busy = running_label(&pending, &network.id);
    let failed = failures_of(&pending, &network, &networks);
    let reason = with_view(&state, |view| {
        disabled_reason(
            &Subject::Network(network.id.clone()),
            "network.delete",
            view,
        )
    });
    // Delete is its own busy state: only the built-in reason is a lock.
    let delete = delete_state(&network, reason);
    let offered: Vec<Container> = net::connectable(&network, &engine, &containers)
        .into_iter()
        .cloned()
        .collect();
    let menu_items: Vec<MenuEntry> = offered
        .iter()
        .map(|c| MenuEntry::new(c.id.0.clone(), c.name.clone()))
        .collect();
    let connect_reason = if busy.is_some() {
        busy.clone().map(|l| format!("{l} is still in progress"))
    } else if offered.is_empty() {
        Some("Every container on this engine is already connected".to_string())
    } else {
        None
    };

    let delete_id = network.id.clone();
    let connect_network = network.clone();
    let connect_engine = engine.clone();
    let connect_actions = actions.clone();
    let connect_offered = offered.clone();
    rsx! {
        style { "{DETAIL_CSS}" }
        style { "{KV_CSS}" }
        div { class: "nd",
            header { class: "nd-header",
                div { class: "nd-title",
                    h2 { class: "nd-name", title: "{network.name}", "{network.name}" }
                    Badge { kind: BadgeKind::Tag, text: network.driver.clone() }
                }
                match delete.clone() {
                    DeleteState::Allowed => rsx! {
                        div { class: "nd-buttons",
                            Button {
                                label: "Delete network".to_string(),
                                variant: ButtonVariant::Danger,
                                size: ControlSize::Sm,
                                icon: "trash-2",
                                loading: busy.is_some(),
                                onclick: move |_| {
                                    confirm::open_delete_network(&mut state, &delete_id);
                                },
                            }
                        }
                    },
                    DeleteState::Disabled(reason) => rsx! {
                        div { class: "nd-buttons",
                            Button {
                                label: "Delete network".to_string(),
                                variant: ButtonVariant::Danger,
                                size: ControlSize::Sm,
                                icon: "trash-2",
                                disabled: true,
                                disabled_reason: Some(reason.clone()),
                            }
                        }
                        p { class: "nd-note", "{reason}." }
                    },
                    DeleteState::Locked(reason) => rsx! {
                        p { class: "nd-note", "{reason}. Docker creates this network." }
                    },
                }
            }
            div { class: "nd-body",
                for (key , text) in failed {
                    div { key: "{key.action}-{text}", class: "nd-section",
                        InlineMessage { message: text, tone: InlineTone::Danger }
                        div { class: "nd-buttons",
                            Button {
                                label: "Dismiss".to_string(),
                                variant: ButtonVariant::Ghost,
                                size: ControlSize::Sm,
                                onclick: move |_| dismiss(&mut state, &key),
                            }
                        }
                    }
                }
                KeyValueList { rows }
                section { class: "nd-section",
                    div { class: "nd-section-head",
                        h3 { class: "nd-section-title", "Containers ({on_it.len()})" }
                        div { class: "nd-menu-host",
                            Button {
                                label: "Connect container".to_string(),
                                variant: ButtonVariant::Secondary,
                                size: ControlSize::Sm,
                                icon: "plus",
                                disabled: connect_reason.is_some(),
                                disabled_reason: connect_reason.clone(),
                                onclick: move |e: MouseEvent| {
                                    e.stop_propagation();
                                    let open = *menu_open.peek();
                                    menu_open.set(!open);
                                },
                            }
                            if *menu_open.read() && !menu_items.is_empty() {
                                div { class: "nd-menu",
                                    Menu {
                                        items: menu_items.clone(),
                                        aria_label: "Connect a container".to_string(),
                                        on_select: move |container_id: String| {
                                            menu_open.set(false);
                                            let picked = connect_offered
                                                .iter()
                                                .find(|c| c.id.0 == container_id);
                                            if let Some(picked) = picked {
                                                let op = NetOp::Connect {
                                                    network: connect_network.name.clone(),
                                                    container: picked.id.clone(),
                                                };
                                                send(&mut state, connect_actions.as_ref(), &connect_engine, &connect_network, op);
                                            }
                                        },
                                    }
                                }
                            }
                        }
                    }
                    if on_it.is_empty() {
                        p { class: "nd-note", "No container is connected." }
                    } else {
                        ul { class: "nd-list",
                            for member in on_it {
                                {
                                    let go_id = member.id.clone();
                                    let off_network = network.clone();
                                    let off_engine = containers
                                        .iter()
                                        .find(|c| c.id == member.id)
                                        .map(|c| c.engine.clone());
                                    let off_id = member.id.clone();
                                    let off_actions = actions.clone();
                                    rsx! {
                                        li { key: "{member.id.0}", class: "nd-item",
                                            StatusDot { status: member.status }
                                            button {
                                                class: "nd-item-name",
                                                r#type: "button",
                                                title: "Open {member.name}",
                                                onclick: move |_| {
                                                    state.section.set(Section::Containers);
                                                    select_only(&mut state, &go_id);
                                                },
                                                "{member.name}"
                                            }
                                            Button {
                                                label: "Disconnect".to_string(),
                                                variant: ButtonVariant::Secondary,
                                                size: ControlSize::Sm,
                                                disabled: busy.is_some(),
                                                disabled_reason: busy.clone().map(|l| format!("{l} is still in progress")),
                                                onclick: move |_| {
                                                    if let Some(engine) = off_engine.as_ref() {
                                                        let op = NetOp::Disconnect {
                                                            network: off_network.name.clone(),
                                                            container: off_id.clone(),
                                                        };
                                                        send(&mut state, off_actions.as_ref(), engine, &off_network, op);
                                                    }
                                                },
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// A container's networks, in its Overview
// ---------------------------------------------------------------------

/// One network a container is on.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerNetworkRow {
    pub name: String,
    pub built_in: bool,
    /// The network's id when the list knows it (the key of its actions).
    pub network: Option<Network>,
}

/// The networks `container` is on, by name, each with the network the list
/// knows by that name.
#[must_use]
pub fn container_rows(container: &Container, networks: &[Network]) -> Vec<ContainerNetworkRow> {
    let mut rows: Vec<ContainerNetworkRow> = container
        .networks
        .iter()
        .map(|name| {
            let network = net::network_named(name, container, networks).cloned();
            ContainerNetworkRow {
                name: name.clone(),
                built_in: network.as_ref().is_some_and(|n| n.built_in),
                network,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

/// The "Networks" row of a container's Overview: each network the
/// container is on with Disconnect, and a "Connect to network" menu of the
/// others on its engine. Draws like a row of the key-value list above it.
#[component]
pub fn ContainerNetworks(id: ContainerId) -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_consume_context::<ActionsService>();
    let mut menu_open = use_signal(|| false);

    let containers = state.containers.read().clone();
    let Some(container) = containers.iter().find(|c| c.id == id).cloned() else {
        return rsx! {};
    };
    let networks = state.networks.read().clone();
    let pending = state.pending.read().clone();

    let rows = container_rows(&container, &networks);
    let offered: Vec<Network> = net::joinable(&container, &networks)
        .into_iter()
        .cloned()
        .collect();
    let menu_items: Vec<MenuEntry> = offered
        .iter()
        .map(|n| MenuEntry::new(n.id.clone(), n.name.clone()))
        .collect();
    let failed: Vec<(PendingKey, String)> = rows
        .iter()
        .filter_map(|r| r.network.as_ref())
        .flat_map(|n| failures_of(&pending, n, &networks))
        .collect();
    let join_actions = actions.clone();
    let join_container = container.clone();
    let join_offered = offered.clone();
    rsx! {
        style { "{DETAIL_CSS}" }
        dl { class: "kv-list",
            div { class: "kv-row",
                dt { class: "kv-label", "Networks" }
                dd { class: "kv-value cn-value",
                    ul { class: "nd-list cn-list",
                        if rows.is_empty() {
                            li { class: "nd-item kv-empty", "\u{2013}" }
                        }
                        for row in rows {
                            {
                                let busy = row
                                    .network
                                    .as_ref()
                                    .and_then(|n| running_label(&pending, &n.id));
                                let off_actions = actions.clone();
                                let off_container = container.clone();
                                let off_row = row.clone();
                                rsx! {
                                    li { key: "{row.name}", class: "nd-item",
                                        span { class: "nd-item-name kv-mono", title: "{row.name}", "{row.name}" }
                                        Button {
                                            label: "Disconnect".to_string(),
                                            variant: ButtonVariant::Ghost,
                                            size: ControlSize::Sm,
                                            disabled: row.network.is_none() || busy.is_some(),
                                            disabled_reason: if let Some(label) = busy.clone() {
                                                Some(format!("{label} is still in progress"))
                                            } else if row.network.is_none() {
                                                Some("This network is not in the list yet".to_string())
                                            } else {
                                                None
                                            },
                                            onclick: move |_| {
                                                if let Some(network) = off_row.network.as_ref() {
                                                    let op = NetOp::Disconnect {
                                                        network: network.name.clone(),
                                                        container: off_container.id.clone(),
                                                    };
                                                    send(&mut state, off_actions.as_ref(), &off_container.engine, network, op);
                                                }
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "nd-menu-host",
                        Button {
                            label: "Connect to network".to_string(),
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Sm,
                            icon: "plus",
                            disabled: menu_items.is_empty(),
                            disabled_reason: menu_items
                                .is_empty()
                                .then(|| "Already on every network of this engine".to_string()),
                            onclick: move |e: MouseEvent| {
                                e.stop_propagation();
                                let open = *menu_open.peek();
                                menu_open.set(!open);
                            },
                        }
                        if *menu_open.read() && !menu_items.is_empty() {
                            div { class: "nd-menu",
                                Menu {
                                    items: menu_items.clone(),
                                    aria_label: "Connect to a network".to_string(),
                                    on_select: move |network_id: String| {
                                        menu_open.set(false);
                                        if let Some(network) = join_offered.iter().find(|n| n.id == network_id) {
                                            let op = NetOp::Connect {
                                                network: network.name.clone(),
                                                container: join_container.id.clone(),
                                            };
                                            send(&mut state, join_actions.as_ref(), &join_container.engine, network, op);
                                        }
                                    },
                                }
                            }
                        }
                    }
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
        }
    }
}

#[cfg(test)]
#[path = "network_detail_tests.rs"]
mod tests;
