use dioxus::prelude::*;

use super::icon::{Icon, IconSize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

impl SelectOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

/// One of many (for 2–4 options use `SegmentedControl`). A native `<select>`
/// (PLAN 7.3) styled to the tokens, **controlled**: `value` + `onchange`.
#[component]
pub fn Select(
    options: Vec<SelectOption>,
    value: String,
    #[props(default)] label: Option<String>,
    #[props(default)] disabled: bool,
    #[props(default)] id: Option<String>,
    #[props(default)] aria_label: Option<String>,
    #[props(default)] onchange: Option<EventHandler<FormEvent>>,
) -> Element {
    rsx! {
        div { class: "select-field",
            if let Some(label_text) = &label {
                label { class: "field-label", r#for: id.clone(), "{label_text}" }
            }
            div { class: "select-control",
                select {
                    class: "select-input",
                    id: id.clone(),
                    disabled,
                    "aria-label": aria_label,
                    onchange: move |e| {
                        if let Some(handler) = &onchange {
                            handler.call(e);
                        }
                    },
                    for opt in &options {
                        option {
                            key: "{opt.value}",
                            value: "{opt.value}",
                            selected: opt.value == value,
                            "{opt.label}"
                        }
                    }
                }
                span { class: "select-chevron",
                    Icon { name: "chevron-down", size: IconSize::Sm }
                }
            }
        }
    }
}
