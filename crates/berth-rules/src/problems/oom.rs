//! OutOfMemory: the kernel killed the container for using more memory than its limit.

use berth_core::{
    ContainerSpec, EffectKind, EngineAction, Fix, Problem, ProblemKind, Severity, Subject,
};

use super::support::{
    action_fix, compose_rel, compose_services, edit_fix, edit_plan, effect, explain_fix, line,
    oom_killed, shown_name, Ctx,
};
use super::ContainerFacts;
use crate::compose_edit;

const MIB: u64 = 1024 * 1024;
/// A raised limit is never under this.
const FLOOR: u64 = 64 * MIB;

/// How many times the old limit the new one is: double, and at least `FLOOR`.
fn factor(old: u64) -> u64 {
    FLOOR.div_ceil(old.max(1)).max(2)
}

fn mib(bytes: u64) -> String {
    format!("{} MiB", bytes.div_ceil(MIB))
}

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for cf in &ctx.containers {
        if !oom_killed(cf) {
            continue;
        }
        let name = shown_name(&cf.container);
        // The typed field first; the raw inspect carries the limit when the typed one does not.
        let limit_bytes = cf
            .detail
            .as_ref()
            .and_then(|d| d.limits.memory)
            .map(|l| l.as_u64())
            .or_else(|| {
                cf.detail
                    .as_ref()?
                    .inspect
                    .pointer("/HostConfig/Memory")?
                    .as_u64()
                    .filter(|m| *m > 0)
            });
        let killed = "The container was killed for running out of memory".to_string();
        let (explanation, fixes, evidence) = match limit_bytes {
            Some(old) => {
                let times = factor(old);
                let new = old.saturating_mul(times);
                let fix = raise_fix(ctx, cf, times, new.div_ceil(MIB));
                (
                    format!(
                        "{name} used more memory than its limit of {} and the kernel stopped it. \
                         Give it more room ({}), or find out why it uses so much.",
                        mib(old),
                        mib(new)
                    ),
                    vec![fix.unwrap_or_else(|| {
                        explain_fix(format!("Raise the memory limit of {name} to {}", mib(new)))
                    })],
                    vec![killed, format!("Memory limit: {}", mib(old))],
                )
            }
            None => (
                format!(
                    "{name} has no limit of its own, so the whole engine ran out of memory and \
                     the kernel stopped it. Give the engine more memory, or limit the \
                     containers that grow without end."
                ),
                vec![explain_fix("Give the engine more memory")],
                vec![killed],
            ),
        };
        out.push(Problem {
            evidence: ctx.evidence(evidence),
            fixes,
            ..ctx.problem(
                ProblemKind::OutOfMemory,
                Severity::Error,
                Subject::Container(cf.container.id.clone()),
                format!("{name} ran out of memory"),
                explanation,
            )
        });
    }
    out
}

/// The fix that raises the limit: `mem_limit` in the Compose file, else a recreate of a loose
/// container with the limit multiplied.
fn raise_fix(ctx: &Ctx<'_>, cf: &ContainerFacts, times: u64, new_mib: u64) -> Option<Fix> {
    let name = shown_name(&cf.container);
    let label = format!("Raise the memory limit of {name} to {new_mib} MiB");
    if let Some(service) = cf.container.service.as_deref() {
        let project_facts = ctx.project_of(&cf.container)?;
        let text = project_facts.compose_text.as_deref()?;
        let rel = compose_rel(&project_facts.project)?;
        let def = compose_services(text)
            .into_iter()
            .find(|s| s.name == service)?;
        if def.deploy_memory {
            return None;
        }
        let new_text =
            compose_edit::set_service_key(text, service, "mem_limit", &format!("{new_mib}m"))
                .ok()?;
        let plan = edit_plan(
            &project_facts.project,
            &ProblemKind::OutOfMemory,
            &rel,
            Some(text),
            &new_text,
            &[service],
        )?;
        return Some(edit_fix(label, plan));
    }
    let detail = cf.detail.as_ref()?;
    let mut spec = ContainerSpec::from_detail(detail);
    spec.limits.memory = spec.limits.memory.map(|m| m * times);
    spec.limits.memory?;
    let summary = effect(
        "Recreate",
        "container",
        "containers",
        1,
        vec![
            line(
                EffectKind::Changes,
                format!("{name} may use {new_mib} MiB of memory"),
            ),
            line(EffectKind::Stays, "Its image, volumes and settings"),
        ],
    );
    Some(action_fix(
        label,
        EngineAction::Recreate {
            engine: ctx.input.engine.id.clone(),
            id: cf.container.id.clone(),
            spec,
            pull: false,
        },
        summary,
    ))
}
