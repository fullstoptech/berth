//! P4-T7 tests for the Clean up sheet: the two frames' scenes draw what the
//! frames show, volumes start unticked, and the button and the toast speak
//! of the same count and size.

use crate::scenes;

const SHEET: &str = "Clean up / sheet / 1440";
const DONE: &str = "Clean up / done / 1440";

/// The checkbox of a group, as `aria-checked` says it.
fn checked(html: &str, label: &str) -> String {
    let at = html
        .find(&format!("aria-label=\"{label}\""))
        .unwrap_or_else(|| panic!("no checkbox for {label}: {html}"));
    let tag_start = html[..at].rfind("<button").expect("a button tag");
    let tag_end = at + html[at..].find('>').expect("tag end");
    let tag = &html[tag_start..tag_end];
    let key = "aria-checked=\"";
    let from = tag.find(key).expect("aria-checked") + key.len();
    tag[from..from + tag[from..].find('"').expect("quote")].to_string()
}

#[test]
fn clean_up_sheet_scene_volumes_start_unticked_and_the_rest_ticked() {
    let html = scenes::render_scene_html(SHEET).expect("scene");
    assert!(html.contains("role=\"dialog\""));
    assert!(html.contains("aria-label=\"Clean up default\""), "{html}");
    assert_eq!(checked(&html, "Unused volumes"), "false");
    for label in [
        "Build cache",
        "Dangling images",
        "Stopped containers not in a project",
    ] {
        assert_eq!(checked(&html, label), "true", "{label}");
    }
}

#[test]
fn clean_up_sheet_scene_button_summary_and_rows_match_the_frame() {
    let html = scenes::render_scene_html(SHEET).expect("scene");
    assert!(html.contains(">Remove 13 items (5.7 GB)<"), "{html}");
    assert!(
        html.contains(
            "13 items selected: 1 build cache entry, 9 images, 3 containers \u{b7} 5.7 GB. \
             Volumes hold data and stay unless you tick them."
        ),
        "{html}"
    );
    for text in [
        "Build cache",
        "1 entry",
        "3.1 GB",
        "Dangling images",
        "9 images",
        "2.4 GB",
        "3 containers",
        "210 MB",
        "eager_hopper",
        "alpine:3.20",
        "pg-old",
        "postgres:15",
        "test-runner",
        "node:22-alpine",
        "3 volumes \u{b7} hold data",
        "1.6 GB",
        "Containers that belong to a project are never cleaned up here.",
        "Select what to remove from engine default. Nothing is removed until you press the button.",
    ] {
        assert!(html.contains(text), "{text}: {html}");
    }
    // Unticked volumes are not listed by name.
    assert!(!html.contains("stale-cache"), "{html}");
    assert!(html.contains(">Cancel<"));
}

#[test]
fn clean_up_done_scene_toast_says_what_the_button_promised() {
    let sheet = scenes::render_scene_html(SHEET).expect("scene");
    let done = scenes::render_scene_html(DONE).expect("scene");
    assert!(sheet.contains(">Remove 13 items (5.7 GB)<"));
    assert!(
        done.contains("Removed 13 items \u{b7} 5.7 GB freed"),
        "{done}"
    );
    // The disk the dashboard shows afterwards is what is left.
    assert!(done.contains("16.8 of 60 GB used"), "{done}");
}
