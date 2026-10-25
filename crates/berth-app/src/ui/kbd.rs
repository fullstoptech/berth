use dioxus::prelude::*;

use super::icon::{Icon, IconSize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KbdSymbol {
    Cmd,
    Shift,
    Ctrl,
    Option,
    Backspace,
    Return,
    Letter(String),
}

impl KbdSymbol {
    fn icon(&self) -> Option<&'static str> {
        match self {
            Self::Cmd => Some("command"),
            Self::Shift => Some("shift"),
            Self::Option => Some("option"),
            Self::Backspace => Some("backspace"),
            Self::Return => Some("corner-down-left"),
            Self::Ctrl | Self::Letter(_) => None,
        }
    }
    fn text(&self) -> &str {
        match self {
            Self::Ctrl => "^",
            Self::Letter(s) => s,
            _ => "",
        }
    }
    pub fn accessible_name(&self) -> String {
        match self {
            Self::Cmd => "Command \u{2318}".to_string(),
            Self::Shift => "Shift \u{21e7}".to_string(),
            Self::Ctrl => "Control \u{2303}".to_string(),
            Self::Option => "Option \u{2325}".to_string(),
            Self::Backspace => "Backspace \u{232b}".to_string(),
            Self::Return => "Return \u{21b5}".to_string(),
            Self::Letter(s) => s.clone(),
        }
    }
    pub fn data_symbol(&self) -> &'static str {
        match self {
            Self::Cmd => "cmd",
            Self::Shift => "shift",
            Self::Ctrl => "ctrl",
            Self::Option => "option",
            Self::Backspace => "backspace",
            Self::Return => "return",
            Self::Letter(_) => "letter",
        }
    }
}

/// Shortcut hint (not a button). Glyphs are icons from the sprite (the symbol
/// fonts lack them, per the Components page); letters use the label style.
/// The Cmd key carries the `⌘` symbol in its accessible name.
#[component]
pub fn Kbd(symbol: KbdSymbol) -> Element {
    let name = symbol.accessible_name();
    rsx! {
        kbd { class: "kbd", "data-symbol": symbol.data_symbol(), "aria-label": "{name}",
            if let Some(icon) = symbol.icon() {
                Icon { name: icon, size: IconSize::Sm }
            } else {
                "{symbol.text()}"
            }
        }
    }
}

/// Convenience: a `⌘` + letter shortcut pair, the common tooltip shape.
#[component]
pub fn KbdShortcut(letter: String) -> Element {
    rsx! {
        span { class: "kbd-shortcut",
            Kbd { symbol: KbdSymbol::Cmd }
            Kbd { symbol: KbdSymbol::Letter(letter) }
        }
    }
}
