//! One container in the Containers table (PLAN 7.3, components-spec
//! `ContainerRow`): checkbox, status dot, name, image, ports as links,
//! CPU and memory with a micro bar ("—" without stats), uptime, engine and
//! the row buttons. The buttons show on hover and always on the selected
//! row. The row is pure markup over [`ContainerRowData`]; the screen builds
//! the data (labels, tooltips and disabled reasons come from `actions.rs`
//! and `disabled_reason`) and owns every callback.

use dioxus::prelude::*;

use super::{
    CheckState, Checkbox, ControlSize, IconButton, IconButtonTone, Menu, MenuEntry, StatusDot,
};

/// The buttons a row can show, in frame order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    StartStop,
    Restart,
    Logs,
    Terminal,
    Delete,
    More,
}

impl RowAction {
    /// Stable name for markup hooks and tests.
    pub fn name(self) -> &'static str {
        match self {
            Self::StartStop => "start-stop",
            Self::Restart => "restart",
            Self::Logs => "logs",
            Self::Terminal => "terminal",
            Self::Delete => "delete",
            Self::More => "more",
        }
    }
}

/// One published port. A link only when it has a host port (D-ports:
/// opens `http://localhost:<port>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortLink {
    pub label: String,
    pub host_port: Option<u16>,
}

/// A number with its micro bar (`fraction` 0..=1).
#[derive(Debug, Clone, PartialEq)]
pub struct Meter {
    pub text: String,
    pub fraction: f32,
}

/// One row button, already resolved: tooltip text is "Restart · ⌘R" or,
/// for a disabled button, the reason it cannot run.
#[derive(Debug, Clone, PartialEq)]
pub struct RowButton {
    pub action: RowAction,
    pub icon: &'static str,
    pub label: String,
    pub tooltip: String,
    pub disabled: bool,
    pub loading: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContainerRowData {
    pub id: berth_core::ContainerId,
    pub name: String,
    pub image: String,
    pub status: berth_core::StatusKind,
    /// The status text: the engine's state, or "Stopping…" while an action
    /// runs (D34).
    pub status_text: String,
    pub ports: Vec<PortLink>,
    /// Published ports beyond the ones listed in `ports`.
    pub extra_ports: usize,
    pub cpu: Option<Meter>,
    pub memory: Option<Meter>,
    pub uptime: Option<String>,
    pub engine: String,
    pub buttons: Vec<RowButton>,
    /// The ⋯ menu, shown under the row while `menu_open`.
    pub menu: Vec<MenuEntry>,
    pub menu_open: bool,
}

fn bar_style(fraction: f32) -> String {
    let pct = (fraction.clamp(0.0, 1.0) * 100.0).round();
    format!("width: {pct}%;")
}

#[component]
fn MeterCell(meter: Option<Meter>, class: &'static str) -> Element {
    rsx! {
        div { class: "ct-cell ct-meter {class}",
            if let Some(meter) = meter {
                span { class: "ct-bar", "aria-hidden": "true",
                    span { class: "ct-bar-fill", style: "{bar_style(meter.fraction)}" }
                }
                span { class: "ct-num", "{meter.text}" }
            } else {
                span { class: "ct-dash", "—" }
            }
        }
    }
}

#[component]
pub fn ContainerRow(
    data: ContainerRowData,
    selected: bool,
    show_uptime: bool,
    show_engine: bool,
    onselect: EventHandler<berth_core::ContainerId>,
    ontoggle: EventHandler<berth_core::ContainerId>,
    onaction: EventHandler<(berth_core::ContainerId, RowAction)>,
    onport: EventHandler<u16>,
    onmenu: EventHandler<(berth_core::ContainerId, String)>,
) -> Element {
    let mut class = String::from("ct-row");
    if selected {
        class.push_str(" ct-selected");
    }
    if show_uptime {
        class.push_str(" ct-has-uptime");
    }
    if show_engine {
        class.push_str(" ct-has-engine");
    }
    let id = data.id.clone();
    let toggle_id = data.id.clone();
    let check = CheckState::from_bool(selected);
    let busy = data.buttons.iter().any(|b| b.loading);
    rsx! {
        div {
            class,
            "data-container": "{data.name}",
            "aria-selected": selected.to_string(),
            "aria-busy": busy.to_string(),
            onclick: move |_| onselect.call(id.clone()),
            span {
                class: "ct-cell ct-check",
                onclick: move |e| e.stop_propagation(),
                Checkbox {
                    state: check,
                    aria_label: format!("Select {}", data.name),
                    on_toggle: move |_| ontoggle.call(toggle_id.clone()),
                }
            }
            span { class: "ct-cell ct-dot", StatusDot { status: data.status } }
            span { class: "ct-cell ct-name", title: "{data.name}", "{data.name}" }
            span { class: "ct-cell ct-image", title: "{data.image}",
                span { class: "ct-image-text", "{data.image}" }
            }
            div { class: "ct-cell ct-ports",
                if data.ports.is_empty() {
                    span { class: "ct-dash", "—" }
                }
                for port in data.ports.iter() {
                    {
                        let label = port.label.clone();
                        match port.host_port {
                            Some(host) => rsx! {
                                a {
                                    key: "{label}",
                                    class: "ct-port-link",
                                    href: "http://localhost:{host}",
                                    title: "Open http://localhost:{host}",
                                    onclick: move |e| {
                                        e.prevent_default();
                                        e.stop_propagation();
                                        onport.call(host);
                                    },
                                    "{label}"
                                }
                            },
                            None => rsx! { span { key: "{label}", class: "ct-port", "{label}" } },
                        }
                    }
                }
                if data.extra_ports > 0 {
                    span { class: "ct-port-more", "+{data.extra_ports}" }
                }
            }
            MeterCell { meter: data.cpu.clone(), class: "ct-cpu" }
            MeterCell { meter: data.memory.clone(), class: "ct-mem" }
            if show_uptime {
                div { class: "ct-cell ct-uptime",
                    if let Some(uptime) = &data.uptime {
                        "{uptime}"
                    } else {
                        span { class: "ct-dash", "—" }
                    }
                }
            }
            if show_engine {
                span { class: "ct-cell ct-engine", "{data.engine}" }
            }
            div { class: "ct-cell ct-status",
                span { class: "ct-status-text", "{data.status_text}" }
                if data.menu_open {
                    {
                        let menu_id = data.id.clone();
                        rsx! {
                            div {
                                class: "ct-menu",
                                onclick: move |e| e.stop_propagation(),
                                Menu {
                                    items: data.menu.clone(),
                                    aria_label: format!("More actions for {}", data.name),
                                    on_select: move |item: String| onmenu.call((menu_id.clone(), item)),
                                }
                            }
                        }
                    }
                }
                div { class: "ct-actions", role: "group", "aria-label": "Actions for {data.name}",
                    for button in data.buttons.iter() {
                        {
                            let action = button.action;
                            let id = data.id.clone();
                            rsx! {
                                IconButton {
                                    key: "{action.name()}",
                                    icon: button.icon,
                                    label: button.label.clone(),
                                    tooltip: button.tooltip.clone(),
                                    size: ControlSize::Sm,
                                    tone: if action == RowAction::Delete {
                                        IconButtonTone::Danger
                                    } else {
                                        IconButtonTone::Neutral
                                    },
                                    disabled: button.disabled,
                                    loading: button.loading,
                                    onclick: move |e: MouseEvent| {
                                        e.stop_propagation();
                                        onaction.call((id.clone(), action));
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
