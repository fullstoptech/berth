//! Which installed harness drives the assistant (frame "Project /
//! assistant / 1440": top-right of the panel, "Claude Code 2.4.1").
//! The version is the detected one; an unavailable harness cannot be
//! picked.

use dioxus::prelude::*;

use berth_core::Harness;

use super::icon::{Icon, IconSize};
use super::select::{Select, SelectOption};

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
