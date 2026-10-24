use dioxus::prelude::*;

use super::icon::{Icon, IconSize};
use super::kbd::{Kbd, KbdSymbol};
use super::ControlSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    Primary,
    #[default]
    Secondary,
    Ghost,
    Danger,
}

impl ButtonVariant {
    pub fn class(self) -> &'static str {
        match self {
            Self::Primary => "btn-primary",
            Self::Secondary => "btn-secondary",
            Self::Ghost => "btn-ghost",
            Self::Danger => "btn-danger",
        }
    }
}

/// A command with a label (verb + object), never navigation. `loading`
/// swaps the leading icon for a spinner and keeps the label (the caller
/// passes the pending label, e.g. "Stopping…"). A disabled button stays
/// visible; `disabled_reason` becomes its tooltip text.
#[component]
pub fn Button(
    label: String,
    #[props(default)] variant: ButtonVariant,
    #[props(default)] size: ControlSize,
    #[props(default)] icon: Option<&'static str>,
    #[props(default)] kbd_letter: Option<String>,
    #[props(default)] disabled: bool,
    #[props(default)] disabled_reason: Option<String>,
    #[props(default)] loading: bool,
    #[props(default)] onclick: Option<EventHandler<MouseEvent>>,
) -> Element {
    let class = format!("btn {} {}", variant.class(), size.class());
    let icon_name = if loading { Some("loader") } else { icon };
    rsx! {
        button {
            class,
            r#type: "button",
            disabled: disabled || loading,
            "aria-busy": loading.to_string(),
            "data-tooltip": disabled_reason,
            onclick: move |e| {
                if let Some(handler) = &onclick {
                    handler.call(e);
                }
            },
            if let Some(name) = icon_name {
                span { class: if loading { "btn-icon btn-spinner" } else { "btn-icon" },
                    Icon { name, size: IconSize::Sm }
                }
            }
            span { class: "btn-label", "{label}" }
            if let Some(letter) = kbd_letter {
                Kbd { symbol: KbdSymbol::Cmd }
                Kbd { symbol: KbdSymbol::Letter(letter) }
            }
        }
    }
}
