use dioxus::prelude::*;

use super::icon::{Icon, IconSize};
use super::kbd::{Kbd, KbdSymbol};

/// Free input. **Uncontrolled** (D59): the caller passes `initial_value`
/// once and reads changes through `oninput`/`onchange`; the value is never
/// bound back to a signal, so typing never fights a re-render.
/// Label above, help or error below.
#[component]
pub fn TextField(
    #[props(default)] label: Option<String>,
    #[props(default)] initial_value: String,
    #[props(default)] placeholder: Option<String>,
    #[props(default)] help: Option<String>,
    #[props(default)] error: Option<String>,
    #[props(default)] disabled: bool,
    #[props(default)] prefix_icon: Option<&'static str>,
    #[props(default)] kbd: Option<KbdSymbol>,
    #[props(default = "text".to_string())] input_type: String,
    #[props(default)] id: Option<String>,
    #[props(default)] aria_label: Option<String>,
    #[props(default)] oninput: Option<EventHandler<FormEvent>>,
    #[props(default)] onchange: Option<EventHandler<FormEvent>>,
) -> Element {
    let has_error = error.is_some();
    let group_class = if has_error {
        "text-field text-field-error"
    } else {
        "text-field"
    };
    rsx! {
        div { class: group_class,
            if let Some(label_text) = &label {
                label { class: "field-label", r#for: id.clone(), "{label_text}" }
            }
            div { class: "field-control",
                if let Some(name) = prefix_icon {
                    span { class: "field-prefix",
                        Icon { name, size: IconSize::Sm }
                    }
                }
                input {
                    class: "field-input",
                    r#type: "{input_type}",
                    id: id.clone(),
                    initial_value: "{initial_value}",
                    placeholder,
                    disabled,
                    "aria-label": aria_label,
                    "aria-invalid": has_error.to_string(),
                    // Plain typing must not reach the root shortcut handler (the
                    // focus rule in keys.rs also holds the bare keys back; this is the
                    // second guard). Shortcuts with Cmd/Ctrl pass, and so does Escape,
                    // so a sheet closes from inside its fields.
                    onkeydown: move |e| {
                        use dioxus::html::ModifiersInteraction;
                        let mods = e.data.modifiers();
                        if !mods.contains(dioxus::html::Modifiers::META)
                            && !mods.contains(dioxus::html::Modifiers::CONTROL)
                            && e.data.key() != Key::Escape
                        {
                            e.stop_propagation();
                        }
                    },
                    oninput: move |e| {
                        if let Some(handler) = &oninput {
                            handler.call(e);
                        }
                    },
                    onchange: move |e| {
                        if let Some(handler) = &onchange {
                            handler.call(e);
                        }
                    },
                }
                if let Some(symbol) = kbd {
                    span { class: "field-suffix",
                        Kbd { symbol }
                    }
                }
            }
            if let Some(message) = &error {
                p { class: "field-error-text", role: "alert",
                    Icon { name: "alert-circle", size: IconSize::Sm }
                    span { "{message}" }
                }
            } else if let Some(help_text) = &help {
                p { class: "field-help", "{help_text}" }
            }
        }
    }
}
