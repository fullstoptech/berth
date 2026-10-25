use dioxus::prelude::*;

use super::icon::{Icon, IconSize};

/// One result row of the command palette: icon, label (verb + object or
/// record name), muted scope text, and the shortcut or a disabled reason on
/// the right. Presentation only; the palette owns selection and running.
#[component]
pub fn PaletteItem(
    /// DOM id, so the palette can scroll the active row into view.
    id: String,
    icon: &'static str,
    title: String,
    #[props(default)] meta: String,
    #[props(default)] shortcut: Option<String>,
    #[props(default)] active: bool,
    #[props(default)] danger: bool,
    /// When set the row cannot run; the text replaces the shortcut.
    #[props(default)]
    disabled_reason: Option<String>,
    #[props(default)] onclick: Option<EventHandler<MouseEvent>>,
    #[props(default)] onhover: Option<EventHandler<MouseEvent>>,
) -> Element {
    let mut class = String::from("palette-item");
    if active {
        class.push_str(" palette-item-active");
    }
    if danger {
        class.push_str(" palette-item-danger");
    }
    let disabled = disabled_reason.is_some();
    if disabled {
        class.push_str(" palette-item-disabled");
    }
    let hint = disabled_reason.clone().or(shortcut);
    rsx! {
        div {
            id: "{id}",
            class: "{class}",
            role: "option",
            "aria-selected": "{active}",
            "aria-disabled": "{disabled}",
            title: disabled_reason.unwrap_or_default(),
            onmousemove: move |e| {
                if let Some(handler) = &onhover {
                    handler.call(e);
                }
            },
            onclick: move |e| {
                if !disabled {
                    if let Some(handler) = &onclick {
                        handler.call(e);
                    }
                }
            },
            span { class: "palette-item-icon",
                Icon { name: icon, size: IconSize::Md }
            }
            span { class: "palette-item-title", "{title}" }
            if !meta.is_empty() {
                span { class: "palette-item-meta", "{meta}" }
            }
            if let Some(hint) = hint {
                span { class: "palette-item-hint", "{hint}" }
            }
        }
    }
}
