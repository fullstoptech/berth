//! Header for a group of table rows (PLAN 7.3): chevron, name, counts like
//! "3 running · 1 exited", and one group action ("Start 1 stopped").

use dioxus::prelude::*;

/// Build the counts text from `(count, word)` parts, skipping zero counts.
/// `[(3, "running"), (1, "exited"), (0, "paused")]` → `"3 running · 1 exited"`.
pub fn count_label(parts: &[(usize, &str)]) -> String {
    parts
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, word)| format!("{n} {word}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[component]
pub fn GroupHeaderRow(
    name: String,
    counts: String,
    #[props(default = true)] expanded: bool,
    action_label: Option<String>,
    ontoggle: Option<EventHandler<()>>,
    onaction: Option<EventHandler<()>>,
) -> Element {
    let chevron = if expanded {
        "chevron-down"
    } else {
        "chevron-right"
    };
    let state_class = if expanded { "expanded" } else { "collapsed" };
    let toggle_name = name.clone();
    rsx! {
        div { class: "group-header-row {state_class}", role: "row",
            button {
                class: "group-header-toggle",
                r#type: "button",
                "aria-expanded": "{expanded}",
                "aria-label": "Toggle group {toggle_name}",
                onclick: move |_| {
                    if let Some(handler) = ontoggle {
                        handler.call(());
                    }
                },
                svg { class: "group-header-chevron", "aria-hidden": "true",
                    r#use { href: "/icons.svg#{chevron}" }
                }
                svg { class: "group-header-icon", "aria-hidden": "true",
                    r#use { href: "/icons.svg#folder" }
                }
                span { class: "group-header-name", "{name}" }
            }
            span { class: "group-header-counts", "{counts}" }
            span { class: "group-header-spacer" }
            if let Some(label) = action_label {
                button {
                    class: "group-header-action",
                    r#type: "button",
                    onclick: move |_| {
                        if let Some(handler) = onaction {
                            handler.call(());
                        }
                    },
                    "{label}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_group_header_count_label_joins_and_skips_zero() {
        assert_eq!(
            count_label(&[(3, "running"), (1, "exited")]),
            "3 running · 1 exited"
        );
        assert_eq!(
            count_label(&[(0, "running"), (2, "exited"), (0, "paused")]),
            "2 exited"
        );
        assert_eq!(count_label(&[]), "");
        assert_eq!(count_label(&[(0, "running")]), "");
    }

    #[test]
    fn feedback_group_header_ssr_expanded_shows_counts_and_action() {
        let html = dioxus_ssr::render_element(rsx! {
            GroupHeaderRow {
                name: "shop-api".to_string(),
                counts: count_label(&[(3, "running"), (1, "restarting"), (1, "exited")]),
                expanded: true,
                action_label: Some("Start 1 stopped".to_string()),
            }
        });
        assert!(html.contains("shop-api"), "{html}");
        assert!(
            html.contains("3 running · 1 restarting · 1 exited"),
            "{html}"
        );
        assert!(html.contains("Start 1 stopped"), "{html}");
        assert!(html.contains("/icons.svg#chevron-down"), "{html}");
        assert!(html.contains("expanded"), "{html}");
    }

    #[test]
    fn feedback_group_header_ssr_collapsed_points_right() {
        let html = dioxus_ssr::render_element(rsx! {
            GroupHeaderRow {
                name: "ml-notebooks".to_string(),
                counts: "2 exited".to_string(),
                expanded: false,
                action_label: Some("Start 2 stopped".to_string()),
            }
        });
        assert!(html.contains("/icons.svg#chevron-right"), "{html}");
        assert!(!html.contains("chevron-down"), "{html}");
        assert!(html.contains("collapsed"), "{html}");
        assert!(html.contains("2 exited"), "{html}");
    }
}
