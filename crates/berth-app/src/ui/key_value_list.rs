//! KeyValueList (components-spec): the details of one record, label muted
//! on the left, value (mono for technical text) on the right, a copy
//! button on hover. Not for comparison; a table does that.

use dioxus::prelude::*;

use super::icon::{Icon, IconSize};

/// Copy text through the webview's clipboard (CLAUDE.md: `document::eval`
/// plus `navigator.clipboard`).
pub fn copy_to_clipboard(text: &str) {
    let quoted = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
    let script = format!("navigator.clipboard.writeText({quoted});");
    spawn(async move {
        let _ = document::eval(&script).await;
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValueTone {
    #[default]
    Normal,
    /// An exit code other than 0, a failed check.
    Danger,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyValue {
    pub label: String,
    pub value: String,
    pub mono: bool,
    pub tone: ValueTone,
    /// Shown muted when `value` is empty ("Not set").
    pub empty_text: String,
}

impl KeyValue {
    /// A technical value (mono): ids, paths, commands, ports.
    pub fn mono(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            mono: true,
            tone: ValueTone::Normal,
            empty_text: "Not set".to_string(),
        }
    }

    /// A value in the text face.
    pub fn text(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            mono: false,
            ..Self::mono(label, value)
        }
    }

    #[must_use]
    pub fn danger(mut self) -> Self {
        self.tone = ValueTone::Danger;
        self
    }

    #[must_use]
    pub fn empty_text(mut self, text: impl Into<String>) -> Self {
        self.empty_text = text.into();
        self
    }
}

#[component]
pub fn KeyValueList(rows: Vec<KeyValue>) -> Element {
    rsx! {
        dl { class: "kv-list",
            for row in rows {
                {
                    let empty = row.value.is_empty();
                    let mut value_class = String::from("kv-value");
                    if row.mono && !empty {
                        value_class.push_str(" kv-mono");
                    }
                    if empty {
                        value_class.push_str(" kv-empty");
                    }
                    if row.tone == ValueTone::Danger {
                        value_class.push_str(" kv-danger");
                    }
                    let shown = if empty { row.empty_text.clone() } else { row.value.clone() };
                    let copy_value = row.value.clone();
                    let copy_label = format!("Copy {}", row.label.to_lowercase());
                    rsx! {
                        div { key: "{row.label}", class: "kv-row",
                            dt { class: "kv-label", "{row.label}" }
                            dd { class: value_class,
                                span { class: "kv-text", "{shown}" }
                                if !empty {
                                    button {
                                        class: "kv-copy",
                                        r#type: "button",
                                        "aria-label": "{copy_label}",
                                        onclick: move |_| copy_to_clipboard(&copy_value),
                                        Icon { name: "copy", size: IconSize::Sm }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_value_list_renders_label_value_and_copy_for_filled_rows() {
        let html = dioxus_ssr::render_element(rsx! {
            KeyValueList { rows: vec![KeyValue::mono("Image", "shop-api-worker:latest")] }
        });
        assert!(html.contains("Image"), "{html}");
        assert!(html.contains("shop-api-worker:latest"), "{html}");
        assert!(html.contains("kv-mono"), "{html}");
        assert!(html.contains("aria-label=\"Copy image\""), "{html}");
    }

    #[test]
    fn key_value_list_empty_value_shows_not_set_without_a_copy_button() {
        let html = dioxus_ssr::render_element(rsx! {
            KeyValueList { rows: vec![
                KeyValue::mono("User", ""),
                KeyValue::mono("Ports", "").empty_text("–"),
            ] }
        });
        assert!(html.contains("Not set"), "{html}");
        assert!(html.contains(">–<"), "{html}");
        assert!(html.contains("kv-empty"), "{html}");
        assert!(!html.contains("kv-copy"), "{html}");
    }

    #[test]
    fn key_value_list_danger_tone_marks_the_value() {
        let html = dioxus_ssr::render_element(rsx! {
            KeyValueList { rows: vec![KeyValue::mono("Exit code", "1").danger()] }
        });
        assert!(html.contains("kv-danger"), "{html}");
    }
}
