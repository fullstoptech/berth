//! BuildFailed: an image build stopped on an error.

use std::sync::LazyLock;

use berth_core::{Problem, ProblemKind, Severity, Subject};

use super::support::{caps, explain_fix, is_match, re, Ctx, Re};

static BUILD_ERROR: Re =
    LazyLock::new(|| re(r"(?i)failed to solve|failed to build|ERROR \[[^\]]+\]"));
/// BuildKit's own marker: ` => ERROR [web 3/5] RUN npm ci`.
static STEP: Re = LazyLock::new(|| re(r"ERROR \[[^\]]*\]\s+(.+)$"));
/// The step BuildKit prints under the dashes: ` > [web 3/5] RUN npm ci:`.
static LISTED_STEP: Re = LazyLock::new(|| re(r"^\s*> \[[^\]]*\]\s+(.+?):?\s*$"));
static PROCESS: Re = LazyLock::new(|| re(r#"process "([^"]+)" did not complete successfully"#));

/// The step that failed, as a person reads it: `RUN npm ci`.
fn step_of(lines: &[String]) -> Option<String> {
    if let Some(c) = lines.iter().find_map(|l| caps(&STEP, l)) {
        return Some(c[1].trim().to_string());
    }
    if let Some(c) = lines.iter().find_map(|l| caps(&LISTED_STEP, l)) {
        return Some(c[1].trim().to_string());
    }
    let process = lines.iter().find_map(|l| caps(&PROCESS, l))?;
    let command = process[1].trim();
    let command = command
        .strip_prefix("/bin/sh -c ")
        .unwrap_or(command)
        .trim();
    Some(format!("RUN {command}"))
}

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for project_facts in &ctx.projects {
        let lines = &project_facts.failure_lines;
        if !lines.iter().any(|l| is_match(&BUILD_ERROR, l)) {
            continue;
        }
        let step = step_of(lines);
        let project = &project_facts.project;
        let at = step
            .as_deref()
            .map_or_else(String::new, |s| format!(" at the step {s}"));
        let evidence: Vec<&String> = lines
            .iter()
            .filter(|l| is_match(&BUILD_ERROR, l) || caps(&PROCESS, l).is_some())
            .collect();
        out.push(Problem {
            evidence: ctx.evidence(evidence),
            fixes: vec![explain_fix("Read the build output above the error")],
            ..ctx.problem(
                ProblemKind::BuildFailed { step: step.clone() },
                Severity::Error,
                Subject::Project(project.id.clone()),
                step.as_deref().map_or_else(
                    || format!("The build of {} failed", project.name),
                    |s| format!("The build of {} failed at {s}", project.name),
                ),
                format!(
                    "Docker could not build an image for {}{at}. The command in that step \
                     returned an error, so no image was made and the service did not start. The \
                     lines above the error in the build output say what the command printed.",
                    project.name
                ),
            )
        });
    }
    out
}
