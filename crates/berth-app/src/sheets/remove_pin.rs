//! The Remove from Berth confirm (P10-T3): unpin a folder that nothing runs from, so a clone
//! whose validation failed (or a folder opened by mistake) can leave the Projects list. It never
//! touches files or Docker. The rows, the button and the result message come from one
//! `EffectSummary` (`confirm::remove_pin_effect`); the pin goes through `folders::remove_pin`
//! and the engines service is told the pins changed.

use std::path::Path;

use dioxus::prelude::*;

use berth_core::ProjectId;

use crate::services::confirm::{remove_pin_effect, remove_pin_title, result_label};
use crate::services::folders;
use crate::services::Services;
use crate::state::{AppState, Section, Selection, Toast, ToastTone};
use crate::ui::confirm_dialog::CONFIRM_DIALOG_CSS;
use crate::ui::{Button, ButtonVariant, ConfirmDialog, ControlSize};

/// What the dialog says for a pinned folder: its title and the one summary behind its rows,
/// button and result.
#[must_use]
pub fn dialog_of(name: &str, folder: &Path, home: &Path) -> (String, berth_core::EffectSummary) {
    let shown = berth_core::fmt::tilde(folder, home);
    (remove_pin_title(name), remove_pin_effect(name, &shown))
}

/// Unpin `folder` and say the result from `effect`: the toast text and tone.
#[must_use]
pub fn unpin(
    services: Option<&Services>,
    folder: &Path,
    effect: &berth_core::EffectSummary,
) -> (String, ToastTone) {
    let Some(services) = services else {
        return (
            "Berth could not reach its store".to_string(),
            ToastTone::Danger,
        );
    };
    match folders::remove_pin(&services.store, folder) {
        Ok(_) => (result_label(effect, 1), ToastTone::Success),
        Err(err) => (
            format!("Could not remove the pin: {err}"),
            ToastTone::Danger,
        ),
    }
}

#[component]
pub fn RemovePinSheet(id: ProjectId) -> Element {
    let mut state = use_context::<AppState>();
    let services = try_consume_context::<Services>();
    let link = try_consume_context::<Option<crate::services::engines::EngineLink>>().flatten();

    let project = state.projects.read().iter().find(|p| p.id == id).cloned();
    let Some((project, folder)) = project.and_then(|p| p.folder.clone().map(|f| (p, f))) else {
        return rsx! {
            style { "{CONFIRM_DIALOG_CSS}" }
            div {
                class: "confirm-dialog",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "Remove from Berth",
                onclick: move |e| e.stop_propagation(),
                h2 { class: "confirm-title", "This project is no longer listed" }
                div { class: "confirm-actions",
                    Button {
                        label: "Close".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Md,
                        onclick: move |_| {
                            state.overlays.write().pop();
                        },
                    }
                }
            }
        };
    };
    let (title, effect) = dialog_of(&project.name, &folder, &berth_sys::paths::home());
    let run_effect = effect.clone();
    rsx! {
        ConfirmDialog {
            title,
            effect,
            confirm_variant: ButtonVariant::Danger,
            oncheck: move |_| {},
            oncancel: move |()| {
                state.overlays.write().pop();
            },
            onconfirm: move |()| {
                state.overlays.write().pop();
                let (text, tone) = unpin(services.as_ref(), &folder, &run_effect);
                if tone == ToastTone::Success {
                    if let Some(link) = &link {
                        link.pins_changed();
                    }
                    if *state.selection.peek() == Selection::Project(id.clone()) {
                        state.selection.set(Selection::None);
                        state.section.set(Section::Projects);
                    }
                }
                state.toasts.write().push(Toast {
                    id: uuid::Uuid::now_v7(),
                    text,
                    tone,
                    action: None,
                });
            },
        }
    }
}

#[cfg(test)]
#[path = "remove_pin_tests.rs"]
mod tests;
