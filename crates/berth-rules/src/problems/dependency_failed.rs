//! DependencyFailed: a service did not start because a service it needs failed.

use std::sync::LazyLock;

use berth_core::{
    ComposeCommand, ComposeTarget, EffectKind, EngineAction, Fix, Problem, ProblemKind, Severity,
    Subject,
};

use super::support::{action_fix, caps, effect, explain_fix, line, re, Ctx, Re};
use super::ProjectFacts;

static CONTAINER: Re = LazyLock::new(|| {
    re(
        r"dependency failed to start: container ([A-Za-z0-9][A-Za-z0-9_.-]*) (?:is unhealthy|exited|has no healthcheck)",
    )
});
static SERVICE: Re =
    LazyLock::new(|| re(r#"service "([A-Za-z0-9][A-Za-z0-9_.-]*)" didn't complete successfully"#));

/// The service a container name stands for: the container's own service label, else the name
/// without the project prefix and the `-1` suffix Compose adds.
fn service_of(ctx: &Ctx<'_>, project: &ProjectFacts, container: &str) -> String {
    if let Some(service) = ctx
        .container_by_name(container)
        .and_then(|c| c.container.service.clone())
    {
        return service;
    }
    let name = container
        .strip_prefix(&format!("{}-", project.project.name))
        .unwrap_or(container);
    let without_index = name
        .rsplit_once('-')
        .filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit()))
        .map_or(name, |(head, _)| head);
    without_index.to_string()
}

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for project_facts in &ctx.projects {
        let mut seen: Vec<String> = Vec::new();
        for text in &project_facts.failure_lines {
            let service = if let Some(c) = caps(&CONTAINER, text) {
                service_of(ctx, project_facts, &c[1])
            } else if let Some(c) = caps(&SERVICE, text) {
                c[1].to_string()
            } else {
                continue;
            };
            if seen.contains(&service) {
                continue;
            }
            seen.push(service.clone());
            let project = &project_facts.project;
            out.push(Problem {
                evidence: ctx.evidence([text]),
                fixes: vec![fix(ctx, project_facts, &service)],
                ..ctx.problem(
                    ProblemKind::DependencyFailed {
                        service: service.clone(),
                    },
                    Severity::Error,
                    Subject::Project(project.id.clone()),
                    format!("{service} failed, so the services that need it did not start"),
                    format!(
                        "Other services of {} wait for {service} and only start once it is \
                         healthy or has finished. {service} did not get there, so Compose \
                         stopped. Look at {service} first: its own problem, or its log, says \
                         why. Starting the others will not help until it works.",
                        project.name
                    ),
                )
            });
        }
    }
    out
}

fn fix(ctx: &Ctx<'_>, project_facts: &ProjectFacts, service: &str) -> Fix {
    let project = &project_facts.project;
    let target = ctx
        .input
        .engine
        .docker_host
        .clone()
        .and_then(|docker_host| {
            let folder = project.folder.clone()?;
            if project.compose_files.is_empty() {
                return None;
            }
            Some(ComposeTarget {
                engine: project.engine.clone(),
                docker_host,
                project_name: project.name.clone(),
                folder,
                files: project.compose_files.clone(),
            })
        });
    match target {
        Some(target) => action_fix(
            format!("Restart {service}"),
            EngineAction::Compose {
                target,
                command: ComposeCommand::Restart {
                    services: vec![service.to_string()],
                },
            },
            effect(
                "Restart",
                "service",
                "services",
                1,
                vec![
                    line(
                        EffectKind::Changes,
                        format!("{service} stops and starts again"),
                    ),
                    line(EffectKind::Stays, "Its data and the other services"),
                ],
            ),
        ),
        None => explain_fix(format!("Look at {service} first")),
    }
}
