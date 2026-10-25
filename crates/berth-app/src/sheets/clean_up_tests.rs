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

// ---------------------------------------------------------------------
// After a run: the dashboard reads the disk again
// ---------------------------------------------------------------------

use std::sync::Arc;

use berth_core::EngineId;
use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use super::after_run;
use crate::screens::engine_dashboard::disk_summary;
use crate::services::engine_dashboard::{DashboardStore, UsageEntry};
use crate::services::Services;

fn default_engine() -> EngineId {
    EngineId("colima:default".to_string())
}

thread_local! {
    static RIG: std::cell::RefCell<Option<(DashboardStore, Signal<bool>)>> =
        const { std::cell::RefCell::new(None) };
    static SERVICES: std::cell::RefCell<Option<Services>> = const { std::cell::RefCell::new(None) };
}

/// Stands in for the sheet: it ends a run in its own scope, which goes away
/// at once, as the sheet does when it closes.
#[component]
fn FinishingSheet() -> Element {
    use_hook(|| {
        let store = try_consume_context::<DashboardStore>().expect("store");
        let services = SERVICES
            .with(|s| s.borrow().clone())
            .expect("services were set");
        let outcome = crate::services::prune::Outcome {
            engine: default_engine(),
            removed: Vec::new(),
            failed: Vec::new(),
            freed: bytesize::ByteSize(0),
            cancelled: false,
        };
        after_run(&services, Some(store), &default_engine(), &outcome);
    });
    rsx! {}
}

#[component]
fn Dashboard() -> Element {
    let store = use_hook(|| provide_context(DashboardStore::new()));
    let open = use_signal(|| true);
    use_hook(move || RIG.with(|rig| *rig.borrow_mut() = Some((store, open))));
    rsx! {
        if open() {
            FinishingSheet {}
        }
    }
}

fn nothing_left(usage: &berth_core::DiskUsage) -> berth_core::DiskUsage {
    let mut usage = usage.clone();
    for group in [
        &mut usage.images,
        &mut usage.containers,
        &mut usage.volumes,
        &mut usage.build_cache,
    ] {
        group.reclaimable = bytesize::ByteSize(0);
    }
    usage
}

#[tokio::test]
async fn clean_up_after_a_run_the_dashboard_reads_the_disk_again_even_though_the_sheet_closed() {
    let mut world = berth_engine::MockWorld::bundled();
    let before = world.disk_usage_by_engine[&default_engine().0].clone();
    let (_, reclaimable_before) = disk_summary(&before, None);
    assert_ne!(
        reclaimable_before.as_deref(),
        Some("0 B reclaimable"),
        "the stale reading has something to reclaim"
    );
    // What the engine answers once the run is over.
    world
        .disk_usage_by_engine
        .insert(default_engine().0, nothing_left(&before));
    let mock =
        Arc::new(berth_engine::MockEngine::from_world(world, &default_engine()).expect("engine"));
    let clients = crate::services::engines::client_map();
    clients.lock().expect("clients").insert(
        default_engine(),
        mock as Arc<dyn berth_engine::EngineClient>,
    );
    SERVICES.with(|s| {
        *s.borrow_mut() = Some(Services {
            store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
            clients,
            cancel: CancellationToken::new(),
        });
    });

    let mut dom = VirtualDom::new(Dashboard);
    // The sheet runs its end-of-run code in its first render, then goes.
    // The stale reading is put in before it can answer.
    let _ = dom.rebuild_to_vec();
    let (store, mut open) = RIG
        .with(|rig| rig.borrow_mut().take())
        .expect("rig captured");
    dom.in_runtime(|| {
        store.seed(
            &default_engine(),
            Some(UsageEntry::Ready(before.clone())),
            None,
        );
        open.set(false);
    });
    dom.render_immediate(&mut dioxus::core::NoOpMutations);

    let waited = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let now = dom.in_runtime(|| store.usage.peek().get(&default_engine()).cloned());
            if let Some(UsageEntry::Ready(usage)) = now {
                if usage != before {
                    return usage;
                }
            }
            let _ = tokio::time::timeout(std::time::Duration::from_millis(20), dom.wait_for_work())
                .await;
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
        }
    })
    .await
    .expect("the dashboard's disk usage was read again after the run");
    let (_, reclaimable) = disk_summary(&waited, None);
    assert_eq!(reclaimable.as_deref(), Some("0 B reclaimable"));
}
