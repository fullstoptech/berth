//! Which installed harness drives the assistant (frame "Project /
//! assistant / 1440": top-right of the panel, "Claude Code 2.4.1"),
//! and which model it runs (AD39). The version is the detected one;
//! an unavailable harness cannot be picked.

use dioxus::prelude::*;

use berth_core::{Harness, HarnessTransport};

use super::icon::{Icon, IconSize};
use super::select::{Select, SelectOption};
use super::text_field::TextField;

/// "Claude Code 2.4.1" — name plus detected version when there is one.
#[must_use]
pub fn harness_label(harness: &Harness) -> String {
    match &harness.version {
        Some(version) => format!("{} {version}", harness.name),
        None => harness.name.clone(),
    }
}

#[component]
pub fn HarnessPicker(
    harnesses: Vec<Harness>,
    /// The picked harness id.
    selected: String,
    #[props(default)] onchange: Option<EventHandler<String>>,
) -> Element {
    let available: Vec<&Harness> = harnesses.iter().filter(|h| h.available).collect();
    if available.is_empty() {
        return rsx! {
            span { class: "harness-picker harness-picker-empty",
                span { class: "harness-picker-icon",
                    Icon { name: "sparkles", size: IconSize::Sm }
                }
                "No assistant found"
            }
        };
    }
    let options: Vec<SelectOption> = available
        .iter()
        .map(|h| SelectOption::new(h.id.0.clone(), harness_label(h)))
        .collect();
    rsx! {
        span { class: "harness-picker",
            span { class: "harness-picker-icon",
                Icon { name: "sparkles", size: IconSize::Sm }
            }
            Select {
                options,
                value: selected,
                aria_label: "Assistant".to_string(),
                onchange: move |e: FormEvent| {
                    if let Some(handler) = &onchange {
                        handler.call(e.value());
                    }
                },
            }
        }
    }
}

/// The select value that opens the custom-model field.
pub const CUSTOM_MODEL: &str = "__custom";

/// The models a harness is known to accept, as the picker lists them
/// (value passed to the harness, label shown). Claude Code's aliases
/// are stable; muse ids churn, so only the one Berth has verified is
/// listed and the custom field covers the rest.
#[must_use]
pub fn curated_models(harness: &Harness) -> &'static [(&'static str, &'static str)] {
    match harness.transport {
        HarnessTransport::ClaudeStreamJson => {
            &[("opus", "Opus"), ("sonnet", "Sonnet"), ("haiku", "Haiku")]
        }
        HarnessTransport::MuseMsp => &[("muse-spark-1.2", "muse-spark-1.2")],
    }
}

/// The model picker's options: the harness default, the curated list,
/// the saved pick when it is a custom one, and Custom….
#[must_use]
pub fn model_options(harness: &Harness, saved: Option<&str>) -> Vec<SelectOption> {
    let mut options = vec![SelectOption::new("", "Default model")];
    let curated = curated_models(harness);
    options.extend(
        curated
            .iter()
            .map(|(value, label)| SelectOption::new(*value, *label)),
    );
    if let Some(saved) = saved.filter(|s| !s.is_empty()) {
        if !curated.iter().any(|(value, _)| *value == saved) {
            options.push(SelectOption::new(saved, saved));
        }
    }
    options.push(SelectOption::new(CUSTOM_MODEL, "Custom\u{2026}"));
    options
}

/// Which model the picked harness runs (AD39): a select of
/// [`model_options`], or a text field while a custom id is typed
/// (Enter applies; empty means the harness default).
#[component]
pub fn ModelPicker(
    harness: Harness,
    /// The saved model for this harness, `None` for its default.
    selected: Option<String>,
    #[props(default)] onchange: Option<EventHandler<Option<String>>>,
) -> Element {
    let mut editing = use_signal(|| false);
    let mut draft = use_signal(String::new);
    if editing() {
        let initial = selected.clone().unwrap_or_default();
        return rsx! {
            span { class: "harness-picker model-picker",
                for initial in [initial] {
                    TextField {
                        key: "{harness.id.0}",
                        initial_value: initial,
                        placeholder: "Model id".to_string(),
                        aria_label: format!("Model for {}", harness.name),
                        oninput: move |e: FormEvent| draft.set(e.value()),
                        on_enter: move |()| {
                            editing.set(false);
                            if let Some(handler) = &onchange {
                                let text = draft.peek().trim().to_string();
                                handler.call((!text.is_empty()).then_some(text));
                            }
                        },
                    }
                }
            }
        };
    }
    rsx! {
        span { class: "harness-picker model-picker",
            Select {
                options: model_options(&harness, selected.as_deref()),
                value: selected.clone().unwrap_or_default(),
                aria_label: format!("Model for {}", harness.name),
                onchange: move |e: FormEvent| {
                    let value = e.value();
                    if value == CUSTOM_MODEL {
                        draft.set(selected.clone().unwrap_or_default());
                        editing.set(true);
                    } else if let Some(handler) = &onchange {
                        handler.call((!value.is_empty()).then_some(value));
                    }
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "harness_picker_tests.rs"]
mod tests;
