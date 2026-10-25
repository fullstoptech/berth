//! Clean up (P4-T7): what an engine could shed, grouped as the sheet shows
//! it, and the removal that runs item by item.
//!
//! The plan is built from the lists Berth already holds plus `disk_usage`
//! (container sizes, build cache). The engine call stays
//! `EngineClient::prune(plan)`: [`run`] sends it one-item plans in order and
//! counts the reports it gets back, so the sheet can show progress and keep
//! the items that failed. Build cache is one item and goes last.
//!
//! Everything here is pure or takes a client; the sheet owns the signals.

use std::collections::BTreeSet;

use bytesize::ByteSize;
use tokio_util::sync::CancellationToken;

use berth_core::{
    Container, ContainerId, ContainerState, DiskUsage, EffectSummary, EngineId, Image, Network,
    PruneItem, PruneKind, PrunePlan, Volume,
};
use berth_engine::EngineClient;

/// The activity action of a finished clean up.
pub const CLEAN_UP: &str = "engine.clean_up";

/// The id of the one build-cache item.
pub const BUILD_CACHE_ID: &str = "build-cache";

// ---------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------

/// One checkbox of the sheet. Declaration order is the order the sheet
/// shows them in (the frame's); [`Group::EXECUTION`] is the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    BuildCache,
    DanglingImages,
    StoppedContainers,
    UnusedImages,
    UnusedVolumes,
    UnusedNetworks,
}

impl Group {
    /// Display order.
    pub const ALL: [Self; 6] = [
        Self::BuildCache,
        Self::DanglingImages,
        Self::StoppedContainers,
        Self::UnusedImages,
        Self::UnusedVolumes,
        Self::UnusedNetworks,
    ];

    /// Run order: containers first (an image they use is not unused until
    /// they are gone), build cache once, last.
    pub const EXECUTION: [Self; 6] = [
        Self::StoppedContainers,
        Self::DanglingImages,
        Self::UnusedImages,
        Self::UnusedVolumes,
        Self::UnusedNetworks,
        Self::BuildCache,
    ];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::BuildCache => "build-cache",
            Self::DanglingImages => "dangling-images",
            Self::StoppedContainers => "stopped-containers",
            Self::UnusedImages => "unused-images",
            Self::UnusedVolumes => "unused-volumes",
            Self::UnusedNetworks => "unused-networks",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::BuildCache => "Build cache",
            Self::DanglingImages => "Dangling images",
            Self::StoppedContainers => "Stopped containers not in a project",
            Self::UnusedImages => "Unused images",
            Self::UnusedVolumes => "Unused volumes",
            Self::UnusedNetworks => "Unused networks",
        }
    }

    #[must_use]
    pub fn kind(self) -> PruneKind {
        match self {
            Self::BuildCache => PruneKind::BuildCache,
            Self::DanglingImages | Self::UnusedImages => PruneKind::Image,
            Self::StoppedContainers => PruneKind::Container,
            Self::UnusedVolumes => PruneKind::Volume,
            Self::UnusedNetworks => PruneKind::Network,
        }
    }

    /// "1 entry", "9 images", "3 volumes" for the count beside the label.
    #[must_use]
    pub fn count_text(self, count: usize) -> String {
        let (one, many) = match self {
            Self::BuildCache => ("entry", "entries"),
            Self::DanglingImages | Self::UnusedImages => ("image", "images"),
            Self::StoppedContainers => ("container", "containers"),
            Self::UnusedVolumes => ("volume", "volumes"),
            Self::UnusedNetworks => ("network", "networks"),
        };
        format!("{count} {}", if count == 1 { one } else { many })
    }

    /// The warning that rides beside a group's count.
    #[must_use]
    pub fn hint(self) -> Option<&'static str> {
        (self == Self::UnusedVolumes).then_some("hold data")
    }

    /// Does the sheet list this group's items under its row?
    /// Containers always; volumes once they are ticked, so the names of
    /// what holds data are in front of the person who ticked them.
    #[must_use]
    pub fn lists_items(self, checked: bool) -> bool {
        match self {
            Self::StoppedContainers => true,
            Self::UnusedVolumes => checked,
            _ => false,
        }
    }
}

/// The groups ticked when the sheet opens: everything except volumes,
/// which hold data and stay until the person ticks them.
#[must_use]
pub fn default_checked() -> BTreeSet<Group> {
    Group::ALL
        .into_iter()
        .filter(|g| *g != Group::UnusedVolumes)
        .collect()
}

// ---------------------------------------------------------------------
// Candidates
// ---------------------------------------------------------------------

/// One thing the engine could remove, with the group it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub group: Group,
    pub item: PruneItem,
    /// The image of a container, shown beside its name.
    pub detail: Option<String>,
    /// Why the last attempt to remove it failed.
    pub failure: Option<String>,
}

/// The lists a plan is built from.
pub struct Lists<'a> {
    pub containers: &'a [Container],
    pub images: &'a [Image],
    pub volumes: &'a [Volume],
    pub networks: &'a [Network],
    /// `None` until the engine has answered `disk_usage`: no build cache
    /// is offered and container sizes are unknown.
    pub usage: Option<&'a DiskUsage>,
}

fn short_image_id(id: &str) -> String {
    let bare = id.strip_prefix("sha256:").unwrap_or(id);
    bare.get(..12).unwrap_or(bare).to_string()
}

fn entry(group: Group, item: PruneItem, detail: Option<String>) -> Entry {
    Entry {
        group,
        item,
        detail,
        failure: None,
    }
}

/// Everything on `engine` Clean up could remove, in run order. Containers
/// that belong to a project are never offered; volumes in use, networks
/// with containers and built-in networks never are either.
#[must_use]
pub fn candidates(engine: &EngineId, lists: &Lists<'_>) -> Vec<Entry> {
    let sizes = lists.usage.map(|u| &u.container_sizes);
    let mut out = Vec::new();
    for c in lists.containers.iter().filter(|c| c.engine == *engine) {
        let stopped = matches!(
            c.state,
            ContainerState::Exited | ContainerState::Dead | ContainerState::Created
        );
        if !stopped || c.project.is_some() || c.compose_project.is_some() {
            continue;
        }
        let size = sizes.and_then(|all| {
            all.iter()
                .find(|(id, _): &&(ContainerId, ByteSize)| *id == c.id)
                .map(|(_, size)| *size)
        });
        out.push(entry(
            Group::StoppedContainers,
            PruneItem {
                kind: PruneKind::Container,
                id: c.id.0.clone(),
                name: c.name.clone(),
                size,
                selected: true,
            },
            Some(c.image.reference.clone()),
        ));
    }
    for (dangling, group) in [(true, Group::DanglingImages), (false, Group::UnusedImages)] {
        for image in lists
            .images
            .iter()
            .filter(|i| i.engine == *engine && i.dangling == dangling && i.used_by.is_empty())
        {
            let name = image
                .repo_tags
                .first()
                .cloned()
                .unwrap_or_else(|| short_image_id(&image.id));
            out.push(entry(
                group,
                PruneItem {
                    kind: PruneKind::Image,
                    id: image.id.clone(),
                    name,
                    size: Some(image.size),
                    selected: true,
                },
                None,
            ));
        }
    }
    for v in lists
        .volumes
        .iter()
        .filter(|v| v.engine == *engine && v.used_by.is_empty())
    {
        out.push(entry(
            Group::UnusedVolumes,
            PruneItem {
                kind: PruneKind::Volume,
                id: v.name.clone(),
                name: v.name.clone(),
                size: v.size,
                selected: true,
            },
            None,
        ));
    }
    for n in lists
        .networks
        .iter()
        .filter(|n| n.engine == *engine && !n.built_in && n.containers.is_empty())
    {
        out.push(entry(
            Group::UnusedNetworks,
            PruneItem {
                kind: PruneKind::Network,
                id: n.id.clone(),
                name: n.name.clone(),
                size: None,
                selected: true,
            },
            None,
        ));
    }
    if let Some(cache) = lists.usage.map(|u| &u.build_cache) {
        if cache.count > 0 {
            out.push(entry(
                Group::BuildCache,
                PruneItem {
                    kind: PruneKind::BuildCache,
                    id: BUILD_CACHE_ID.to_string(),
                    name: Group::BuildCache.label().to_string(),
                    size: Some(cache.reclaimable),
                    selected: true,
                },
                None,
            ));
        }
    }
    out
}

/// The plan for the groups that are ticked: every candidate stays in it,
/// the unticked ones with `selected: false`, in run order.
#[must_use]
pub fn plan(engine: &EngineId, entries: &[Entry], checked: &BTreeSet<Group>) -> PrunePlan {
    let mut items = Vec::new();
    for group in Group::EXECUTION {
        for e in entries.iter().filter(|e| e.group == group) {
            let mut item = e.item.clone();
            item.selected = checked.contains(&group);
            items.push(item);
        }
    }
    PrunePlan {
        engine: engine.clone(),
        items,
    }
}

/// The entries of one group, in the order they were found.
#[must_use]
pub fn in_group(entries: &[Entry], group: Group) -> Vec<&Entry> {
    entries.iter().filter(|e| e.group == group).collect()
}

/// The size shown at the right of a group row.
#[must_use]
pub fn group_size(entries: &[Entry], group: Group) -> ByteSize {
    ByteSize(
        in_group(entries, group)
            .iter()
            .filter_map(|e| e.item.size)
            .map(|s| s.as_u64())
            .sum(),
    )
}

/// "13 items selected: 1 build cache entry, 9 images, 3 containers ·
/// 5.7 GB." followed by the volumes sentence while volumes stay unticked.
#[must_use]
pub fn summary_line(plan: &PrunePlan, entries: &[Entry], checked: &BTreeSet<Group>) -> String {
    let effect = plan.effect();
    let kind_count = |kind: PruneKind| {
        plan.items
            .iter()
            .filter(|i| i.selected && i.kind == kind)
            .count()
    };
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut parts = Vec::new();
    for (kind, one, many) in [
        (
            PruneKind::BuildCache,
            "build cache entry",
            "build cache entries",
        ),
        (PruneKind::Image, "image", "images"),
        (PruneKind::Container, "container", "containers"),
        (PruneKind::Volume, "volume", "volumes"),
        (PruneKind::Network, "network", "networks"),
    ] {
        let n = kind_count(kind);
        if n > 0 {
            parts.push(plural(n, one, many));
        }
    }
    let noun = if effect.count == 1 { "item" } else { "items" };
    let mut text = format!("{} {noun} selected", effect.count);
    if !parts.is_empty() {
        text.push_str(": ");
        text.push_str(&parts.join(", "));
    }
    if let Some(bytes) = effect.bytes {
        text.push_str(&format!(" \u{b7} {}", berth_core::fmt::bytes(bytes)));
    }
    text.push('.');
    let volumes_left = !checked.contains(&Group::UnusedVolumes)
        && entries.iter().any(|e| e.group == Group::UnusedVolumes);
    if volumes_left {
        text.push_str(" Volumes hold data and stay unless you tick them.");
    }
    text
}

// ---------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------

/// Where a run stands after each item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    /// The item being removed next, or the last one when none is left.
    pub current: String,
}

impl Progress {
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            1.0
        } else {
            self.done as f32 / self.total as f32
        }
    }
}

/// What a run did: the items the engine removed and the ones it did not.
/// Items never tried (a cancelled run) are in neither list.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub engine: EngineId,
    pub removed: Vec<PruneItem>,
    pub failed: Vec<(PruneItem, String)>,
    pub freed: ByteSize,
    pub cancelled: bool,
}

impl Outcome {
    /// The effect of what was removed: count and size of the removed items,
    /// the same sum the button showed for the plan when all of it went.
    #[must_use]
    pub fn effect(&self) -> EffectSummary {
        PrunePlan {
            engine: self.engine.clone(),
            items: self.removed.clone(),
        }
        .effect()
    }

    /// "Removed 13 items (5.7 GB)": the button's text in the past tense.
    #[must_use]
    pub fn result_label(&self) -> String {
        self.effect().result_label()
    }

    /// The toast of a clean run: "Removed 13 items · 5.7 GB freed".
    #[must_use]
    pub fn toast_text(&self) -> String {
        let effect = self.effect();
        let mut bare = effect.clone();
        bare.bytes = None;
        match effect.bytes {
            Some(bytes) => format!(
                "{} \u{b7} {} freed",
                bare.result_label(),
                berth_core::fmt::bytes(bytes)
            ),
            None => bare.result_label(),
        }
    }

    /// Nothing failed and nothing was left untried.
    #[must_use]
    pub fn clean(&self) -> bool {
        self.failed.is_empty() && !self.cancelled
    }

    /// "2 items could not be removed" when some failed.
    #[must_use]
    pub fn failure_line(&self) -> Option<String> {
        let n = self.failed.len();
        (n > 0).then(|| {
            format!(
                "{n} {} could not be removed.",
                if n == 1 { "item" } else { "items" }
            )
        })
    }

    /// The activity entry of the run.
    #[must_use]
    pub fn activity(&self) -> berth_core::ActivityEntry {
        let outcome = if self.clean() {
            berth_core::Outcome::Ok
        } else {
            berth_core::Outcome::Failed {
                reason: self
                    .failure_line()
                    .unwrap_or_else(|| "Cancelled".to_string()),
            }
        };
        let effect = self.effect();
        let name = match effect.bytes {
            Some(bytes) => format!(
                "{} {} ({})",
                effect.count,
                if effect.count == 1 { "item" } else { "items" },
                berth_core::fmt::bytes(bytes)
            ),
            None => format!("{} items", effect.count),
        };
        crate::services::activity::entry_for_action(
            self.engine.clone(),
            CLEAN_UP,
            berth_core::Subject::Engine(self.engine.clone()),
            name,
            None,
            outcome,
        )
    }
}

/// Write the run to the activity feed (D65).
pub fn record(store: &berth_store::Store, outcome: &Outcome) {
    crate::services::activity::record(store, &outcome.activity());
}

/// What the sheet says after a run that did not finish clean: what went,
/// then what did not.
#[must_use]
pub fn partial_notice(outcome: &Outcome) -> String {
    let mut parts = Vec::new();
    if !outcome.removed.is_empty() {
        parts.push(format!("{}.", outcome.result_label()));
    }
    if let Some(line) = outcome.failure_line() {
        parts.push(line);
    }
    if outcome.cancelled {
        parts.push("Stopped before the end.".to_string());
    }
    parts.join(" ")
}

/// The entries still on the sheet after a run: everything that was not
/// removed. The ones that failed carry the engine's reason.
#[must_use]
pub fn remaining(entries: Vec<Entry>, outcome: &Outcome) -> Vec<Entry> {
    entries
        .into_iter()
        .filter(|e| {
            !outcome
                .removed
                .iter()
                .any(|r| r.kind == e.item.kind && r.id == e.item.id)
        })
        .map(|mut e| {
            if let Some((_, reason)) = outcome
                .failed
                .iter()
                .find(|(f, _)| f.kind == e.item.kind && f.id == e.item.id)
            {
                e.failure = Some(reason.clone());
            }
            e
        })
        .collect()
}

/// The selected items in the order they run: build cache once, last.
fn run_order(plan: &PrunePlan) -> Vec<PruneItem> {
    let mut items: Vec<PruneItem> = plan.items.iter().filter(|i| i.selected).cloned().collect();
    let mut cache_seen = false;
    items.retain(|i| {
        if i.kind == PruneKind::BuildCache {
            !std::mem::replace(&mut cache_seen, true)
        } else {
            true
        }
    });
    items.sort_by_key(|i| i.kind == PruneKind::BuildCache);
    items
}

/// Remove the selected items of `plan` one at a time through
/// `EngineClient::prune`, counting the reports. `on_progress` is called
/// before the first item and after each one. A cancelled run stops where it
/// is; what it did not try is in neither list of the outcome.
pub async fn run(
    client: &dyn EngineClient,
    plan: &PrunePlan,
    cancel: CancellationToken,
    mut on_progress: impl FnMut(Progress) + Send,
) -> Outcome {
    let items = run_order(plan);
    let total = items.len();
    let mut outcome = Outcome {
        engine: plan.engine.clone(),
        removed: Vec::new(),
        failed: Vec::new(),
        freed: ByteSize(0),
        cancelled: false,
    };
    let mut freed: u64 = 0;
    let mut tried = 0_usize;
    for item in items {
        if cancel.is_cancelled() {
            outcome.cancelled = true;
            break;
        }
        on_progress(Progress {
            done: tried,
            total,
            current: item.name.clone(),
        });
        tried += 1;
        if let Some(reason) = refuse_running(client, &item).await {
            outcome.failed.push((item, reason));
            continue;
        }
        let one = PrunePlan {
            engine: plan.engine.clone(),
            items: vec![item.clone()],
        };
        match client.prune(&one, cancel.clone()).await {
            Ok(report) => {
                freed = freed.saturating_add(report.freed.as_u64());
                let reported = report.removed.len() + report.failed.len();
                outcome.removed.extend(report.removed);
                outcome.failed.extend(report.failed);
                if reported == 0 {
                    outcome
                        .failed
                        .push((item, "The engine did not report this item".to_string()));
                }
            }
            Err(berth_engine::EngineError::Cancelled) => {
                outcome.cancelled = true;
                break;
            }
            Err(err) => outcome.failed.push((item, err.to_string())),
        }
    }
    outcome.freed = ByteSize(freed);
    on_progress(Progress {
        done: tried,
        total,
        current: String::new(),
    });
    outcome
}

/// The engine force-removes containers in a plan, so a container that was
/// started after the list was read is refused here instead.
async fn refuse_running(client: &dyn EngineClient, item: &PruneItem) -> Option<String> {
    if item.kind != PruneKind::Container {
        return None;
    }
    let detail = client.container(&ContainerId(item.id.clone())).await.ok()?;
    let state = detail.container.state;
    matches!(
        state,
        ContainerState::Running | ContainerState::Paused | ContainerState::Restarting
    )
    .then(|| format!("{} is running again", item.name))
}

#[cfg(test)]
#[path = "prune_tests.rs"]
mod tests;
