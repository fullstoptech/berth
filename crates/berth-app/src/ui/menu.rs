use dioxus::prelude::*;

use super::icon::{Icon, IconSize};
use super::kbd::{Kbd, KbdSymbol};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    pub id: String,
    pub label: String,
    pub icon: Option<&'static str>,
    /// Shortcut letter shown with ⌘ (e.g. "T" for ⌘T).
    pub kbd_letter: Option<String>,
    pub danger: bool,
    /// When set the item is disabled and this reason replaces the shortcut
    /// on the right (frame: "Container is not running").
    pub disabled_reason: Option<String>,
    pub separator_before: bool,
}

impl MenuEntry {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            kbd_letter: None,
            danger: false,
            disabled_reason: None,
            separator_before: false,
        }
    }
    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn kbd(mut self, letter: impl Into<String>) -> Self {
        self.kbd_letter = Some(letter.into());
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn disabled(mut self, reason: impl Into<String>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }
    pub fn separator_before(mut self) -> Self {
        self.separator_before = true;
        self
    }
}

/// More-actions and context menus. Not navigation. Destructive items sit
/// last, in the danger tone, after a separator (the caller orders them).
///
/// Keyboard: items are native buttons (Tab reaches them today). Arrow-key
/// navigation is wired by the parent overlay in P2-T3 — this component is
/// the markup (`role="menu"`/`menuitem"`, `active` marks the hovered row).
#[component]
pub fn Menu(
    items: Vec<MenuEntry>,
    #[props(default)] active: Option<usize>,
    #[props(default)] aria_label: Option<String>,
    #[props(default)] on_select: Option<EventHandler<String>>,
) -> Element {
    rsx! {
        div { class: "menu", role: "menu", "aria-label": aria_label,
            for (index, item) in items.iter().enumerate() {
                {
                    let disabled = item.disabled_reason.is_some();
                    let mut class = "menu-item".to_string();
                    if item.danger {
                        class.push_str(" menu-item-danger");
                    }
                    if active == Some(index) {
                        class.push_str(" menu-item-active");
                    }
                    let item_id = item.id.clone();
                    rsx! {
                        div { key: "{item.id}", class: "menu-entry",
                        if item.separator_before {
                            div { class: "menu-separator", role: "separator" }
                        }
                        button {
                            class,
                            r#type: "button",
                            role: "menuitem",
                            disabled,
                            "aria-disabled": disabled.to_string(),
                            onclick: move |_| {
                                if let Some(handler) = &on_select {
                                    handler.call(item_id.clone());
                                }
                            },
                            if let Some(name) = item.icon {
                                Icon { name, size: IconSize::Sm }
                            }
                            span { class: "menu-item-label", "{item.label}" }
                            if let Some(reason) = &item.disabled_reason {
                                span { class: "menu-item-reason", "{reason}" }
                            } else if let Some(letter) = &item.kbd_letter {
                                span { class: "menu-item-kbd",
                                    Kbd { symbol: KbdSymbol::Cmd }
                                    Kbd { symbol: KbdSymbol::Letter(letter.clone()) }
                                }
                            }
                        }
                        }
                    }
                }
            }
        }
    }
}
