//! Result of a row action, shown under the row (PLAN D34). Tones are only
//! danger and warning per components-spec; never page-level (use Banner).

use dioxus::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InlineTone {
    #[default]
    Danger,
    Warning,
}

impl InlineTone {
    pub fn class(self) -> &'static str {
        match self {
            InlineTone::Danger => "tone-danger",
            InlineTone::Warning => "tone-warning",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            InlineTone::Danger => "alert-circle",
            InlineTone::Warning => "alert-triangle",
        }
    }
}

#[component]
pub fn InlineMessage(message: String, #[props(default)] tone: InlineTone) -> Element {
    let tone_class = tone.class();
    rsx! {
        div {
            class: "inline-message {tone_class}",
            role: "alert",
            svg { class: "inline-message-icon", "aria-hidden": "true",
                r#use { href: "/icons.svg#{tone.icon()}" }
            }
            span { class: "inline-message-text", "{message}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_inline_message_ssr_danger_shows_engine_text() {
        let html = dioxus_ssr::render_element(rsx! {
            InlineMessage {
                message: "Bind for 0.0.0.0:5432 failed: port is already allocated".to_string(),
            }
        });
        assert!(html.contains("inline-message tone-danger"), "{html}");
        assert!(html.contains("/icons.svg#alert-circle"), "{html}");
        assert!(html.contains("port is already allocated"), "{html}");
        assert!(html.contains("role=\"alert\""), "{html}");
    }

    #[test]
    fn feedback_inline_message_ssr_warning_tone() {
        let html = dioxus_ssr::render_element(rsx! {
            InlineMessage {
                message: "Image is amd64; it runs under emulation.".to_string(),
                tone: InlineTone::Warning,
            }
        });
        assert!(html.contains("tone-warning"), "{html}");
        assert!(html.contains("/icons.svg#alert-triangle"), "{html}");
        assert!(!html.contains("tone-danger"), "{html}");
    }
}
