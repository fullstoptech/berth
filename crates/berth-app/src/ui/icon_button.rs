use dioxus::prelude::*;

use super::icon::{Icon, IconSize};
use super::ControlSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconButtonTone {
    #[default]
    Neutral,
    /// Neutral at rest; turns danger on hover (Components page).
    Danger,
}

impl IconButtonTone {
    pub fn class(self) -> &'static str {
        match self {
            Self::Neutral => "icon-btn-neutral",
            Self::Danger => "icon-btn-danger",
        }
    }
}

/// Row and toolbar command, never the primary action. `label` is the
/// required accessible name ("Restart api-db-1") — there is no default.
/// `tooltip` is pure-CSS hover text ("Restart · ⌘R"). `pressed` renders
/// the toggle "on" state (Follow, Wrap, Timestamps).
#[component]
pub fn IconButton(
    icon: &'static str,
    label: String,
    #[props(default)] tone: IconButtonTone,
    #[props(default)] size: ControlSize,
    #[props(default)] tooltip: Option<String>,
    #[props(default)] disabled: bool,
    #[props(default)] loading: bool,
    #[props(default)] pressed: Option<bool>,
    #[props(default)] onclick: Option<EventHandler<MouseEvent>>,
) -> Element {
    let on = pressed == Some(true);
    let class = format!(
        "icon-btn {} {}{}",
        tone.class(),
        size.class(),
        if on { " icon-btn-on" } else { "" }
    );
    let icon_name = if loading { "loader" } else { icon };
    rsx! {
        button {
            class,
            r#type: "button",
            "aria-label": "{label}",
            "aria-pressed": pressed.map(|p| p.to_string()),
            "aria-busy": loading.to_string(),
            "data-tooltip": tooltip,
            disabled: disabled || loading,
            onclick: move |e| {
                if let Some(handler) = &onclick {
                    handler.call(e);
                }
            },
            span { class: if loading { "btn-spinner" } else { "" },
                Icon { name: icon_name, size: IconSize::Sm }
            }
        }
    }
}
