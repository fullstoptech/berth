//! ConfirmDialog (D33, PLAN 7.3): renders one `EffectSummary`. The title
//! names the subject, one row per effect line says what is removed, kept
//! or changed, and the confirm button repeats the action
//! (`confirm::button_label`). A summary with `typed_name` also asks for the
//! name to be typed (uncontrolled field, D59) before the button enables.
//!
//! [`ConfirmDialog`] is markup; [`ConfirmOverlay`] is the overlay body that
//! sends a confirmed `ConfirmModel` to the actions service.

use dioxus::prelude::*;

use berth_core::{EffectKind, EffectSummary};

use super::{Button, ButtonVariant, CheckState, Checkbox, ControlSize, Icon, IconSize, TextField};
use crate::services::actions::ActionsService;
use crate::services::confirm;
use crate::state::{AppState, ConfirmModel};

/// The CSS of the dialog, rendered with it (tokens only).
pub const CONFIRM_DIALOG_CSS: &str = include_str!("../../assets/css/confirm_dialog.css");

/// The row label of an effect line.
pub fn row_label(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Goes => "Removes",
        EffectKind::Stays => "Keeps",
        EffectKind::Changes => "Changes",
        EffectKind::Writes => "Writes",
    }
}

/// The line under the rows: the size that goes, when the summary has one.
pub fn footer(effect: &EffectSummary) -> Option<String> {
    effect
        .bytes
        .map(|b| format!("Removes {}", berth_core::fmt::bytes(b)))
}

/// Does the typed text unlock the button? Always, without a typed name.
pub fn name_matches(effect: &EffectSummary, typed: &str) -> bool {
    effect
        .typed_name
        .as_deref()
        .is_none_or(|name| name == typed)
}

#[component]
pub fn ConfirmDialog(
    title: String,
    effect: EffectSummary,
    #[props(default)] checkbox: Option<(String, bool)>,
    /// The confirm button's look: danger unless the action only adds
    /// (a download).
    #[props(default = ButtonVariant::Danger)]
    confirm_variant: ButtonVariant,
    oncheck: EventHandler<bool>,
    oncancel: EventHandler<()>,
    onconfirm: EventHandler<()>,
    /// Labelled rows (`App: landing`) in place of the effect's own, for a
    /// confirm whose rows are not removes/keeps/changes (a plugin action, AD52).
    #[props(default)]
    rows: Vec<(String, String)>,
    /// An amber warning box under the rows.
    #[props(default)]
    warning: Option<String>,
    /// Muted notes under the options.
    #[props(default)]
    notes: Vec<String>,
    /// The confirm button's text when the summary's own wording is not it.
    #[props(default)]
    button_label: Option<String>,
    /// More options (checkboxes) between the warning and the notes.
    #[props(default = rsx! {})]
    body: Element,
    /// More buttons after the confirm button ("Fix with Claude Code").
    #[props(default = rsx! {})]
    buttons: Element,
    /// The wider dialog of a plugin action (478 px, as its frame).
    #[props(default)]
    wide: bool,
) -> Element {
    let mut typed = use_signal(String::new);
    let label = button_label.unwrap_or_else(|| confirm::button_label(&effect));
    let unlocked = name_matches(&effect, &typed.read());
    let reason = effect
        .typed_name
        .as_ref()
        .filter(|_| !unlocked)
        .map(|name| format!("Type {name} to confirm"));
    let footer = footer(&effect);
    rsx! {
        style { "{CONFIRM_DIALOG_CSS}" }
        div {
            class: if wide { "confirm-dialog confirm-dialog-wide" } else { "confirm-dialog" },
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "{title}",
            onclick: move |e| e.stop_propagation(),
            h2 { class: "confirm-title", "{title}" }
            dl { class: "confirm-rows",
                if rows.is_empty() {
                    for (index, line) in effect.lines.iter().enumerate() {
                        div { key: "{index}", class: "confirm-row",
                            dt { class: "confirm-row-label", "{row_label(line.kind)}" }
                            dd { class: "confirm-row-text", "{line.text}" }
                        }
                    }
                } else {
                    for (index, (label, value)) in rows.iter().enumerate() {
                        div { key: "{index}", class: "confirm-row",
                            dt { class: "confirm-row-label", "{label}" }
                            // A commit is a hash: monospace, as the plugin screens draw one.
                            dd {
                                class: if label == "Commit" { "confirm-row-text pv-mono" } else { "confirm-row-text" },
                                "{value}"
                            }
                        }
                    }
                }
            }
            if let Some(text) = warning {
                div { class: "confirm-warning tone-warning", role: "alert",
                    Icon { name: "alert-triangle", size: IconSize::Sm }
                    span { "{text}" }
                }
            }
            {body}
            if let Some((text, on)) = checkbox {
                div { class: "confirm-option",
                    Checkbox {
                        state: CheckState::from_bool(on),
                        label: text,
                        on_toggle: move |next: bool| oncheck.call(next),
                    }
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
            for (index, note) in notes.iter().enumerate() {
                p { key: "{index}", class: "confirm-footer", "{note}" }
            }
            if let Some(text) = footer {
                p { class: "confirm-footer", "{text}" }
            }
            div { class: "confirm-actions",
                Button {
                    label: "Cancel".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: move |_| oncancel.call(()),
                }
                Button {
                    label,
                    variant: confirm_variant,
                    size: ControlSize::Md,
                    disabled: !unlocked,
                    disabled_reason: reason,
                    onclick: move |_| onconfirm.call(()),
                }
                {buttons}
            }
        }
    }
}

/// The overlay body for `Overlay::Confirm`: the dialog over a model, with
/// the checkbox state kept here and the confirmed action sent through the
/// actions service. The result is said from the same summary
/// (`confirm::watch_result`).
#[component]
pub fn ConfirmOverlay(model: ConfirmModel) -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_consume_context::<ActionsService>();
    let mut checked = use_signal(|| model.checkbox.as_ref().is_some_and(|(_, on)| *on));
    let checkbox = model
        .checkbox
        .as_ref()
        .map(|(text, _)| (text.clone(), *checked.read()));
    let confirm_model = model.clone();
    rsx! {
        ConfirmDialog {
            title: model.title.clone(),
            effect: model.effect.clone(),
            checkbox,
            oncheck: move |next| checked.set(next),
            oncancel: move |()| {
                state.overlays.write().pop();
            },
            onconfirm: move |()| {
                state.overlays.write().pop();
                let Some(service) = actions.as_ref() else { return };
                let launched = confirm::run(&mut state, service, &confirm_model, *checked.peek());
                let effect = confirm_model.effect.clone();
                spawn(confirm::watch_result(state, effect, launched));
            },
        }
    }
}
