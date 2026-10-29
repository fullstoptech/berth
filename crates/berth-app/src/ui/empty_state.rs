//! First-run / no-results state (components-spec: icon, title, one-line
//! payoff, one primary action). Frames: "Projects / empty", "Containers /
//! engine stopped". Plain button markup — no primitive Button dependency.

use dioxus::prelude::*;

#[component]
pub fn EmptyState(
    title: String,
    body: String,
    #[props(default = "folder")] icon: &'static str,
    action_label: Option<String>,
    hint: Option<String>,
    onaction: Option<EventHandler<()>>,
    /// The action is shown but cannot be clicked (it is already running).
    #[props(default)]
    action_disabled: bool,
) -> Element {
    rsx! {
        div { class: "empty-state",
            svg { class: "empty-state-icon", "aria-hidden": "true",
                r#use { href: "/icons.svg#{icon}" }
            }
            div { class: "empty-state-title", "{title}" }
            div { class: "empty-state-body", "{body}" }
            if let Some(label) = action_label {
                button {
                    class: "empty-state-action",
                    r#type: "button",
                    disabled: action_disabled,
                    "aria-busy": if action_disabled { "true" } else { "false" },
                    onclick: move |_| {
                        if let Some(handler) = onaction {
                            handler.call(());
                        }
                    },
                    "{label}"
                }
            }
            if let Some(hint) = hint {
                div { class: "empty-state-hint", "{hint}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_empty_state_ssr_engine_stopped_copy_from_frame() {
        let html = dioxus_ssr::render_element(rsx! {
            EmptyState {
                title: "Start the engine to see its containers".to_string(),
                body: "Start it to see its 23 containers, 31 images and 12 volumes.".to_string(),
                icon: "server",
                action_label: Some("Start engine default".to_string()),
                hint: Some("colima start default · 4 CPU · 8 GB · about 20 s".to_string()),
            }
        });
        assert!(html.contains("empty-state-title"), "{html}");
        assert!(
            html.contains("Start the engine to see its containers"),
            "{html}"
        );
        assert!(
            html.contains("Start it to see its 23 containers, 31 images and 12 volumes."),
            "{html}"
        );
        assert!(html.contains("Start engine default"), "{html}");
        assert!(html.contains("/icons.svg#server"), "{html}");
        assert!(html.contains("colima start default"), "{html}");
    }

    #[test]
    fn feedback_empty_state_ssr_projects_empty_copy_from_frame() {
        let html = dioxus_ssr::render_element(rsx! {
            EmptyState {
                title: "No projects yet".to_string(),
                body: "Open a folder and Berth finds its Compose file, or writes one for you.".to_string(),
                action_label: Some("Open folder".to_string()),
            }
        });
        assert!(html.contains("No projects yet"), "{html}");
        assert!(html.contains("Open folder"), "{html}");
        assert!(html.contains("/icons.svg#folder"), "{html}");
    }

    #[test]
    fn feedback_empty_state_ssr_action_can_be_disabled_while_it_runs() {
        let html = dioxus_ssr::render_element(rsx! {
            EmptyState {
                title: "Starting Colima default".to_string(),
                body: "Wait.".to_string(),
                action_label: Some("Starting".to_string()),
                action_disabled: true,
            }
        });
        assert!(html.contains("disabled"), "{html}");
        let live = dioxus_ssr::render_element(rsx! {
            EmptyState {
                title: "Start".to_string(),
                body: "Go.".to_string(),
                action_label: Some("Start".to_string()),
            }
        });
        assert!(!live.contains("disabled"), "{live}");
    }

    #[test]
    fn feedback_empty_state_ssr_without_action_has_no_button() {
        let html = dioxus_ssr::render_element(rsx! {
            EmptyState {
                title: "No containers match".to_string(),
                body: "Try a different filter.".to_string(),
            }
        });
        assert!(html.contains("No containers match"), "{html}");
        assert!(!html.contains("<button"), "{html}");
        assert!(!html.contains("empty-state-hint"), "{html}");
    }
}
