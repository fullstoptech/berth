//! Unhealthy: the container runs, but its health check keeps failing.

use berth_core::{ContainerAction, ContainerState, EffectKind, Health, Problem, ProblemKind};
use berth_core::{Severity, Subject};

use super::support::{action_fix, container_action, effect, line, shown_name, Ctx};

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for cf in &ctx.containers {
        let c = &cf.container;
        if c.health != Some(Health::Unhealthy)
            || !matches!(
                c.state,
                ContainerState::Running | ContainerState::Restarting
            )
        {
            continue;
        }
        let name = shown_name(c);
        let mut evidence = vec!["The health check reports unhealthy".to_string()];
        let mut test = None;
        if let Some(detail) = &cf.detail {
            test = detail
                .inspect
                .pointer("/Config/Healthcheck/Test")
                .and_then(|t| t.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|i| i.as_str())
                        .filter(|s| !matches!(*s, "CMD" | "CMD-SHELL" | "NONE"))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .filter(|t| !t.is_empty());
            if let Some(t) = &test {
                evidence.push(format!("Check: {t}"));
            }
            for run in detail.health_log.iter().rev().take(3).rev() {
                let output = run.output.trim();
                evidence.push(if output.is_empty() {
                    format!("Exit code {} with no output", run.exit_code)
                } else {
                    format!("Exit code {}: {output}", run.exit_code)
                });
            }
        }
        let check_text = test.map_or_else(String::new, |t| format!(" The check runs: {t}."));
        let summary = effect(
            "Restart",
            "container",
            "containers",
            1,
            vec![
                line(
                    EffectKind::Changes,
                    format!("{name} stops and starts again"),
                ),
                line(EffectKind::Stays, "Its data and settings"),
            ],
        );
        out.push(Problem {
            evidence: ctx.evidence(evidence),
            fixes: vec![action_fix(
                format!("Restart {name}"),
                container_action(ctx, &c.id, ContainerAction::Restart { timeout_s: None }),
                summary,
            )],
            ..ctx.problem(
                ProblemKind::Unhealthy,
                Severity::Warning,
                Subject::Container(c.id.clone()),
                format!("{name} is unhealthy"),
                format!(
                    "{name} is running, but its health check keeps failing, so the service \
                     inside is probably not answering.{check_text} Read the log lines below the \
                     check, then fix what the check looks for. Restarting helps when the service \
                     only got stuck."
                ),
            )
        });
    }
    out
}
