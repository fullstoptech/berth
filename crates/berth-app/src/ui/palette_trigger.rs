use dioxus::prelude::*;

use super::icon::{Icon, IconSize};
use super::kbd::{Kbd, KbdSymbol};
use crate::state::{AppState, Overlay};

/// The title bar's one global action: opens the command palette (⌘K).
/// It is a button, not a field — typing happens in the palette itself.
#[component]
pub fn PaletteTrigger() -> Element {
    let mut state = use_context::<AppState>();
    let mod_symbol = if crate::actions::mod_label() == "\u{2318}" {
        KbdSymbol::Cmd
    } else {
        KbdSymbol::Ctrl
    };
    rsx! {
        button {
            class: "palette-trigger",
            r#type: "button",
            "aria-label": "Search or run a command",
            onmousedown: move |e| e.stop_propagation(),
            ondoubleclick: move |e| e.stop_propagation(),
            onclick: move |_| {
                let open = state.overlays.peek().iter().any(|o| matches!(o, Overlay::Palette));
                if !open {
                    state.overlays.write().push(Overlay::Palette);
                }
            },
            Icon { name: "search", size: IconSize::Md }
            span { class: "palette-trigger-text", "Search or run a command\u{2026}" }
            span { class: "palette-trigger-keys",
                Kbd { symbol: mod_symbol }
                Kbd { symbol: KbdSymbol::Letter("K".to_string()) }
            }
        }
    }
}
