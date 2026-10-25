use dioxus::prelude::*;

use super::badge::{Badge, BadgeKind};
use super::icon::{Icon, IconSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabsVariant {
    #[default]
    Underline,
    /// The bottom panel's tabs: carry a count or a close ×.
    Panel,
}

impl TabsVariant {
    pub fn class(self) -> &'static str {
        match self {
            Self::Underline => "tabs tabs-underline",
            Self::Panel => "tabs tabs-panel",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabItem {
    pub id: String,
    pub label: String,
    pub count: Option<usize>,
    pub closable: bool,
    /// Quiet text after the label ("1 pulling"); not a count.
    pub note: Option<String>,
}

impl TabItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            count: None,
            closable: false,
            note: None,
        }
    }
    pub fn with_count(id: impl Into<String>, label: impl Into<String>, count: usize) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            count: Some(count),
            closable: false,
            note: None,
        }
    }
    pub fn closable(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            count: None,
            closable: true,
            note: None,
        }
    }

    /// The tab with quiet text after its label.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// Peer views of one object (inspector, bottom panel). Not steps.
/// **Controlled**: `selected` id + `on_select`; panel close buttons report
/// through `on_close` and each has the accessible name "Close <label>".
#[component]
pub fn Tabs(
    items: Vec<TabItem>,
    selected: String,
    #[props(default)] variant: TabsVariant,
    #[props(default)] aria_label: Option<String>,
    #[props(default)] on_select: Option<EventHandler<String>>,
    #[props(default)] on_close: Option<EventHandler<String>>,
) -> Element {
    rsx! {
        div { class: variant.class(), role: "tablist", "aria-label": aria_label,
            for item in &items {
                {
                    let is_selected = item.id == selected;
                    let class = if is_selected { "tab tab-selected" } else { "tab" };
                    let select_id = item.id.clone();
                    let close_id = item.id.clone();
                    let close_label = format!("Close {}", item.label);
                    rsx! {
                        span { key: "{item.id}", class: "tab-wrap",
                            button {
                                class,
                                r#type: "button",
                                role: "tab",
                                "aria-selected": is_selected.to_string(),
                                onclick: move |_| {
                                    if let Some(handler) = &on_select {
                                        handler.call(select_id.clone());
                                    }
                                },
                                span { "{item.label}" }
                                if let Some(note) = &item.note {
                                    span { class: "tab-note", "{note}" }
                                }
                                if let Some(count) = item.count {
                                    Badge { kind: BadgeKind::Count, text: count.to_string() }
                                }
                            }
                            if item.closable {
                                button {
                                    class: "tab-close",
                                    r#type: "button",
                                    "aria-label": "{close_label}",
                                    onclick: move |_| {
                                        if let Some(handler) = &on_close {
                                            handler.call(close_id.clone());
                                        }
                                    },
                                    Icon { name: "x", size: IconSize::Sm }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
