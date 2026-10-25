//! Persistent page/engine condition (components-spec: tone, icon, message,
//! one action; dismissible or not). Frame: "Containers / engine stopped".

use dioxus::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BannerTone {
    #[default]
    Info,
    Success,
    Warning,
    Danger,
}

impl BannerTone {
    pub fn class(self) -> &'static str {
        match self {
            BannerTone::Info => "tone-info",
            BannerTone::Success => "tone-success",
            BannerTone::Warning => "tone-warning",
            BannerTone::Danger => "tone-danger",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            BannerTone::Info => "info",
            BannerTone::Success => "circle-check",
            BannerTone::Warning => "alert-triangle",
            BannerTone::Danger => "alert-circle",
        }
    }

    pub fn role(self) -> &'static str {
        match self {
            BannerTone::Danger | BannerTone::Warning => "alert",
            BannerTone::Info | BannerTone::Success => "status",
        }
    }
}

#[component]
pub fn Banner(
    message: String,
    #[props(default)] tone: BannerTone,
    action_label: Option<String>,
    #[props(default)] dismissible: bool,
    onaction: Option<EventHandler<()>>,
    ondismiss: Option<EventHandler<()>>,
) -> Element {
    let mut dismissed = use_signal(|| false);
    if *dismissed.read() {
        return rsx! {};
    }
    let tone_class = tone.class();
    rsx! {
        div {
            class: "banner {tone_class}",
            role: "{tone.role()}",
            svg { class: "banner-icon", "aria-hidden": "true",
                r#use { href: "/icons.svg#{tone.icon()}" }
            }
            span { class: "banner-message", "{message}" }
            if let Some(label) = action_label {
                button {
                    class: "banner-action",
                    r#type: "button",
                    onclick: move |_| {
                        if let Some(handler) = onaction {
                            handler.call(());
                        }
                    },
                    "{label}"
                }
            }
            if dismissible {
                button {
                    class: "banner-dismiss",
                    r#type: "button",
                    "aria-label": "Dismiss",
                    onclick: move |_| {
                        dismissed.set(true);
                        if let Some(handler) = ondismiss {
                            handler.call(());
                        }
                    },
                    svg { "aria-hidden": "true",
                        r#use { href: "/icons.svg#x" }
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
    fn feedback_banner_every_tone_maps_to_class_icon_and_role() {
        let cases = [
            (BannerTone::Info, "tone-info", "info", "status"),
            (
                BannerTone::Success,
                "tone-success",
                "circle-check",
                "status",
            ),
            (
                BannerTone::Warning,
                "tone-warning",
                "alert-triangle",
                "alert",
            ),
            (BannerTone::Danger, "tone-danger", "alert-circle", "alert"),
        ];
        for (tone, class, icon, role) in cases {
            assert_eq!(tone.class(), class);
            assert_eq!(tone.icon(), icon);
            assert_eq!(tone.role(), role);
        }
    }

    #[test]
    fn feedback_banner_ssr_engine_stopped_warning_with_action() {
        let html = dioxus_ssr::render_element(rsx! {
            Banner {
                message: "Colima default is stopped. Containers on it are not running.".to_string(),
                tone: BannerTone::Warning,
                action_label: Some("Start engine".to_string()),
            }
        });
        assert!(html.contains("banner tone-warning"), "{html}");
        assert!(html.contains("/icons.svg#alert-triangle"), "{html}");
        assert!(
            html.contains("Colima default is stopped. Containers on it are not running."),
            "{html}"
        );
        assert!(html.contains("Start engine"), "{html}");
    }

    #[test]
    fn feedback_banner_ssr_without_action_or_dismiss_has_no_buttons() {
        let html = dioxus_ssr::render_element(rsx! {
            Banner {
                message: "Engine unreachable. Last seen 2 min ago.".to_string(),
                tone: BannerTone::Danger,
            }
        });
        assert!(html.contains("tone-danger"), "{html}");
        assert!(!html.contains("<button"), "{html}");
    }

    #[test]
    fn feedback_banner_ssr_dismissible_shows_dismiss_button() {
        let html = dioxus_ssr::render_element(rsx! {
            Banner {
                message: "Docker Hub limits anonymous searches.".to_string(),
                tone: BannerTone::Info,
                dismissible: true,
            }
        });
        assert!(html.contains("banner-dismiss"), "{html}");
        assert!(html.contains("Dismiss"), "{html}");
    }
}
