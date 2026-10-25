//! The Delete project sheet (P5-T6, brief 7.3): `docker compose down` for
//! one project, with two checkboxes. Containers and networks always go;
//! "Also delete volumes" and "Also remove images" are off until ticked, and
//! the folder is never touched. The rows, the button and the result message
//! all come from one `EffectSummary` (`confirm::project_delete_effect`).
//!
//! With volumes ticked and some to delete, the project's name must be typed
//! first (the same rule as deleting a volume, D33). The dialog is drawn with
//! the confirm dialog's own markup and styles: `ConfirmDialog` carries one
//! checkbox and this sheet needs two.
//!
//! Confirm runs `ActionsService::project(ProjectOp::Delete{..})`; the result
//! is said from the summary (`confirm::watch_result`) and a failure stays on
//! the project (D34).

use dioxus::prelude::*;

use berth_core::{ProjectId, Subject};

use crate::services::actions::{disabled_reason, with_view, ActionsService, ProjectOp};
use crate::services::confirm::{
    self, delete_option_labels, project_delete_effect, project_holdings, DeleteOptions, Launched,
};
use crate::state::{AppState, PendingKey};
use crate::ui::confirm_dialog::{footer, name_matches, row_label, CONFIRM_DIALOG_CSS};
use crate::ui::{Button, ButtonVariant, CheckState, Checkbox, ControlSize, TextField};

/// The action the sheet runs for `options`.
#[must_use]
pub fn delete_op(options: DeleteOptions) -> ProjectOp {
    ProjectOp::Delete {
        volumes: options.volumes,
        remove_images: options.remove_images,
    }
}

/// The pending key a confirmed delete is watched under.
#[must_use]
pub fn delete_key(id: &ProjectId) -> PendingKey {
    PendingKey {
        subject: Subject::Project(id.clone()),
        action: "project.delete",
    }
}

/// Why the Delete button is locked: what the engine says first, else the
/// name still to be typed. `None` when it may run.
#[must_use]
pub fn delete_blocker(
    effect: &berth_core::EffectSummary,
    typed: &str,
    engine_reason: Option<String>,
) -> Option<String> {
    engine_reason.or_else(|| {
        let name = effect.typed_name.as_ref()?;
        (!name_matches(effect, typed)).then(|| format!("Type {name} to confirm"))
    })
}

/// What the Delete button does: close the sheet and run `down` with the ticked options through the
/// actions service. A refusal (engine down, no Compose file) lands on the project as a failure, so
/// the keys to watch are returned either way.
pub fn confirm_delete(
    state: &mut AppState,
    actions: Option<&ActionsService>,
    id: &ProjectId,
    options: DeleteOptions,
) -> Launched {
    state.overlays.write().pop();
    crate::ui::project_row::run_project_op(state, actions, id, delete_op(options));
    Launched {
        keys: vec![delete_key(id)],
        refused: Vec::new(),
    }
}

#[component]
pub fn DeleteProjectSheet(id: ProjectId) -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_consume_context::<ActionsService>();
    let mut volumes = use_signal(|| false);
    let mut images = use_signal(|| false);
    let mut typed = use_signal(String::new);

    let project = state.projects.read().iter().find(|p| p.id == id).cloned();
    let Some(project) = project else {
        return rsx! {
            style { "{CONFIRM_DIALOG_CSS}" }
            div {
                class: "confirm-dialog",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "Delete project",
                onclick: move |e| e.stop_propagation(),
                h2 { class: "confirm-title", "This project no longer exists" }
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

    let holdings = project_holdings(&project, &state.containers.read(), &state.volumes.read());
    let options = DeleteOptions {
        volumes: *volumes.read(),
        remove_images: *images.read(),
    };
    let home = berth_sys::paths::home();
    let folder = project
        .folder
        .as_deref()
        .map(|f| berth_core::fmt::tilde(f, &home));
    let effect = project_delete_effect(&project, &holdings, folder.as_deref(), options);
    let labels = delete_option_labels(&holdings);
    let engine_reason = with_view(&state, |view| {
        disabled_reason(&Subject::Project(id.clone()), "project.delete", view)
    });
    let blocker = delete_blocker(&effect, &typed.read(), engine_reason);
    let label = confirm::button_label(&effect);
    let note = footer(&effect);
    let title = format!("Delete project {}?", project.name);
    let run_effect = effect.clone();
    let run_id = id.clone();

    rsx! {
        style { "{CONFIRM_DIALOG_CSS}" }
        div {
            class: "confirm-dialog",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "{title}",
            onclick: move |e| e.stop_propagation(),
            h2 { class: "confirm-title", "{title}" }
            dl { class: "confirm-rows",
                for (index, line) in effect.lines.iter().enumerate() {
                    div { key: "{index}", class: "confirm-row",
                        dt { class: "confirm-row-label", "{row_label(line.kind)}" }
                        dd { class: "confirm-row-text", "{line.text}" }
                    }
                }
            }
            div { class: "confirm-option",
                Checkbox {
                    state: CheckState::from_bool(options.volumes),
                    label: labels[0].clone(),
                    disabled: holdings.volumes.is_empty(),
                    on_toggle: move |next: bool| volumes.set(next),
                }
            }
            div { class: "confirm-option",
                Checkbox {
                    state: CheckState::from_bool(options.remove_images),
                    label: labels[1].clone(),
                    disabled: holdings.images.is_empty(),
                    on_toggle: move |next: bool| images.set(next),
                }
            }
            if let Some(name) = effect.typed_name.clone() {
                div { class: "confirm-typed",
                    TextField {
                        label: format!("Type {name} to confirm"),
                        placeholder: name.clone(),
                        aria_label: format!("Type {name} to confirm"),
                        oninput: move |e: FormEvent| typed.set(e.value()),
                    }
                }
            }
            if let Some(text) = note {
                p { class: "confirm-footer", "{text}" }
            }
            div { class: "confirm-actions",
                Button {
                    label: "Cancel".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: move |_| {
                        state.overlays.write().pop();
                    },
                }
                Button {
                    label,
                    variant: ButtonVariant::Danger,
                    size: ControlSize::Md,
                    disabled: blocker.is_some(),
                    disabled_reason: blocker,
                    onclick: move |_| {
                        let launched = confirm_delete(&mut state, actions.as_ref(), &run_id, options);
                        spawn(confirm::watch_result(state, run_effect.clone(), launched));
                    },
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "delete_project_tests.rs"]
mod tests;
