//! Loading placeholder rows (components-spec: shapes matching ContainerRow;
//! "Containers / loading / 1440" shows 8). Shown after 300 ms by callers.

use dioxus::prelude::*;

/// One placeholder row: checkbox square, status dot, then one bar per column.
#[component]
pub fn SkeletonRow() -> Element {
    rsx! {
        div { class: "skeleton-row", "aria-hidden": "true",
            span { class: "skeleton-box skeleton-checkbox" }
            span { class: "skeleton-box skeleton-dot" }
            span { class: "skeleton-box skeleton-bar skeleton-bar-name" }
            span { class: "skeleton-box skeleton-bar skeleton-bar-wide" }
            span { class: "skeleton-box skeleton-bar skeleton-bar-mid" }
            span { class: "skeleton-box skeleton-bar skeleton-bar-short" }
            span { class: "skeleton-box skeleton-bar skeleton-bar-short" }
            span { class: "skeleton-box skeleton-bar skeleton-bar-short" }
        }
    }
}

/// `count` skeleton rows under a busy region.
#[component]
pub fn SkeletonRows(#[props(default = 8)] count: usize) -> Element {
    rsx! {
        div { class: "skeleton-rows", "aria-busy": "true", role: "status",
            span { class: "skeleton-sr-only", "Loading…" }
            for i in 0..count {
                div { key: "{i}", SkeletonRow {} }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_skeleton_rows_ssr_default_eight_like_the_loading_frame() {
        let html = dioxus_ssr::render_element(rsx! { SkeletonRows {} });
        assert_eq!(html.matches("skeleton-row\"").count(), 8, "{html}");
        assert!(html.contains("Loading…"), "{html}");
        assert!(html.contains("aria-busy"), "{html}");
    }

    #[test]
    fn feedback_skeleton_rows_ssr_count_and_row_shapes() {
        let html = dioxus_ssr::render_element(rsx! { SkeletonRows { count: 3_usize } });
        assert_eq!(html.matches("skeleton-row\"").count(), 3, "{html}");
        assert!(html.contains("skeleton-checkbox"), "{html}");
        assert!(html.contains("skeleton-dot"), "{html}");
        assert!(html.contains("skeleton-bar-name"), "{html}");
    }
}
