//! P2-T8: the bar's per-action counts come from the same rules as the row
//! buttons, and the bulk frame's scene shows the bar in place of the header.

use std::collections::BTreeMap;

use super::*;
use crate::state::{PendingKey, PendingState};

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn ids(world: &berth_engine::MockWorld, names: &[&str]) -> Vec<ContainerId> {
    names
        .iter()
        .map(|n| {
            world
                .containers
                .iter()
                .find(|c| c.name == *n)
                .map(|c| c.id.clone())
                .unwrap_or_else(|| panic!("{n} in the mock world"))
        })
        .collect()
}

fn view_of<'a>(
    world: &'a berth_engine::MockWorld,
    pending: &'a BTreeMap<PendingKey, PendingState>,
) -> StateView<'a> {
    StateView {
        engines: &world.engines,
        containers: &world.containers,
        projects: &world.projects,
        images: &[],
        volumes: &[],
        networks: &[],
        pending,
    }
}

fn counts_of(
    world: &berth_engine::MockWorld,
    names: &[&str],
    pending: &BTreeMap<PendingKey, PendingState>,
) -> BulkCounts {
    bulk_counts(&ids(world, names), &view_of(world, pending))
}

const FRAME_SELECTION: [&str; 4] = [
    "shop-api-api-1",
    "shop-api-worker-1",
    "shop-api-redis-1",
    "shop-api-mailpit-1",
];

#[test]
fn bulk_counts_for_the_frames_selection() {
    // Three running and one restarting: nothing to start, the restarting
    // one can still be stopped, only running ones pause.
    let counts = counts_of(&world(), &FRAME_SELECTION, &BTreeMap::new());
    assert_eq!(
        counts.text(),
        "Start 0 \u{00b7} Stop 4 \u{00b7} Restart 4 \u{00b7} Pause 3 \u{00b7} Delete 4"
    );
}

#[test]
fn bulk_counts_text_with_one_stopped_container() {
    let counts = counts_of(
        &world(),
        &[
            "shop-api-api-1",
            "shop-api-redis-1",
            "shop-api-mailpit-1",
            "shop-api-db-1",
        ],
        &BTreeMap::new(),
    );
    assert_eq!(
        counts.text(),
        "Start 1 \u{00b7} Stop 3 \u{00b7} Restart 4 \u{00b7} Pause 3 \u{00b7} Delete 4"
    );
}

#[test]
fn bulk_counts_skip_containers_with_an_action_in_flight() {
    let world = world();
    let mut pending = BTreeMap::new();
    let busy = ids(&world, &["shop-api-api-1"]).remove(0);
    pending.insert(
        PendingKey {
            subject: Subject::Container(busy),
            action: "container.stop",
        },
        PendingState::Running {
            label: "Stopping\u{2026}".to_string(),
        },
    );
    let counts = counts_of(&world, &FRAME_SELECTION, &pending);
    assert_eq!(counts.stop, 3);
    assert_eq!(counts.delete, 3);
}

#[test]
fn bulk_applicable_lists_the_ids_each_action_will_run_on() {
    let world = world();
    let selected = ids(&world, &["shop-api-api-1", "shop-api-db-1"]);
    let pending = BTreeMap::new();
    let view = view_of(&world, &pending);
    assert_eq!(
        applicable(&selected, BulkAction::Start, &view),
        [selected[1].clone()]
    );
    assert_eq!(
        applicable(&selected, BulkAction::Pause, &view),
        [selected[0].clone()]
    );
    assert_eq!(applicable(&selected, BulkAction::Delete, &view), selected);
}

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

fn scene(frame: &str) -> String {
    strip_styles(&crate::scenes::render_scene_html(frame).expect("scene exists"))
}

#[test]
fn bulk_scene_shows_the_bar_instead_of_the_header() {
    let html = scene("Containers / bulk / 1440");
    assert!(html.contains("4 selected"), "{html}");
    assert!(
        html.contains(
            "Bulk actions: Start 0 \u{00b7} Stop 4 \u{00b7} Restart 4 \u{00b7} Pause 3 \u{00b7} Delete 4"
        ),
        "{html}"
    );
    for label in [
        "Stop 4",
        "Restart 4",
        "Pause 3",
        "Delete 4\u{2026}",
        "Clear",
    ] {
        assert!(html.contains(label), "{label} in {html}");
    }
    // The title, the filter and the inspector give way to the bar.
    assert!(!html.contains("Filter containers"), "{html}");
    assert!(!html.contains("ct-title"));
    assert!(!html.contains("class=\"inspector"), "{html}");
    // Four ticked rows.
    assert_eq!(html.matches("ct-row ct-selected").count(), 4);
}

#[test]
fn bulk_scene_disables_an_action_that_applies_to_none_with_its_reason() {
    let html = scene("Containers / bulk / 1440");
    let start = html
        .split("<button")
        .find(|b| b.contains("bulk-btn") && b.contains("Start 0"))
        .expect("Start button");
    assert!(start.contains("disabled"), "{start}");
    assert!(
        start.contains("data-tooltip=\"None of the selected containers can be started\""),
        "{start}"
    );
    let stop = html
        .split("<button")
        .find(|b| b.contains("bulk-btn") && b.contains("Stop 4"))
        .expect("Stop button");
    assert!(!stop.contains("disabled"), "{stop}");
}

#[test]
fn bulk_bar_is_absent_for_one_selected_row() {
    let html = scene("Containers / selected / 1440");
    assert!(!html.contains("bulk-bar"), "{html}");
    assert!(html.contains("Filter containers"));
}

#[test]
fn bulk_selection_of_two_or_more_gives_the_inspector_up() {
    use crate::state::{Section, Selection};
    let id = |n: &str| ContainerId(n.to_string());
    let one = Selection::Containers(vec![id("a")]);
    let two = Selection::Containers(vec![id("a"), id("b")]);
    assert!(crate::ui::inspector_visible(Section::Containers, &one));
    assert!(!crate::ui::inspector_visible(Section::Containers, &two));
}
