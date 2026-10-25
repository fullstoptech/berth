//! EnvVarMissing: the Compose file asks for `${VAR}` and nothing gives it a value.

use std::path::Path;
use std::sync::LazyLock;

use berth_core::{Problem, ProblemKind, Severity, Subject};

use super::support::{edit_fix, edit_plan, explain_fix, re, Ctx, Re};
use crate::compose_edit;

/// `${NAME`, optionally followed by an operator (`:-`, `-`, `:+`, `+`, `:?`, `?`), or `$NAME`.
static VAR: Re =
    LazyLock::new(|| re(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(:?[-+?])?|\$([A-Za-z_][A-Za-z0-9_]*)"));

/// Variables the shell or Compose itself provides.
fn provided(name: &str) -> bool {
    matches!(
        name,
        "HOME"
            | "USER"
            | "PWD"
            | "PATH"
            | "SHELL"
            | "LANG"
            | "LOGNAME"
            | "TMPDIR"
            | "HOSTNAME"
            | "UID"
            | "GID"
    ) || name.starts_with("COMPOSE_")
        || name.starts_with("DOCKER_")
}

/// Everything before a `#` that starts a comment.
fn code_of(line: &str) -> &str {
    let mut prev_space = true;
    for (i, ch) in line.char_indices() {
        if ch == '#' && prev_space {
            return &line[..i];
        }
        prev_space = ch.is_whitespace();
    }
    line
}

/// The variables the file needs a value for, with the line of the first use: no default, or a
/// default that is an error message (`${VAR:?text}`).
fn required_vars(compose: &str) -> Vec<(String, usize)> {
    let Some(re) = VAR.as_ref() else {
        return Vec::new();
    };
    let mut out: Vec<(String, usize)> = Vec::new();
    for (index, line) in compose.lines().enumerate() {
        let code = code_of(line);
        for c in re.captures_iter(code) {
            let Some(whole) = c.get(0) else { continue };
            // `$$` is a literal dollar sign.
            if code[..whole.start()].ends_with('$') {
                continue;
            }
            let required = match c.get(1) {
                Some(_) => c.get(2).is_none_or(|op| op.as_str().contains('?')),
                None => true,
            };
            let name = c
                .get(1)
                .or_else(|| c.get(3))
                .map(|m| m.as_str().to_string());
            let Some(name) = name else { continue };
            if required && !provided(&name) && !out.iter().any(|(n, _)| *n == name) {
                out.push((name, index + 1));
            }
        }
    }
    out
}

/// The keys an `.env` file sets.
fn env_keys(env_text: &str) -> Vec<String> {
    env_text
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.starts_with('#') {
                return None;
            }
            let l = l.strip_prefix("export ").unwrap_or(l);
            let (key, _) = l.split_once('=')?;
            let key = key.trim();
            (!key.is_empty()).then(|| key.to_string())
        })
        .collect()
}

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for project_facts in &ctx.projects {
        let Some(compose) = project_facts.compose_text.as_deref() else {
            continue;
        };
        let project = &project_facts.project;
        let set = env_keys(project_facts.env_text.as_deref().unwrap_or(""));
        for (var, line) in required_vars(compose) {
            if set.contains(&var) {
                continue;
            }
            let kind = ProblemKind::EnvVarMissing { var: var.clone() };
            let fix = project.folder.as_ref().and_then(|_| {
                let old = project_facts.env_text.as_deref();
                let new = compose_edit::add_env_lines(old, &[(var.clone(), String::new())]);
                let plan = edit_plan(project, &kind, Path::new(".env"), old, &new, &[])?;
                Some(edit_fix(format!("Add {var} to .env"), plan))
            });
            out.push(Problem {
                evidence: ctx.evidence([format!(
                    "The Compose file uses ${{{var}}} on line {line} and .env does not set it"
                )]),
                fixes: vec![fix.unwrap_or_else(|| explain_fix(format!("Set {var} in .env")))],
                ..ctx.problem(
                    kind,
                    Severity::Warning,
                    Subject::Project(project.id.clone()),
                    format!("{var} has no value"),
                    format!(
                        "The Compose file of {} reads the variable {var}, but there is no value \
                         for it in the .env file and the file gives no default. Compose then \
                         uses an empty text, so the service starts with a blank setting. Add \
                         {var} to .env and give it its value.",
                        project.name
                    ),
                )
            });
        }
    }
    out
}
