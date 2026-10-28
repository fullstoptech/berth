//! P8-T4 tests for the Save as image bar, read from its scenes: the suggested name, the words,
//! and what each state offers.

use crate::scenes;

fn scene(frame: &str) -> String {
    scenes::render_scene_html(frame).unwrap_or_else(|| panic!("{frame} renders"))
}

/// The bar's markup: from its group to the tabs under it.
fn bar(html: &str) -> &str {
    let start = html
        .find("aria-label=\"Save as image\"")
        .unwrap_or_else(|| panic!("no bar: {html}"));
    let rest = &html[start..];
    let end = rest.find("cd-tabs").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn save_as_image_scene_asks_for_a_name_the_engine_will_take() {
    let html = scene("Containers / save as image / 1440");
    let bar = bar(&html);
    assert!(bar.contains("initial_value=\"pg-scratch-snap:1\""), "{bar}");
    assert!(bar.contains("Image name"), "{bar}");
    assert!(
        bar.contains("Its environment variables are saved with it."),
        "the environment travels with the image and the bar says so: {bar}"
    );
    assert!(bar.contains(">Save as image<"), "{bar}");
    assert!(bar.contains(">Cancel<"), "{bar}");
    assert!(
        !bar.contains("disabled=true"),
        "a valid suggestion leaves the button on: {bar}"
    );
}

#[test]
fn save_as_image_scene_with_a_bad_name_says_why_and_waits() {
    let html = scene("Containers / save as image / invalid / 1440");
    let bar = bar(&html);
    assert!(bar.contains("aria-invalid=\"true\""), "{bar}");
    assert!(bar.contains("An image name has no spaces"), "{bar}");
    assert!(bar.contains("disabled=true"), "{bar}");
}

#[test]
fn save_as_image_scene_while_running_shows_saving_and_a_way_to_stop_waiting() {
    let html = scene("Containers / save as image / running / 1440");
    let bar = bar(&html);
    assert!(bar.contains(">Saving\u{2026}<"), "{bar}");
    assert!(bar.contains(">Stop waiting<"), "{bar}");
    assert!(!bar.contains(">Cancel<"), "{bar}");
    assert!(bar.contains("aria-busy=\"true\""), "{bar}");
}

#[test]
fn save_as_image_scene_when_saved_names_the_image_and_offers_images() {
    let html = scene("Containers / save as image / saved / 1440");
    let bar = bar(&html);
    assert!(bar.contains("Saved as berthfx/pg-snap:1."), "{bar}");
    assert!(bar.contains(">Show in Images<"), "{bar}");
    assert!(bar.contains(">Close<"), "{bar}");
    assert!(!bar.contains("field-input"), "the field is done: {bar}");
}

#[test]
fn save_as_image_scene_after_a_refusal_shows_the_engines_reason() {
    let html = scene("Containers / save as image / failed / 1440");
    let bar = bar(&html);
    assert!(bar.contains("No space left on device"), "{bar}");
    assert!(bar.contains(">Save as image<"), "{bar}");
}

#[test]
fn save_as_image_bar_is_closed_until_the_menu_item_opens_it() {
    // The ordinary containers scene has no bar.
    let html = scene("Containers / selected / 1440");
    assert!(!html.contains("aria-label=\"Save as image\""));
}
