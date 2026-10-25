use dioxus::prelude::*;

use super::icon::{Icon, IconSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckState {
    #[default]
    Off,
    On,
    Mixed,
}

impl CheckState {
    pub fn aria_checked(self) -> &'static str {
        match self {
            Self::Off => "false",
            Self::On => "true",
            Self::Mixed => "mixed",
        }
    }
    pub fn from_bool(on: bool) -> Self {
        if on {
            Self::On
        } else {
            Self::Off
        }
    }
}

/// Row selection and options. Square, radius sm. **Controlled**:
/// `state` + `on_toggle(next_checked)`. Renders as a `role="checkbox"`
/// button so the mixed state is real markup, not a script trick.
#[component]
pub fn Checkbox(
    state: CheckState,
    #[props(default)] label: Option<String>,
    #[props(default)] aria_label: Option<String>,
    #[props(default)] disabled: bool,
    #[props(default)] on_toggle: Option<EventHandler<bool>>,
) -> Element {
    let box_class = match state {
        CheckState::Off => "checkbox-box",
        CheckState::On => "checkbox-box checkbox-on",
        CheckState::Mixed => "checkbox-box checkbox-mixed",
    };
    let glyph = match state {
        CheckState::On => Some("check"),
        CheckState::Mixed => Some("minus"),
        CheckState::Off => None,
    };
    rsx! {
        button {
            class: "checkbox",
            r#type: "button",
            role: "checkbox",
            "aria-checked": state.aria_checked(),
            "aria-label": aria_label,
            disabled,
            onclick: move |_| {
                if let Some(handler) = &on_toggle {
                    handler.call(state != CheckState::On);
                }
            },
            span { class: box_class,
                if let Some(name) = glyph {
                    Icon { name, size: IconSize::Sm }
                }
            }
            if let Some(text) = &label {
                span { class: "checkbox-label", "{text}" }
            }
        }
    }
}
