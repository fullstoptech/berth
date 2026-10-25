//! The Create network sheet (P4-T5, brief §7.3; PLAN §7.2: no frame, built
//! from the Clean up sheet). Name, driver, subnet, gateway, internal, and
//! the containers to attach at once. The draft is checked as it changes
//! (`services::networks::validate`): an invalid subnet or gateway shows its
//! message under the field and keeps Create disabled with the reason.
//!
//! Text inputs are uncontrolled (D59): the sheet reads `oninput` into a
//! signal and never writes a value back. Create runs through the actions
//! service as one pending action keyed by the new name; the sheet closes
//! when it succeeds and shows the engine's message when it fails.

use dioxus::prelude::*;

use berth_core::{ContainerId, EngineId, EngineState, Network};

use crate::services::actions::{dismiss, ActionsService};
use crate::services::networks::{
    self as net, create_blocker, validate, Draft, Field, FieldError, NetOp, DRIVERS,
};
use crate::state::{AppState, EngineScope, Overlay, PendingKey, PendingState, Toast, ToastTone};
use crate::ui::{
    Button, ButtonVariant, CheckState, Checkbox, ControlSize, IconButton, InlineMessage,
    InlineTone, Select, SelectOption, StatusDot, Switch, TextField,
};

/// The sheet's CSS (the hand-written rules of the Networks screen).
const SHEET_CSS: &str = include_str!("../../assets/css/networks.css");

/// Dev scenes put edits into the form before it is drawn.
#[derive(Clone, Copy)]
pub struct DevNetworkDraft(pub fn(&mut Draft));

/// The pending state of a create of `name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateStatus {
    Idle,
    Running(String),
    Failed { key: PendingKey, message: String },
}

/// What the create action of `name` is doing.
#[must_use]
pub fn create_status(
    pending: &std::collections::BTreeMap<PendingKey, PendingState>,
    name: &str,
) -> CreateStatus {
    let key = PendingKey {
        subject: berth_core::Subject::Network(name.to_string()),
        action: "network.create",
    };
    match pending.get(&key) {
        Some(PendingState::Running { label }) => CreateStatus::Running(label.clone()),
        Some(PendingState::Failed { message }) => CreateStatus::Failed {
            key,
            message: message.clone(),
        },
        None => CreateStatus::Idle,
    }
}

/// The networks `name` and `subnet` are checked against: the ones on this
/// engine, plus the ones whose engine cannot be told (a false "taken" is
/// better than Docker's "Pool overlaps" after the click).
fn existing_on(
    engine: &EngineId,
    networks: &[Network],
    containers: &[berth_core::Container],
    engines: &[berth_core::Engine],
    scope: &EngineScope,
) -> Vec<Network> {
    networks
        .iter()
        .filter(|n| net::engine_of(n, containers, engines, scope).is_none_or(|e| e == *engine))
        .cloned()
        .collect()
}

fn error_of(errors: &[FieldError], field: Field) -> Option<String> {
    errors
        .iter()
        .find(|e| e.field == field && !e.blank)
        .map(|e| e.message.clone())
}

fn close_sheet(state: &mut AppState) {
    state
        .overlays
        .write()
        .retain(|o| *o != Overlay::CreateNetwork);
}

fn refuse(state: &mut AppState, text: String) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone: ToastTone::Danger,
        action: None,
    });
}

#[component]
pub fn CreateNetworkSheet() -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_consume_context::<ActionsService>();
    let dev = try_consume_context::<DevNetworkDraft>();

    let mut draft = use_signal(move || {
        let mut fresh = Draft::default();
        if let Some(dev) = dev {
            (dev.0)(&mut fresh);
        }
        fresh
    });
    let initial = draft.peek().clone();

    // Engines that can take a network, and the one the sheet creates on.
    let running: Vec<berth_core::Engine> = state
        .engines
        .read()
        .iter()
        .filter(|e| e.state == EngineState::Running)
        .cloned()
        .collect();
    let mut chosen = use_signal(|| None::<EngineId>);
    let default_engine = match &*state.engine_scope.read() {
        EngineScope::One(id) => Some(id.clone()),
        EngineScope::All => running.first().map(|e| e.id.clone()),
    };
    let engine = chosen
        .read()
        .clone()
        .filter(|id| running.iter().any(|e| e.id == *id))
        .or(default_engine);

    let networks = state.networks.read().clone();
    let containers = state.containers.read().clone();
    let engines = state.engines.read().clone();
    let scope = state.engine_scope.read().clone();
    let current = draft.read().clone();
    let existing = engine
        .as_ref()
        .map(|e| existing_on(e, &networks, &containers, &engines, &scope))
        .unwrap_or_default();
    let built = validate(&current, &existing);
    let errors = built.as_ref().err().cloned().unwrap_or_default();

    // The sheet closes when its action succeeds: it was running and is
    // neither running nor failed any more.
    let mut was_running = use_signal(|| false);
    use_effect(move || {
        let name = draft.read().name.trim().to_string();
        let status = create_status(&state.pending.read(), &name);
        let before = *was_running.peek();
        match status {
            CreateStatus::Running(_) => {
                if !before {
                    was_running.set(true);
                }
            }
            CreateStatus::Idle if before => {
                was_running.set(false);
                close_sheet(&mut state);
            }
            _ => {
                if before {
                    was_running.set(false);
                }
            }
        }
    });
    let status = create_status(&state.pending.read(), current.name.trim());

    let attachable: Vec<berth_core::Container> = {
        let mut list: Vec<berth_core::Container> = containers
            .iter()
            .filter(|c| engine.as_ref() == Some(&c.engine))
            .cloned()
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    };
    let engine_options: Vec<SelectOption> = running
        .iter()
        .map(|e| SelectOption::new(e.id.0.clone(), e.name.clone()))
        .collect();
    let driver_options: Vec<SelectOption> =
        DRIVERS.iter().map(|d| SelectOption::new(*d, *d)).collect();

    let running_now = matches!(status, CreateStatus::Running(_));
    let blocker = if engine.is_none() {
        Some("Start an engine first".to_string())
    } else {
        create_blocker(&errors)
    };
    let failure = match &status {
        CreateStatus::Failed { key, message } => Some((key.clone(), message.clone())),
        _ => None,
    };
    let create_engine = engine.clone();
    let create_actions = actions.clone();

    rsx! {
        style { "{SHEET_CSS}" }
        div {
            class: "nc-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Create network",
            onclick: move |e| e.stop_propagation(),
            header { class: "nc-header",
                h2 { class: "nc-title", "Create network" }
                IconButton {
                    icon: "x",
                    label: "Close".to_string(),
                    size: ControlSize::Sm,
                    onclick: move |_| close_sheet(&mut state),
                }
            }
            div { class: "nc-body",
                if engine_options.len() > 1 {
                    Select {
                        label: "Engine".to_string(),
                        options: engine_options,
                        value: engine.as_ref().map(|e| e.0.clone()).unwrap_or_default(),
                        aria_label: "Engine".to_string(),
                        onchange: move |e: FormEvent| chosen.set(Some(EngineId(e.value()))),
                    }
                }
                div { class: "nc-pair",
                    TextField {
                        label: "Name".to_string(),
                        initial_value: initial.name.clone(),
                        placeholder: "my-network".to_string(),
                        aria_label: "Name".to_string(),
                        error: error_of(&errors, Field::Name),
                        oninput: move |e: FormEvent| draft.write().name = e.value(),
                    }
                    Select {
                        label: "Driver".to_string(),
                        options: driver_options,
                        value: current.driver.clone(),
                        aria_label: "Driver".to_string(),
                        onchange: move |e: FormEvent| draft.write().driver = e.value(),
                    }
                }
                div { class: "nc-pair",
                    TextField {
                        label: "Subnet".to_string(),
                        initial_value: initial.subnet.clone(),
                        placeholder: "172.31.250.0/24".to_string(),
                        help: Some("Optional. Docker picks a free one when empty.".to_string()),
                        aria_label: "Subnet".to_string(),
                        error: error_of(&errors, Field::Subnet),
                        oninput: move |e: FormEvent| draft.write().subnet = e.value(),
                    }
                    TextField {
                        label: "Gateway".to_string(),
                        initial_value: initial.gateway.clone(),
                        placeholder: "172.31.250.1".to_string(),
                        help: Some("Optional. Inside the subnet.".to_string()),
                        aria_label: "Gateway".to_string(),
                        error: error_of(&errors, Field::Gateway),
                        oninput: move |e: FormEvent| draft.write().gateway = e.value(),
                    }
                }
                Switch {
                    on: current.internal,
                    label: "Internal: containers on it cannot reach the outside".to_string(),
                    aria_label: "Internal network".to_string(),
                    on_toggle: move |on: bool| draft.write().internal = on,
                }
                section { class: "nc-group",
                    h3 { class: "nc-group-title", "Attach containers" }
                    if attachable.is_empty() {
                        p { class: "nc-muted", "No container on this engine yet. Connect them later from the network." }
                    } else {
                        p { class: "nc-muted", "They join the network as soon as it exists." }
                        div { class: "nc-attach", role: "group", "aria-label": "Containers to attach",
                            for container in attachable {
                                {
                                    let id: ContainerId = container.id.clone();
                                    let on = current.attach.contains(&id);
                                    let status = berth_core::status_of(container.state, container.health, container.exit_code);
                                    rsx! {
                                        div { key: "{id.0}", class: "nc-attach-row",
                                            Checkbox {
                                                state: CheckState::from_bool(on),
                                                label: container.name.clone(),
                                                aria_label: format!("Attach {}", container.name),
                                                on_toggle: move |next: bool| {
                                                    let mut d = draft.write();
                                                    d.attach.retain(|c| *c != id);
                                                    if next {
                                                        d.attach.push(id.clone());
                                                    }
                                                },
                                            }
                                            StatusDot { status }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some((key, message)) = failure {
                    InlineMessage { message: format!("Could not create the network: {message}"), tone: InlineTone::Danger }
                    div {
                        Button {
                            label: "Dismiss".to_string(),
                            variant: ButtonVariant::Ghost,
                            size: ControlSize::Sm,
                            onclick: move |_| dismiss(&mut state, &key),
                        }
                    }
                }
            }
            footer { class: "nc-footer",
                Button {
                    label: "Cancel".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: move |_| close_sheet(&mut state),
                }
                Button {
                    label: if let CreateStatus::Running(label) = &status { label.clone() } else { "Create network".to_string() },
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    loading: running_now,
                    disabled: blocker.is_some(),
                    disabled_reason: blocker,
                    onclick: move |_| {
                        let Ok(spec) = validate(&draft.peek(), &existing) else { return };
                        let (Some(service), Some(engine)) = (create_actions.as_ref(), create_engine.as_ref()) else {
                            return;
                        };
                        let name = spec.name.clone();
                        if let Err(reason) = service.network(&mut state, engine, &name, &name, NetOp::Create(spec)) {
                            refuse(&mut state, reason);
                        }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "create_network_tests.rs"]
mod tests;
