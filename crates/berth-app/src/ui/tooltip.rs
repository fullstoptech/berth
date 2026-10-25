use dioxus::prelude::*;

use super::kbd::{Kbd, KbdSymbol};

/// Name + shortcut of an icon button, or the reason a control is disabled.
/// Pure CSS: shown on hover/focus-within of the wrapped control.
#[component]
pub fn Tooltip(
    text: String,
    #[props(default)] kbd_letter: Option<String>,
    children: Element,
) -> Element {
    rsx! {
        span { class: "tooltip-wrap",
            {children}
            span { class: "tooltip", role: "tooltip",
                span { "{text}" }
                if let Some(letter) = kbd_letter {
                    Kbd { symbol: KbdSymbol::Cmd }
                    Kbd { symbol: KbdSymbol::Letter(letter) }
                }
            }
        }
    }
}
