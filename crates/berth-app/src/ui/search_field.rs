use dioxus::prelude::*;

use super::icon::{Icon, IconSize};

/// Filters a list (not global search — that is the palette). **Uncontrolled**
/// (D59): `initial_value` seeds the input; `oninput` reports keystrokes.
/// Shows the match count as "12 of 48" (danger tone at "0 of 48") and a
/// clear button once there is text or a count. Clearing is the parent's job:
/// it remounts the field (keyed one-item `for`) with an empty initial value.
#[component]
pub fn SearchField(
    #[props(default)] initial_value: String,
    #[props(default = "Filter".to_string())] placeholder: String,
    #[props(default)] matched: Option<usize>,
    #[props(default)] total: Option<usize>,
    #[props(default)] disabled: bool,
    #[props(default = "Filter".to_string())] aria_label: String,
    #[props(default)] oninput: Option<EventHandler<FormEvent>>,
    #[props(default)] onclear: Option<EventHandler<MouseEvent>>,
) -> Element {
    let count_text = match (matched, total) {
        (Some(m), Some(t)) => Some(format!("{m} of {t}")),
        _ => None,
    };
    let no_matches = matched == Some(0) && total.is_some_and(|t| t > 0);
    let show_clear = !initial_value.is_empty() || count_text.is_some();
    let count_class = if no_matches {
        "search-count search-count-none"
    } else {
        "search-count"
    };
    rsx! {
        div { class: "search-field",
            span { class: "field-prefix",
                Icon { name: "search", size: IconSize::Sm }
            }
            input {
                class: "search-input",
                r#type: "search",
                initial_value: "{initial_value}",
                placeholder: "{placeholder}",
                disabled,
                "aria-label": "{aria_label}",
                // Plain typing must not reach the root shortcut handler
                // (keys.rs cannot see focus); shortcuts with Cmd/Ctrl pass.
                onkeydown: move |e| {
                    use dioxus::html::ModifiersInteraction;
                    let mods = e.data.modifiers();
                    if !mods.contains(dioxus::html::Modifiers::META)
                        && !mods.contains(dioxus::html::Modifiers::CONTROL)
                    {
                        e.stop_propagation();
                    }
                },
                oninput: move |e| {
                    if let Some(handler) = &oninput {
                        handler.call(e);
                    }
                },
            }
            if let Some(text) = &count_text {
                span { class: count_class, "{text}" }
            }
            if show_clear {
                button {
                    class: "search-clear",
                    r#type: "button",
                    "aria-label": "Clear filter",
                    disabled,
                    onclick: move |e| {
                        if let Some(handler) = &onclear {
                            handler.call(e);
                        }
                    },
                    Icon { name: "x", size: IconSize::Sm }
                }
            }
        }
    }
}
