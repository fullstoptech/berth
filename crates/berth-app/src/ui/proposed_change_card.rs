//! A harness's proposed file change (frame "Project / assistant /
//! 1440"): "Step 1 of 2", the title, the file rows with their line
//! counts, and Review changes — the only way from here to the disk
//! (ADR 0012). The card holds the AD37 summary, never the plan.

use std::path::PathBuf;

use dioxus::prelude::*;

use berth_core::ProposalState;

use super::button::{Button, ButtonVariant};
use super::icon::{Icon, IconSize};
use super::ControlSize;

/// "2 files · +10 −3" over a proposal's file summaries.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn proposal_totals(files: &[(PathBuf, u32, u32)]) -> String {
    let added: u64 = files.iter().map(|f| u64::from(f.1)).sum();
    let removed: u64 = files.iter().map(|f| u64::from(f.2)).sum();
    let noun = if files.len() == 1 { "file" } else { "files" };
    format!("{} {noun} \u{b7} +{added} \u{2212}{removed}", files.len())
}

fn state_note(state: ProposalState) -> Option<&'static str> {
    match state {
        ProposalState::Pending => None,
        ProposalState::Written => Some("Written"),
        ProposalState::Discarded => Some("Discarded"),
        ProposalState::Gone => Some("No longer available"),
    }
}

#[component]
pub fn ProposedChangeCard(
    title: String,
    files: Vec<(PathBuf, u32, u32)>,
    state: ProposalState,
    /// (this step, steps in all): the frame's "Step 1 of 2".
    step: (usize, usize),
    #[props(default)] on_review: Option<EventHandler<()>>,
) -> Element {
    let (index, total) = step;
    let note = state_note(state);
    let reviewable = state == ProposalState::Pending;
    rsx! {
        section { class: "proposal-card", "aria-label": "Proposed change: {title}",
            p { class: "proposal-step", "Step {index} of {total}" }
            h3 { class: "proposal-title", "{title}" }
            p { class: "proposal-totals", {proposal_totals(&files)} }
            div { class: "proposal-files",
                for (path, added, removed) in &files {
                    div { key: "{path.display()}", class: "proposal-file",
                        span { class: "proposal-file-icon",
                            Icon { name: "file-text", size: IconSize::Sm }
                        }
                        span { class: "proposal-file-path", "{path.display()}" }
                        span { class: "proposal-file-added", "+{added}" }
                        span { class: "proposal-file-removed", "\u{2212}{removed}" }
                    }
                }
            }
            div { class: "proposal-actions",
                Button {
                    label: "Review changes",
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Sm,
                    disabled: !reviewable,
                    disabled_reason: note.map(ToString::to_string),
                    onclick: move |_| {
                        if let Some(handler) = &on_review {
                            handler.call(());
                        }
                    },
                }
                if let Some(note) = note {
                    span { class: "proposal-state", "{note}" }
                }
            }
        }
    }
}
