//! The Create volume sheet (P4-T4, no frame: the confirm dialog's card).
//! Name, driver and driver options, and one primary button. A name that is
//! already taken on the engine shows inline before the button enables, and
//! the engine's own refusal (a race, a bad option) shows in the same place;
//! the sheet closes only when the volume exists.
//!
//! The engine is the selected one; under "All engines" with several running
//! it is a choice. The create runs through the actions service like every
//! other action (pending state, activity entry).

use dioxus::prelude::*;

use berth_core::{EngineId, EngineState};

use crate::screens::volumes::VOLUMES_CSS;
use crate::services::actions::ActionsService;
use crate::services::volumes::{self, VolumeOp};
use crate::state::{AppState, PendingKey, PendingState};
use crate::ui::{
    scope_includes, Button, ButtonVariant, ControlSize, Select, SelectOption, TextField,
};
use berth_core::Subject;

/// The running engines a new volume could go on, in list order.
#[must_use]
pub fn candidates(
    engines: &[berth_core::Engine],
    scope: &crate::state::EngineScope,
) -> Vec<(EngineId, String)> {
    engines
        .iter()
        .filter(|e| e.state == EngineState::Running && scope_includes(scope, &e.id))
        .map(|e| (e.id.clone(), e.name.clone()))
        .collect()
}

/// Is `name` already a volume on `engine`? A volume whose engine is not
/// known counts as taken: better a refusal here than the engine's.
#[must_use]
pub fn name_taken(state: &AppState, name: &str, engine: &EngineId) -> bool {
    let name = name.trim();
    let containers = state.containers.peek();
    let engines = state.engines.peek();
    state
        .volumes
        .peek()
        .iter()
        .filter(|v| v.name == name)
        .any(|v| {
            volumes::owner_engine(v, &containers, &engines)
                .as_ref()
                .is_none_or(|owner| owner == engine)
        })
}

#[component]
pub fn CreateVolumeSheet() -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_consume_context::<ActionsService>();
    let mut name = use_signal(String::new);
    let mut driver = use_signal(|| "local".to_string());
    let mut options = use_signal(String::new);
    let options_list = candidates(&state.engines.read(), &state.engine_scope.read());
    let first = options_list.first().map(|(id, _)| id.clone());
    let mut chosen = use_signal(move || first);
    let mut submitted = use_signal(|| None::<PendingKey>);
    let mut refused = use_signal(|| None::<String>);

    // The sheet is done when the action it started has left the pending map
    // without failing.
    use_effect(move || {
        let Some(key) = submitted.read().clone() else {
            return;
        };
        if !state.pending.read().contains_key(&key) {
            submitted.set(None);
            state.overlays.write().pop();
        }
    });

    let typed = name.read().clone();
    let engine = chosen.read().clone();
    let taken = engine
        .as_ref()
        .is_some_and(|e| !typed.trim().is_empty() && name_taken(&state, &typed, e));
    let name_problem = volumes::name_problem(&typed, taken);
    let options_problem = volumes::parse_options(&options.read()).err();
    let (running, failed) = {
        let pending = state.pending.read();
        let started = submitted.read().clone();
        match started.as_ref().and_then(|k| pending.get(k)) {
            Some(PendingState::Running { .. }) => (true, None),
            Some(PendingState::Failed { message }) => (false, Some(message.clone())),
            None => (false, None),
        }
    };
    let blocker = if typed.trim().is_empty() {
        Some("Give the volume a name".to_string())
    } else if let Some(problem) = name_problem.clone().or_else(|| options_problem.clone()) {
        Some(problem)
    } else if engine.is_none() {
        Some("No engine is running".to_string())
    } else {
        None
    };
    let inline_error = name_problem
        .clone()
        .or(failed)
        .or_else(|| refused.read().clone());
    let engine_options: Vec<SelectOption> = options_list
        .iter()
        .map(|(id, label)| SelectOption::new(id.0.clone(), label.clone()))
        .collect();
    let engine_value = engine.as_ref().map(|e| e.0.clone()).unwrap_or_default();
    let ask_engine = options_list.len() > 1;

    let mut create = {
        let actions = actions.clone();
        move |()| {
            let Some(service) = actions.as_ref() else {
                refused.set(Some("Actions are not connected".to_string()));
                return;
            };
            let Some(engine) = chosen.peek().clone() else {
                return;
            };
            let spec = match volumes::create_spec(&name.peek(), &driver.peek(), &options.peek()) {
                Ok(spec) => spec,
                Err(reason) => {
                    refused.set(Some(reason));
                    return;
                }
            };
            refused.set(None);
            let started = PendingKey {
                subject: Subject::Volume(spec.name.clone()),
                action: volumes::CREATE,
            };
            let spec_name = spec.name.clone();
            match service.volume(&mut state, &engine, &spec_name, VolumeOp::Create(spec)) {
                Ok(()) => submitted.set(Some(started)),
                Err(reason) => refused.set(Some(reason)),
            }
        }
    };

    rsx! {
        style { "{VOLUMES_CSS}" }
        div {
            class: "cv-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Create volume",
            onclick: move |e| e.stop_propagation(),
            h2 { class: "cv-title", "Create volume" }
            p { class: "cv-lede", "A volume keeps data after the container that uses it is removed." }
            div { class: "cv-form",
                if ask_engine {
                    Select {
                        label: "Engine".to_string(),
                        options: engine_options,
                        value: engine_value,
                        aria_label: "Engine".to_string(),
                        onchange: move |e: FormEvent| chosen.set(Some(EngineId(e.value()))),
                    }
                }
                TextField {
                    label: "Name".to_string(),
                    placeholder: "my-volume".to_string(),
                    aria_label: "Volume name".to_string(),
                    error: inline_error,
                    oninput: move |e: FormEvent| {
                        refused.set(None);
                        name.set(e.value());
                    },
                }
                TextField {
                    label: "Driver".to_string(),
                    initial_value: "local".to_string(),
                    aria_label: "Volume driver".to_string(),
                    oninput: move |e: FormEvent| driver.set(e.value()),
                }
                TextField {
                    label: "Options".to_string(),
                    placeholder: "type=tmpfs device=tmpfs".to_string(),
                    aria_label: "Driver options".to_string(),
                    help: "Driver options as key=value, separated by spaces. Most volumes need none."
                        .to_string(),
                    error: options_problem,
                    oninput: move |e: FormEvent| options.set(e.value()),
                }
            }
            div { class: "cv-actions",
                Button {
                    label: "Cancel".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: move |_| {
                        state.overlays.write().pop();
                    },
                }
                Button {
                    label: if running { "Creating\u{2026}".to_string() } else { "Create volume".to_string() },
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    loading: running,
                    disabled: blocker.is_some(),
                    disabled_reason: blocker,
                    onclick: move |_| create(()),
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "create_volume_tests.rs"]
mod tests;
