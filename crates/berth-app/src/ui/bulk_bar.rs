//! BulkBar (PLAN 7.3, components-spec): the floating bar that replaces the
//! Containers header while two or more rows are selected. Every action
//! shows how many of the selected containers it applies to ("Stop 3"); an
//! action that applies to none stays visible, disabled, with the reason.
//! The bar is markup over [`BulkCounts`]; the screen owns the callbacks.

use dioxus::prelude::*;

use berth_core::{ContainerId, Subject};

use super::icon::{Icon, IconSize};
use crate::services::actions::{disabled_reason, StateView};

/// The CSS of the bar, rendered with it (tokens only).
pub const BULK_BAR_CSS: &str = include_str!("../../assets/css/bulk_bar.css");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkAction {
    Start,
    Stop,
    Restart,
    Pause,
    Delete,
}

impl BulkAction {
    pub const ALL: [BulkAction; 5] = [
        Self::Start,
        Self::Stop,
        Self::Restart,
        Self::Pause,
        Self::Delete,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
            Self::Pause => "Pause",
            Self::Delete => "Delete",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Start => "play",
            Self::Stop => "square",
            Self::Restart => "rotate-cw",
            Self::Pause => "pause",
            Self::Delete => "trash-2",
        }
    }

    /// The `ActionDef` id a container must be allowed to run for the
    /// action to count it.
    pub fn action_id(self) -> &'static str {
        match self {
            Self::Start => "container.start",
            Self::Stop => "container.stop",
            Self::Restart => "container.restart",
            Self::Pause => "container.pause",
            Self::Delete => "container.delete",
        }
    }

    fn past(self) -> &'static str {
        match self {
            Self::Start => "started",
            Self::Stop => "stopped",
            Self::Restart => "restarted",
            Self::Pause => "paused",
            Self::Delete => "deleted",
        }
    }

    /// Why the action is off when it applies to none of the selection.
    pub fn none_reason(self) -> String {
        format!("None of the selected containers can be {}", self.past())
    }
}

/// How many of the selected containers each action applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BulkCounts {
    pub start: usize,
    pub stop: usize,
    pub restart: usize,
    pub pause: usize,
    pub delete: usize,
}

impl BulkCounts {
    pub fn get(&self, action: BulkAction) -> usize {
        match action {
            BulkAction::Start => self.start,
            BulkAction::Stop => self.stop,
            BulkAction::Restart => self.restart,
            BulkAction::Pause => self.pause,
            BulkAction::Delete => self.delete,
        }
    }

    /// "Start 1 · Stop 3 · Restart 4 · Pause 3 · Delete 4".
    pub fn text(&self) -> String {
        BulkAction::ALL
            .iter()
            .map(|a| format!("{} {}", a.label(), self.get(*a)))
            .collect::<Vec<_>>()
            .join(" \u{00b7} ")
    }
}

/// The selected ids `action` can run on right now, in selection order.
/// Uses the same `disabled_reason` the row buttons use, so the bar never
/// offers what a row would refuse.
#[must_use]
pub fn applicable(
    ids: &[ContainerId],
    action: BulkAction,
    view: &StateView<'_>,
) -> Vec<ContainerId> {
    ids.iter()
        .filter(|id| {
            disabled_reason(&Subject::Container((*id).clone()), action.action_id(), view).is_none()
        })
        .cloned()
        .collect()
}

#[must_use]
pub fn bulk_counts(ids: &[ContainerId], view: &StateView<'_>) -> BulkCounts {
    let count = |action| applicable(ids, action, view).len();
    BulkCounts {
        start: count(BulkAction::Start),
        stop: count(BulkAction::Stop),
        restart: count(BulkAction::Restart),
        pause: count(BulkAction::Pause),
        delete: count(BulkAction::Delete),
    }
}

#[component]
pub fn BulkBar(
    selected: usize,
    counts: BulkCounts,
    onaction: EventHandler<BulkAction>,
    onclear: EventHandler<()>,
) -> Element {
    let text = counts.text();
    rsx! {
        style { "{BULK_BAR_CSS}" }
        div { class: "bulk-bar-wrap",
            div {
                class: "bulk-bar",
                role: "toolbar",
                "aria-label": "Bulk actions: {text}",
                span { class: "bulk-selected", "{selected} selected" }
                span { class: "bulk-divider", "aria-hidden": "true" }
                for action in BulkAction::ALL {
                    {
                        let count = counts.get(action);
                        let disabled = count == 0;
                        let label = if action == BulkAction::Delete {
                            format!("{} {count}\u{2026}", action.label())
                        } else {
                            format!("{} {count}", action.label())
                        };
                        let class = match action {
                            BulkAction::Delete => "bulk-btn bulk-btn-danger",
                            _ => "bulk-btn",
                        };
                        rsx! {
                            button {
                                key: "{action.label()}",
                                class,
                                r#type: "button",
                                disabled,
                                "data-tooltip": if disabled { Some(action.none_reason()) } else { None },
                                onclick: move |e| {
                                    e.stop_propagation();
                                    onaction.call(action);
                                },
                                // The frame draws a disabled action as text only.
                                if !disabled {
                                    span { class: "bulk-btn-icon",
                                        Icon { name: action.icon(), size: IconSize::Sm }
                                    }
                                }
                                span { class: "bulk-btn-label", "{label}" }
                            }
                        }
                    }
                }
                button {
                    class: "bulk-btn bulk-clear",
                    r#type: "button",
                    onclick: move |e| {
                        e.stop_propagation();
                        onclear.call(());
                    },
                    "Clear"
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "bulk_bar_tests.rs"]
mod tests;
