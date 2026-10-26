//! One attached piece of context above the chat input: icon, label and
//! the × that detaches it (frame "Project / assistant / 1440").

use dioxus::prelude::*;

use super::icon::{Icon, IconSize};

#[component]
pub fn ContextChip(
    icon: &'static str,
    label: String,
    #[props(default)] on_remove: Option<EventHandler<()>>,
) -> Element {
    rsx! {
        span { class: "context-chip",
            span { class: "context-chip-icon",
                Icon { name: icon, size: IconSize::Sm }
            }
            span { class: "context-chip-label", "{label}" }
            button {
                class: "context-chip-x",
                r#type: "button",
                "aria-label": "Remove {label}",
                onclick: move |_| {
                    if let Some(handler) = &on_remove {
                        handler.call(());
                    }
                },
                Icon { name: "x", size: IconSize::Sm }
            }
        }
    }
}
