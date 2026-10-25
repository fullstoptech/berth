//! Scenes of the Clean up sheet and its result.

use super::*;

use crate::services::prune::{Entry, Group};
use crate::sheets::clean_up::SceneEntries;
use berth_core::{PruneItem, PruneKind};
use bytesize::ByteSize;

fn entry(
    group: Group,
    kind: PruneKind,
    id: &str,
    name: &str,
    size: u64,
    detail: Option<&str>,
) -> Entry {
    Entry {
        group,
        item: PruneItem {
            kind,
            id: id.to_string(),
            name: name.to_string(),
            size: Some(ByteSize(size)),
            selected: true,
        },
        detail: detail.map(str::to_string),
        failure: None,
    }
}

/// What the frame lists: a build cache entry, nine dangling images, three
/// stopped containers and three unused volumes (5.7 GB ticked, 1.6 GB of
/// volumes left unticked).
fn frame_entries() -> Vec<Entry> {
    let container = |name: &str, image: &str| {
        entry(
            Group::StoppedContainers,
            PruneKind::Container,
            name,
            name,
            70_000_000,
            Some(image),
        )
    };
    let mut entries = vec![
        container("eager_hopper", "alpine:3.20"),
        container("pg-old", "postgres:15"),
        container("test-runner", "node:22-alpine"),
    ];
    for n in 0..9_u64 {
        let size = if n == 8 { 320_000_000 } else { 260_000_000 };
        let id = format!("sha256:{n:012x}");
        let name = format!("{n:012x}");
        entries.push(entry(
            Group::DanglingImages,
            PruneKind::Image,
            &id,
            &name,
            size,
            None,
        ));
    }
    for (name, size) in [
        ("stale-cache", 900_000_000),
        ("pg_old_data", 500_000_000),
        ("tmp_uploads", 200_000_000),
    ] {
        entries.push(entry(
            Group::UnusedVolumes,
            PruneKind::Volume,
            name,
            name,
            size,
            None,
        ));
    }
    entries.push(entry(
        Group::BuildCache,
        PruneKind::BuildCache,
        "build-cache",
        "Build cache",
        3_100_000_000,
        None,
    ));
    entries
}

/// "Clean up / sheet / 1440": the sheet over the Engines dashboard.
fn clean_up_sheet(state: &mut AppState) {
    super::engines::load_engines_world(state);
    provide_context(SceneEntries(frame_entries()));
    state
        .overlays
        .write()
        .push(crate::state::Overlay::CleanUp(berth_core::EngineId(
            "colima:default".to_string(),
        )));
}

/// "Clean up / done / 1440": the dashboard after the run: the disk shows
/// what is left and the toast says what went.
fn clean_up_done(state: &mut AppState) {
    use crate::services::engine_dashboard::{DashboardStore, UsageEntry};
    super::engines::load_engines_world(state);
    let default = berth_core::EngineId("colima:default".to_string());
    if let Some(store) = try_consume_context::<DashboardStore>() {
        let current = store.usage.peek().get(&default).cloned();
        if let Some(UsageEntry::Ready(mut usage)) = current {
            let less = |size: ByteSize, by: u64| ByteSize(size.as_u64().saturating_sub(by));
            usage.images.size = less(usage.images.size, 2_400_000_000);
            usage.images.reclaimable = ByteSize(0);
            usage.build_cache = berth_core::UsageGroup::default();
            usage.containers.size = less(usage.containers.size, 210_000_000);
            usage.containers.reclaimable = ByteSize(0);
            usage.volumes.reclaimable = ByteSize(0);
            store.seed(&default, Some(UsageEntry::Ready(usage)), None);
        }
    }
    // Everything the run could remove is gone, so Clean up has nothing to
    // offer on the dashboard (the frame's greyed button).
    provide_context(SceneEntries(Vec::new()));
    let outcome = crate::services::prune::Outcome {
        engine: default,
        removed: frame_entries()
            .into_iter()
            .filter(|e| e.group != Group::UnusedVolumes)
            .map(|e| e.item)
            .collect(),
        failed: Vec::new(),
        freed: ByteSize(5_710_000_000),
        cancelled: false,
    };
    state.toasts.write().push(crate::state::Toast {
        id: uuid::Uuid::now_v7(),
        text: outcome.toast_text(),
        tone: crate::state::ToastTone::Success,
        action: None,
    });
}

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Clean up / sheet / 1440", clean_up_sheet),
    Scene::design("Clean up / done / 1440", clean_up_done),
];
