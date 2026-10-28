//! The Save as image bar (P8-T4): the container header's ⋯ menu asks for
//! `repo:tag` under the header, like the rename bar, and the engine commits
//! the container to a new image. Three states: the field, the commit
//! running (Cancel stops waiting), and done with a link to Images.
//!
//! It is a bar and not an overlay because PLAN §6 `Overlay` has no variant
//! for it (AD46). Nothing here touches a project's files; the new image
//! lives on the engine, so there is no Review changes step.

use dioxus::prelude::*;

use berth_core::{Container, Subject};

use crate::services::images::{commit_reference, default_reference, ImageStore, COMMIT_ACTION};
use crate::services::Services;
use crate::sheets::edit_recreate::EDIT_RECREATE_CSS;
use crate::state::{AppState, PendingKey, Section};
use crate::ui::{Button, ButtonVariant, ControlSize, TextField};

/// What the bar shows after the commit ran.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Saved(String),
    Failed(String),
}

/// The words under the field: the environment travels with the image.
pub const HELP: &str = "Saves this container's files and settings as a new image. Its environment variables are saved with it.";

/// Scenes and tests start the bar from a state a click would have to reach.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DevSaveImage {
    /// The name typed over the suggestion.
    pub typed: Option<String>,
    /// The commit is running under this name.
    pub running: Option<String>,
    /// The commit ended: the name it made.
    pub saved: Option<String>,
    /// The commit ended: the engine's reason.
    pub failed: Option<String>,
}

#[component]
pub fn SaveImageBar(container: Container, onclose: EventHandler<()>) -> Element {
    let mut state = use_context::<AppState>();
    let services = try_consume_context::<Services>();
    let images = try_consume_context::<ImageStore>();
    let dev = try_consume_context::<DevSaveImage>().unwrap_or_default();
    let initial = dev
        .typed
        .clone()
        .unwrap_or_else(|| default_reference(&container.name));
    let mut typed = use_signal({
        let initial = initial.clone();
        move || initial
    });
    let mut submitted = use_signal({
        let running = dev.running.clone();
        move || running
    });
    let mut outcome = use_signal({
        let seeded = dev
            .saved
            .map(Outcome::Saved)
            .or_else(|| dev.failed.map(Outcome::Failed));
        move || seeded
    });

    let running = submitted.read().as_ref().is_some_and(|name| {
        state.pending.read().contains_key(&PendingKey {
            subject: Subject::Image(name.clone()),
            action: COMMIT_ACTION,
        })
    });
    let problem = commit_reference(&typed.read()).err();
    let engine = container.engine.clone();

    if let Some(Outcome::Saved(name)) = outcome.read().clone() {
        return rsx! {
            style { "{EDIT_RECREATE_CSS}" }
            div { class: "rn", role: "group", "aria-label": "Save as image",
                p { class: "rn-done", "Saved as {name}." }
                div { class: "rn-actions",
                    Button {
                        label: "Show in Images".to_string(),
                        variant: ButtonVariant::Primary,
                        size: ControlSize::Sm,
                        onclick: move |_| {
                            state.section.set(Section::Images);
                            onclose.call(());
                        },
                    }
                    Button {
                        label: "Close".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Sm,
                        onclick: move |_| onclose.call(()),
                    }
                }
            }
        };
    }

    let failed = match outcome.read().clone() {
        Some(Outcome::Failed(reason)) => Some(reason),
        _ => None,
    };
    let error = problem.clone().or(failed);
    let blocker = problem;

    rsx! {
        style { "{EDIT_RECREATE_CSS}" }
        div { class: "rn", role: "group", "aria-label": "Save as image",
            TextField {
                label: "Image name".to_string(),
                initial_value: initial,
                aria_label: "Image name".to_string(),
                help: HELP.to_string(),
                error,
                oninput: move |e: FormEvent| {
                    if let Ok(mut o) = outcome.try_write() {
                        *o = None;
                    }
                    typed.set(e.value());
                },
            }
            div { class: "rn-actions",
                Button {
                    label: if running { "Saving\u{2026}".to_string() } else { "Save as image".to_string() },
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Sm,
                    loading: running,
                    disabled: blocker.is_some() || running,
                    disabled_reason: blocker,
                    onclick: move |_| {
                        let (Some(services), Some(images)) = (services.as_ref(), images) else {
                            outcome.set(Some(Outcome::Failed(
                                "Berth is not connected to the engine.".to_string(),
                            )));
                            return;
                        };
                        let Ok((repo, tag)) = commit_reference(&typed.peek()) else {
                            return;
                        };
                        outcome.set(None);
                        submitted.set(Some(format!("{repo}:{tag}")));
                        let reference = format!("{repo}:{tag}");
                        images.save_as_image(services, &container, &reference, move |result| {
                            // The bar may be gone by now (another container was picked).
                            if let Ok(mut o) = outcome.try_write() {
                                *o = Some(match result {
                                    Ok(name) => Outcome::Saved(name),
                                    Err(reason) => Outcome::Failed(reason),
                                });
                            }
                        });
                    },
                }
                Button {
                    label: if running { "Stop waiting".to_string() } else { "Cancel".to_string() },
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Sm,
                    onclick: move |_| {
                        if running {
                            if let (Some(images), Some(name)) = (images, submitted.peek().clone()) {
                                images.cancel_transfer(&engine, COMMIT_ACTION, &name);
                            }
                        } else {
                            onclose.call(());
                        }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "save_image_tests.rs"]
mod tests;
