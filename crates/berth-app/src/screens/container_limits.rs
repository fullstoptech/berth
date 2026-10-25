//! The Limits tab of the inspector (P3-T7, brief §7.3): CPU, memory and
//! restart policy of one container, applied live with `update_limits`
//! (PLAN §6.5). Mount it in the tab as `LimitsPanel { container }`.
//!
//! A limit that is emptied cannot be cleared on a running container
//! (Docker's update leaves an absent field alone), so the button says so
//! and points at Edit and recreate instead.

use dioxus::prelude::*;

use berth_core::Container;

use crate::services::actions::{dismiss, ActionsService, ContainerTarget, EditOp};
use crate::services::detail::{DetailEntry, DetailStore};
use crate::services::Services;
use crate::sheets::edit_recreate::model::{limits_summary, offer, removes_a_limit, Draft, Offer};
use crate::sheets::edit_recreate::{edit_status, EditStatus, LimitsFields, EDIT_RECREATE_CSS};
use crate::state::{AppState, Overlay};
use crate::ui::{Button, ButtonVariant, ControlSize, InlineMessage, InlineTone};

/// What the tab's button says and whether it can be pressed.
#[must_use]
pub fn apply_button(
    offered: Offer,
    has_errors: bool,
    removes: bool,
    status: &EditStatus,
) -> (String, Option<String>) {
    if let EditStatus::Running(label) = status {
        return (label.clone(), None);
    }
    let reason = if has_errors {
        Some("Fix the fields marked in red first".to_string())
    } else if removes {
        Some(
            "A limit cannot be removed from a container that exists. Use Edit and recreate"
                .to_string(),
        )
    } else if offered == Offer::Nothing {
        Some("Nothing has changed".to_string())
    } else {
        None
    };
    ("Apply limits".to_string(), reason)
}

#[component]
pub fn LimitsPanel(container: Container) -> Element {
    let mut state = use_context::<AppState>();
    let store = use_hook(|| try_consume_context::<DetailStore>().unwrap_or_default());
    let services = try_consume_context::<Services>();
    let actions = try_consume_context::<ActionsService>();
    let id = container.id.clone();

    let ensure_with = container.clone();
    let ensure_services = services.clone();
    use_effect(move || {
        if let Some(services) = ensure_services.clone() {
            store.ensure(&services, &ensure_with);
        }
    });

    let start_id = id.clone();
    let mut draft = use_signal(move || match store.entries.peek().get(&start_id) {
        Some(DetailEntry::Ready(loaded)) => Some(Draft::from_spec(
            &berth_core::ContainerSpec::from_detail(&loaded.detail),
        )),
        _ => None,
    });
    let late_id = id.clone();
    use_effect(move || {
        if draft.peek().is_some() {
            return;
        }
        if let Some(DetailEntry::Ready(loaded)) = store.entries.read().get(&late_id) {
            draft.set(Some(Draft::from_spec(
                &berth_core::ContainerSpec::from_detail(&loaded.detail),
            )));
        }
    });

    // After an apply went through, read the container again so the form's
    // base is what the engine now holds.
    let mut was_running = use_signal(|| false);
    let watch_id = id.clone();
    let watch_container = container.clone();
    let watch_services = services.clone();
    use_effect(move || {
        let status = edit_status(&state.pending.read(), &watch_id);
        let before = *was_running.peek();
        match status {
            EditStatus::Running(_) => {
                if !before {
                    was_running.set(true);
                }
            }
            EditStatus::Idle if before => {
                was_running.set(false);
                store.invalidate(&watch_id);
                if let Some(services) = watch_services.clone() {
                    store.ensure(&services, &watch_container);
                }
            }
            _ => {
                if before {
                    was_running.set(false);
                }
            }
        }
    });

    let mut refused = use_signal(|| None::<String>);
    let status = edit_status(&state.pending.read(), &id);
    let entry = store.entries.read().get(&id).cloned();

    let content = match (entry, draft.read().clone()) {
        (Some(DetailEntry::Ready(loaded)), Some(current)) => {
            let base = berth_core::ContainerSpec::from_detail(&loaded.detail);
            let built = current.to_spec(&base);
            let errors = built.as_ref().err().cloned().unwrap_or_default();
            let (offered, removes) = match &built {
                Ok(new) => (
                    offer(&base, new),
                    removes_a_limit(&base.limits, &new.limits),
                ),
                Err(_) => (Offer::Nothing, false),
            };
            let (label, reason) = apply_button(offered, !errors.is_empty(), removes, &status);
            let running = matches!(status, EditStatus::Running(_));
            let target = ContainerTarget::from(&container);
            let edit_id = id.clone();
            let click_actions = actions.clone();
            let failure = match &status {
                EditStatus::Failed { key, message } => Some((key.clone(), message.clone())),
                _ => None,
            };
            let now = limits_summary(&base);
            rsx! {
                p { class: "er-muted lp-now", "Now: {now}" }
                p { class: "er-muted",
                    "Limits and the restart policy apply at once, without restarting the container."
                }
                LimitsFields { draft, errors }
                if let Some((key, message)) = failure {
                    div { class: "er-failure",
                        InlineMessage { message, tone: InlineTone::Danger }
                        button {
                            class: "er-dismiss",
                            r#type: "button",
                            "aria-label": "Dismiss message",
                            onclick: move |_| dismiss(&mut state, &key),
                            "Dismiss"
                        }
                    }
                }
                if let Some(message) = refused.read().clone() {
                    InlineMessage { message, tone: InlineTone::Danger }
                }
                div { class: "lp-actions",
                    Button {
                        label,
                        variant: ButtonVariant::Primary,
                        size: ControlSize::Sm,
                        disabled: reason.is_some(),
                        disabled_reason: reason,
                        loading: running,
                        onclick: move |_| {
                            refused.set(None);
                            let Some(service) = click_actions.as_ref() else { return };
                            let Some(current) = draft.peek().clone() else { return };
                            let Ok(new) = current.to_spec(&base) else { return };
                            if offer(&base, &new) != Offer::ApplyLimits {
                                return;
                            }
                            let op = EditOp::apply_limits(&base, &new);
                            if let Err(reason) = service.edit(&mut state, &target, op) {
                                refused.set(Some(reason));
                            }
                        },
                    }
                    Button {
                        label: "Edit and recreate…".to_string(),
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Sm,
                        onclick: move |_| {
                            state
                                .overlays
                                .write()
                                .push(Overlay::EditRecreate(edit_id.clone()));
                        },
                    }
                }
            }
        }
        (Some(DetailEntry::Failed { message, .. }), _) => rsx! {
            InlineMessage {
                message: format!("Could not read the limits: {message}"),
                tone: InlineTone::Danger,
            }
        },
        _ => rsx! { p { class: "er-muted", "Loading the limits…" } },
    };

    rsx! {
        style { "{EDIT_RECREATE_CSS}" }
        div { class: "lp", {content} }
    }
}
