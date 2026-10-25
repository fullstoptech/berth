//! P4-T4 tests for the volume detail in the inspector: the scenes render
//! the header, tabs and states, the Overview rows read the volume, and the
//! helper image states say what the first-use dialog promises.

use super::*;

use crate::scenes;

fn scene(frame: &str) -> String {
    scenes::render_scene_html(frame).expect("the scene exists")
}

/// Drop every `<style>` block so class names in assertions mean elements.
fn strip_styles(html: &str) -> String {
    let mut out = html.to_string();
    while let (Some(a), Some(b)) = (out.find("<style>"), out.find("</style>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</style>".len(), "");
    }
    out
}

fn markup(frame: &str) -> String {
    strip_styles(&scene(frame))
}

#[test]
fn volumes_selected_scene_shows_the_header_buttons_and_both_tabs() {
    let html = markup("Volumes / selected / 1440");
    for needle in [
        "shop-api_pgdata",
        "In use by 1",
        ">Back up<",
        ">Restore\u{2026}<",
        ">Delete\u{2026}<",
        ">Overview<",
        ">Files<",
        "Used by",
        "shop-api-db-1",
        "/var/lib/docker/volumes/shop-api_pgdata/_data",
    ] {
        assert!(html.contains(needle), "missing {needle}: {html}");
    }
    // The Overview is showing, not the folder browser.
    assert!(!html.contains("fb-crumbs"), "{html}");
}

#[test]
fn volumes_delete_button_is_disabled_with_the_reason_while_a_container_uses_the_volume() {
    let html = markup("Volumes / selected / 1440");
    let delete = html
        .split(">Delete\u{2026}<")
        .next()
        .and_then(|before| before.rsplit("<button").next())
        .expect("the Delete button");
    assert!(delete.contains("disabled"), "{delete}");
    assert!(
        delete.contains("data-tooltip=\"Used by 1 container\""),
        "{delete}"
    );
}

#[test]
fn volumes_unused_volume_offers_delete_and_says_unused() {
    let html = markup("Volumes / confirm delete / 1440");
    // The selected volume's panel behind the dialog.
    assert!(html.contains("Unused"), "{html}");
    let delete = html
        .split(">Delete\u{2026}<")
        .next()
        .and_then(|before| before.rsplit("<button").next())
        .expect("the Delete button");
    assert!(!delete.contains("disabled"), "{delete}");
}

#[test]
fn volumes_browser_scene_opens_on_the_files_tab_and_mounts_the_file_browser() {
    let html = markup("Volumes / browser / 1440");
    assert!(html.contains("aria-label=\"Files\""), "{html}");
    assert!(html.contains("fb-crumbs"), "{html}");
    // The Overview's rows are not drawn on the Files tab.
    assert!(!html.contains("kv-list"), "{html}");
}

#[test]
fn volumes_helper_scene_asks_before_anything_is_downloaded() {
    let html = markup("Volumes / helper / 1440");
    assert!(
        html.contains("Berth uses a small helper image alpine:3 to read volumes"),
        "{html}"
    );
    assert!(html.contains("Download alpine:3"), "{html}");
    assert!(
        !html.contains("fb-crumbs"),
        "no browser before the helper is there"
    );
}

#[test]
fn volumes_helper_dialog_names_the_image_its_size_and_that_it_stays() {
    let effect = confirm::helper_effect();
    assert_eq!(effect.button_label(), "Download 1 image");
    let text: Vec<&str> = effect.lines.iter().map(|l| l.text.as_str()).collect();
    assert!(text[0].contains("alpine:3 (about 14 MB)"), "{text:?}");
    assert!(text[0].contains("stays on this engine"), "{text:?}");
    assert!(confirm::helper_title().contains("alpine:3"));
    assert_eq!(effect.typed_name, None, "a download needs no typed name");
}

#[test]
fn volumes_backup_file_name_is_the_volume_and_the_day() {
    let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 25).expect("date");
    assert_eq!(
        backup_file_name("berthfx_berthfx_pgdata", day),
        "berthfx_berthfx_pgdata-20261025.tar.gz"
    );
}

#[test]
fn volumes_overview_rows_read_the_volume() {
    let world = berth_engine::MockWorld::bundled();
    let volume = world
        .volumes_by_engine
        .values()
        .flatten()
        .find(|v| v.name == "shop-api_pgdata")
        .cloned()
        .expect("pgdata");
    let now = volume.created.expect("created") + chrono::Duration::days(6);
    let rows = overview_rows(
        &volume,
        Some("default"),
        Some("shop-api"),
        Some(bytesize::ByteSize::mb(1200)),
        now,
    );
    let value = |label: &str| {
        rows.iter()
            .find(|r| r.label == label)
            .map(|r| r.value.clone())
            .expect(label)
    };
    assert_eq!(value("Name"), "shop-api_pgdata");
    assert_eq!(value("Driver"), "local");
    assert_eq!(value("Engine"), "default");
    assert_eq!(value("Project"), "shop-api");
    assert_eq!(value("Size"), "1.2 GB");
    assert!(
        value("Created").starts_with("6 d ago"),
        "{}",
        value("Created")
    );
    assert_eq!(
        value("Mountpoint"),
        "/var/lib/docker/volumes/shop-api_pgdata/_data"
    );
    // No size yet reads "Calculating…", an unknown engine is empty.
    let waiting = overview_rows(&volume, None, None, None, now);
    let size = waiting.iter().find(|r| r.label == "Size").expect("size");
    assert_eq!(size.value, "Calculating\u{2026}");
    let engine = waiting
        .iter()
        .find(|r| r.label == "Engine")
        .expect("engine");
    assert!(engine.value.is_empty());
}

#[test]
fn volumes_restore_dialog_asks_for_the_volume_name_and_lists_who_uses_it() {
    let effect = confirm::restore_effect(
        "shop-api_pgdata",
        "pgdata-20261025.tar.gz",
        2_411,
        bytesize::ByteSize::mb(48),
        &["shop-api-db-1".to_string()],
    );
    assert_eq!(effect.typed_name.as_deref(), Some("shop-api_pgdata"));
    assert_eq!(confirm::button_label(&effect), "Restore backup");
    assert_eq!(confirm::result_label(&effect, 1), "Restored 1 backup");
    let text: String = effect
        .lines
        .iter()
        .map(|l| l.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("2,411 files (48 MB) from pgdata-20261025.tar.gz into shop-api_pgdata"),
        "{text}"
    );
    assert!(text.contains("shop-api-db-1"), "{text}");
    assert!(text.contains("overwritten"), "{text}");
    // Without a user of the volume there is no such line.
    let alone = confirm::restore_effect("v1", "b.tar.gz", 1, bytesize::ByteSize::kb(1), &[]);
    assert!(alone.lines.iter().all(|l| !l.text.contains("stop them")));
}
