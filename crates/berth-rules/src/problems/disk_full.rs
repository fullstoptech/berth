//! DiskFull: the engine's disk has no room left, or nearly none.

use std::sync::LazyLock;

use berth_core::{
    ContainerState, EngineAction, Fix, Problem, ProblemKind, PruneItem, PruneKind, PrunePlan,
    Severity, Subject,
};

use super::support::{action_fix, explain_fix, is_match, re, shown_name, Ctx, Re};

static NO_SPACE: Re = LazyLock::new(|| re(r"(?i)no space left on device|disk quota exceeded"));
/// The share of the disk, in percent, from which a full disk is reported before an error shows it.
const NEARLY_FULL_PERCENT: u64 = 95;

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let said: Vec<String> = ctx
        .sources()
        .into_iter()
        .flat_map(|s| s.texts)
        .filter(|t| is_match(&NO_SPACE, t))
        .collect();
    let usage = usage_percent(ctx);
    let nearly_full = usage.filter(|p| *p >= NEARLY_FULL_PERCENT);
    if said.is_empty() && nearly_full.is_none() {
        return Vec::new();
    }

    let mut evidence: Vec<String> = said.clone();
    if let Some(percent) = usage {
        if percent >= NEARLY_FULL_PERCENT {
            evidence.push(format!("Docker data uses {percent}% of the engine's disk"));
        }
    }
    let how = if said.is_empty() {
        "Docker's images, containers, volumes and build cache fill nearly all of the engine's disk."
    } else {
        "A program was told there is no space left on the engine's disk."
    };
    let mut fixes: Vec<Fix> = Vec::new();
    if let Some(fix) = prune_fix(ctx) {
        fixes.push(fix);
    }
    fixes.push(explain_fix(
        "Review unused images, volumes and build cache in Clean up",
    ));

    vec![Problem {
        evidence: ctx.evidence(evidence),
        fixes,
        ..ctx.problem(
            ProblemKind::DiskFull,
            if said.is_empty() {
                Severity::Warning
            } else {
                Severity::Error
            },
            Subject::Engine(ctx.input.engine.id.clone()),
            "The engine is out of disk space".to_string(),
            format!(
                "{how} Containers cannot write, and pulls and builds fail until space is free. \
                 Remove what you no longer use (Clean up lists it with sizes), or give the \
                 machine a bigger disk."
            ),
        )
    }]
}

/// Docker's data as a share of the disk, in whole percent.
fn usage_percent(ctx: &Ctx<'_>) -> Option<u64> {
    let disk = ctx.input.disk.as_ref()?;
    let total = disk.disk_total?.as_u64();
    if total == 0 {
        return None;
    }
    let used = disk.images.size.as_u64()
        + disk.containers.size.as_u64()
        + disk.volumes.size.as_u64()
        + disk.build_cache.size.as_u64();
    Some(used.saturating_mul(100) / total)
}

/// Removes the stopped containers: the one cleanup that needs no list of images or volumes.
fn prune_fix(ctx: &Ctx<'_>) -> Option<Fix> {
    let items: Vec<PruneItem> = ctx
        .containers
        .iter()
        .filter(|c| {
            matches!(
                c.container.state,
                ContainerState::Exited | ContainerState::Dead
            )
        })
        .map(|c| PruneItem {
            kind: PruneKind::Container,
            id: c.container.id.0.clone(),
            name: shown_name(&c.container).to_string(),
            size: None,
            selected: true,
        })
        .collect();
    if items.is_empty() {
        return None;
    }
    let plan = PrunePlan {
        engine: ctx.input.engine.id.clone(),
        items,
    };
    let summary = plan.effect();
    let label = format!(
        "Remove {} stopped container{}",
        plan.items.len(),
        if plan.items.len() == 1 { "" } else { "s" }
    );
    Some(action_fix(label, EngineAction::Prune { plan }, summary))
}
