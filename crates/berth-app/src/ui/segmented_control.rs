use dioxus::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentedItem {
    pub id: String,
    pub label: String,
    pub count: Option<usize>,
}

impl SegmentedItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            count: None,
        }
    }
    pub fn with_count(id: impl Into<String>, label: impl Into<String>, count: usize) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            count: Some(count),
        }
    }
}

/// 2–4 peer views or modes, optionally with counts. Not navigation.
/// **Controlled**: `selected` id + `on_select` callback.
/// (The frame's multi-select variant for log levels lands with the log
/// toolbar in P3; this is the single-select control.)
#[component]
pub fn SegmentedControl(
    items: Vec<SegmentedItem>,
    selected: String,
    #[props(default)] aria_label: Option<String>,
    #[props(default)] on_select: Option<EventHandler<String>>,
) -> Element {
    rsx! {
        div { class: "segmented", role: "group", "aria-label": aria_label,
            for item in &items {
                {
                    let is_selected = item.id == selected;
                    let class = if is_selected { "segment segment-selected" } else { "segment" };
                    let item_id = item.id.clone();
                    rsx! {
                        button {
                            key: "{item.id}",
                            class,
                            r#type: "button",
                            "aria-pressed": is_selected.to_string(),
                            onclick: move |_| {
                                if let Some(handler) = &on_select {
                                    handler.call(item_id.clone());
                                }
                            },
                            span { "{item.label}" }
                            if let Some(count) = item.count {
                                span { class: "segment-count", "{count}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
