use dioxus::prelude::*;

/// Immediate on/off (Follow, Wrap, Timestamps) — not for options that need
/// Save. **Controlled**: `on` + `on_toggle(next)`. `role="switch"`.
#[component]
pub fn Switch(
    on: bool,
    #[props(default)] label: Option<String>,
    #[props(default)] aria_label: Option<String>,
    #[props(default)] disabled: bool,
    #[props(default)] on_toggle: Option<EventHandler<bool>>,
) -> Element {
    let track_class = if on {
        "switch-track switch-on"
    } else {
        "switch-track"
    };
    rsx! {
        button {
            class: "switch",
            r#type: "button",
            role: "switch",
            "aria-checked": on.to_string(),
            "aria-label": aria_label,
            disabled,
            onclick: move |_| {
                if let Some(handler) = &on_toggle {
                    handler.call(!on);
                }
            },
            span { class: track_class,
                span { class: "switch-thumb" }
            }
            if let Some(text) = &label {
                span { class: "switch-label", "{text}" }
            }
        }
    }
}
