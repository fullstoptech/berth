//! P4-T7 tests for Clean up: the groups and what is ticked, the labels the
//! button and the result share, the item-by-item run and what stays after a
//! partial failure. Every run is against the mock engine.

use super::*;

use berth_engine::{MockEngine, MockWorld};

fn default_engine() -> EngineId {
    EngineId("colima:default".to_string())
}

struct World {
    world: MockWorld,
}

impl World {
    fn new() -> Self {
        Self {
            world: MockWorld::bundled(),
        }
    }

    fn entries(&self) -> Vec<Entry> {
        let engine = default_engine();
        let images: Vec<Image> = self.world.images_by_engine[&engine.0].clone();
        let volumes: Vec<Volume> = self.world.volumes_by_engine[&engine.0].clone();
        let networks: Vec<Network> = self.world.networks_by_engine[&engine.0].clone();
        candidates(
            &engine,
            &Lists {
                containers: &self.world.containers,
                images: &images,
                volumes: &volumes,
                networks: &networks,
                usage: self.world.disk_usage_by_engine.get(&engine.0),
            },
        )
    }

    fn container_id(&self, name: &str) -> String {
        self.world
            .containers
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.id.0.clone())
            .expect("mock container")
    }
}

fn names(entries: &[Entry], group: Group) -> Vec<String> {
    in_group(entries, group)
        .iter()
        .map(|e| e.item.name.clone())
        .collect()
}

fn item(kind: PruneKind, id: &str, name: &str, size: u64) -> PruneItem {
    PruneItem {
        kind,
        id: id.to_string(),
        name: name.to_string(),
        size: Some(ByteSize(size)),
        selected: true,
    }
}

fn entry_of(group: Group, item: PruneItem) -> Entry {
    Entry {
        group,
        item,
        detail: None,
        failure: None,
    }
}

#[test]
fn clean_up_volumes_are_unchecked_by_default_and_everything_else_is_ticked() {
    let checked = default_checked();
    assert!(!checked.contains(&Group::UnusedVolumes));
    for group in Group::ALL
        .into_iter()
        .filter(|g| *g != Group::UnusedVolumes)
    {
        assert!(checked.contains(&group), "{group:?}");
    }
    // The plan the button is built from leaves every volume out.
    let world = World::new();
    let entries = world.entries();
    assert!(!in_group(&entries, Group::UnusedVolumes).is_empty());
    let plan = plan(&default_engine(), &entries, &checked);
    for item in plan.items.iter().filter(|i| i.kind == PruneKind::Volume) {
        assert!(!item.selected, "{item:?}");
    }
    assert!(plan
        .items
        .iter()
        .any(|i| i.kind == PruneKind::Container && i.selected));
}

#[test]
fn clean_up_ticking_volumes_adds_them_to_the_button() {
    let world = World::new();
    let entries = world.entries();
    let before = plan(&default_engine(), &entries, &default_checked()).effect();
    let mut with_volumes = default_checked();
    with_volumes.insert(Group::UnusedVolumes);
    let after = plan(&default_engine(), &entries, &with_volumes).effect();
    assert_eq!(after.count, before.count + 3);
    assert_eq!(
        after.bytes.map(|b| b.as_u64()),
        before.bytes.map(|b| b.as_u64() + 1_600_000_000)
    );
}

#[test]
fn clean_up_groups_follow_the_lists() {
    let world = World::new();
    let entries = world.entries();
    // Only the stopped container that no project owns: the compose ones and
    // the paused one stay out.
    assert_eq!(
        names(&entries, Group::StoppedContainers),
        ["eager_hopper".to_string()]
    );
    let eager = &in_group(&entries, Group::StoppedContainers)[0];
    assert_eq!(eager.detail.as_deref(), Some("alpine:3.20"));
    // Its size comes from `DiskUsage.container_sizes`.
    assert_eq!(eager.item.size, Some(ByteSize(12_000_000)));
    assert_eq!(in_group(&entries, Group::DanglingImages).len(), 3);
    assert_eq!(in_group(&entries, Group::UnusedImages).len(), 6);
    assert!(in_group(&entries, Group::DanglingImages)
        .iter()
        .all(|e| e.item.name.len() == 12));
    assert_eq!(
        names(&entries, Group::UnusedVolumes),
        ["stale-cache", "old-uploads", "tmp-export"]
    );
    // Built-in networks and networks with containers are never offered.
    assert!(in_group(&entries, Group::UnusedNetworks).is_empty());
    // One build cache entry, the size that is reclaimable.
    let cache = in_group(&entries, Group::BuildCache);
    assert_eq!(cache.len(), 1);
    assert_eq!(cache[0].item.size, Some(ByteSize(3_100_000_000)));
}

#[test]
fn clean_up_runs_containers_first_and_the_build_cache_once_last() {
    let world = World::new();
    let entries = world.entries();
    let order: Vec<PruneKind> = plan(&default_engine(), &entries, &default_checked())
        .items
        .iter()
        .map(|i| i.kind)
        .collect();
    assert_eq!(order.first(), Some(&PruneKind::Container));
    assert_eq!(order.last(), Some(&PruneKind::BuildCache));
    assert_eq!(
        order
            .iter()
            .filter(|k| **k == PruneKind::BuildCache)
            .count(),
        1
    );
}

#[test]
fn clean_up_without_disk_usage_offers_no_build_cache() {
    let world = World::new();
    let entries = candidates(
        &default_engine(),
        &Lists {
            containers: &world.world.containers,
            images: &[],
            volumes: &[],
            networks: &[],
            usage: None,
        },
    );
    assert!(in_group(&entries, Group::BuildCache).is_empty());
    assert_eq!(
        in_group(&entries, Group::StoppedContainers)[0].item.size,
        None
    );
}

#[test]
fn clean_up_summary_counts_what_is_ticked_and_warns_about_volumes() {
    let world = World::new();
    let entries = world.entries();
    let checked = default_checked();
    let plan = plan(&default_engine(), &entries, &checked);
    let line = summary_line(&plan, &entries, &checked);
    assert_eq!(
        line,
        "11 items selected: 1 build cache entry, 9 images, 1 container \u{b7} 5.9 GB. \
         Volumes hold data and stay unless you tick them."
    );
    let mut with_volumes = checked.clone();
    with_volumes.insert(Group::UnusedVolumes);
    let plan = super::plan(&default_engine(), &entries, &with_volumes);
    let line = summary_line(&plan, &entries, &with_volumes);
    assert!(line.contains("3 volumes"), "{line}");
    assert!(!line.contains("stay unless"), "{line}");
}

#[tokio::test]
async fn clean_up_button_and_result_labels_are_the_same_sentence_for_the_same_plan() {
    let world = World::new();
    let entries = world.entries();
    let plan = plan(&default_engine(), &entries, &default_checked());
    let button = plan.effect().button_label();
    let engine = MockEngine::bundled(&default_engine());
    let outcome = run(&engine, &plan, CancellationToken::new(), |_| {}).await;
    assert!(outcome.clean(), "{outcome:?}");
    assert_eq!(button, "Remove 11 items (5.9 GB)");
    assert_eq!(outcome.result_label(), "Removed 11 items (5.9 GB)");
    // Same count and size, only the tense differs.
    assert_eq!(
        button.replacen("Remove", "Removed", 1),
        outcome.result_label()
    );
    assert_eq!(outcome.toast_text(), "Removed 11 items \u{b7} 5.9 GB freed");
}

#[tokio::test]
async fn clean_up_removes_what_was_ticked_and_leaves_the_rest() {
    let world = World::new();
    let entries = world.entries();
    let plan = plan(&default_engine(), &entries, &default_checked());
    let engine = MockEngine::bundled(&default_engine());
    let outcome = run(&engine, &plan, CancellationToken::new(), |_| {}).await;
    assert_eq!(outcome.removed.len(), 11);
    let left: Vec<String> = berth_engine::EngineClient::containers(&engine)
        .await
        .expect("containers")
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert!(!left.contains(&"eager_hopper".to_string()));
    assert!(left.contains(&"shop-api-api-1".to_string()));
    // Volumes were not ticked, so they are all still there.
    let volumes: Vec<String> = berth_engine::EngineClient::volumes(&engine)
        .await
        .expect("volumes")
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert!(volumes.contains(&"stale-cache".to_string()));
}

#[tokio::test]
async fn clean_up_reports_progress_item_by_item_with_the_build_cache_last() {
    let world = World::new();
    let mut entries = world.entries();
    // The cache listed first still runs last.
    entries.sort_by_key(|e| e.group != Group::BuildCache);
    let mut plan = plan(&default_engine(), &entries, &default_checked());
    plan.items.sort_by_key(|i| i.kind != PruneKind::BuildCache);
    let engine = MockEngine::bundled(&default_engine());
    let mut seen: Vec<Progress> = Vec::new();
    let outcome = run(&engine, &plan, CancellationToken::new(), |p| seen.push(p)).await;
    assert!(outcome.clean());
    // One progress before each of the 11 items, one after the last.
    assert_eq!(seen.len(), 12);
    assert_eq!(seen[0].done, 0);
    assert_eq!(seen[0].total, 11);
    assert_eq!(seen[0].current, "eager_hopper");
    assert_eq!(seen[10].current, "Build cache");
    assert_eq!(seen[11].done, 11);
    let dones: Vec<usize> = seen.iter().map(|p| p.done).collect();
    assert_eq!(dones, (0..=11).collect::<Vec<_>>());
    assert!((seen[11].fraction() - 1.0).abs() < f32::EPSILON);
}

#[tokio::test]
async fn clean_up_partial_failure_keeps_the_failed_items_listed_with_the_reason() {
    let world = World::new();
    let mut entries = world.entries();
    // An item the engine does not have fails; the rest go.
    let ghost = item(
        PruneKind::Volume,
        "ghost-volume",
        "ghost-volume",
        40_000_000,
    );
    entries.push(entry_of(Group::UnusedVolumes, ghost));
    let mut checked = default_checked();
    checked.insert(Group::UnusedVolumes);
    let plan = plan(&default_engine(), &entries, &checked);
    let engine = MockEngine::bundled(&default_engine());
    let outcome = run(&engine, &plan, CancellationToken::new(), |_| {}).await;
    assert!(!outcome.clean());
    assert_eq!(outcome.failed.len(), 1);
    assert_eq!(outcome.failed[0].0.id, "ghost-volume");
    assert!(
        outcome.failed[0].1.contains("ghost-volume"),
        "{:?}",
        outcome.failed
    );
    // 11 + 3 real volumes removed; the label counts only those.
    assert_eq!(outcome.removed.len(), 14);
    assert_eq!(outcome.result_label(), "Removed 14 items (7.5 GB)");
    assert_eq!(
        outcome.failure_line().as_deref(),
        Some("1 item could not be removed.")
    );

    let rest = remaining(entries, &outcome);
    assert_eq!(rest.len(), 1, "{rest:?}");
    assert_eq!(rest[0].item.id, "ghost-volume");
    assert!(rest[0]
        .failure
        .as_deref()
        .is_some_and(|r| r.contains("ghost-volume")));
    assert_eq!(
        partial_notice(&outcome),
        "Removed 14 items (7.5 GB). 1 item could not be removed."
    );
    // The retry button names only what is left.
    let retry = super::plan(&default_engine(), &rest, &checked).effect();
    assert_eq!(retry.button_label(), "Remove 1 item (40 MB)");
}

#[tokio::test]
async fn clean_up_never_removes_a_container_that_is_running_again() {
    let world = World::new();
    let running = world.container_id("shop-api-api-1");
    let plan = PrunePlan {
        engine: default_engine(),
        items: vec![item(PruneKind::Container, &running, "shop-api-api-1", 1)],
    };
    let engine = MockEngine::bundled(&default_engine());
    let outcome = run(&engine, &plan, CancellationToken::new(), |_| {}).await;
    assert!(outcome.removed.is_empty());
    assert_eq!(outcome.failed.len(), 1);
    assert!(
        outcome.failed[0].1.contains("running"),
        "{:?}",
        outcome.failed
    );
    let names: Vec<String> = berth_engine::EngineClient::containers(&engine)
        .await
        .expect("containers")
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert!(names.contains(&"shop-api-api-1".to_string()));
}

#[tokio::test]
async fn clean_up_a_cancelled_run_leaves_untried_items_on_the_sheet() {
    let world = World::new();
    let entries = world.entries();
    let plan = plan(&default_engine(), &entries, &default_checked());
    let engine = MockEngine::bundled(&default_engine());
    let cancel = CancellationToken::new();
    cancel.cancel();
    let outcome = run(&engine, &plan, cancel, |_| {}).await;
    assert!(outcome.cancelled);
    assert!(!outcome.clean());
    assert!(outcome.removed.is_empty() && outcome.failed.is_empty());
    let rest = remaining(entries.clone(), &outcome);
    assert_eq!(rest.len(), entries.len());
    assert_eq!(partial_notice(&outcome), "Stopped before the end.");
}

#[tokio::test]
async fn clean_up_records_the_run_with_its_sentence_in_the_activity_feed() {
    let world = World::new();
    let entries = world.entries();
    let plan = plan(&default_engine(), &entries, &default_checked());
    let engine = MockEngine::bundled(&default_engine());
    let outcome = run(&engine, &plan, CancellationToken::new(), |_| {}).await;
    let entry = outcome.activity();
    assert_eq!(entry.action, CLEAN_UP);
    assert_eq!(entry.subject_name, "11 items (5.9 GB)");
    assert_eq!(entry.outcome, berth_core::Outcome::Ok);
}
