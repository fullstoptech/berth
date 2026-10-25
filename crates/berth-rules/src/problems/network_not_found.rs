//! NetworkNotFound: a service joins a network that does not exist on the engine.

use std::sync::LazyLock;

use berth_core::{Problem, ProblemKind, Severity};

use super::support::{caps, explain_fix, re, Ctx, Re};

static MISSING: Re = LazyLock::new(|| {
    re(
        r#"network "?([A-Za-z0-9][A-Za-z0-9_.-]*)"? (?:not found|declared as external, but could not be found)"#,
    )
});

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for source in ctx.sources() {
        let mut seen: Vec<String> = Vec::new();
        for text in &source.texts {
            let Some(c) = caps(&MISSING, text) else {
                continue;
            };
            let name = c[1].to_string();
            if seen.contains(&name) {
                continue;
            }
            seen.push(name.clone());
            out.push(Problem {
                evidence: ctx.evidence([text]),
                fixes: vec![explain_fix(format!(
                    "Create the network {name}, or let Compose create it"
                ))],
                ..ctx.problem(
                    ProblemKind::NetworkNotFound { name: name.clone() },
                    Severity::Error,
                    source.subject.clone(),
                    format!("The network {name} does not exist"),
                    format!(
                        "A service wants to join the network {name}, but the engine has no \
                         network with that name. A network marked external is never created by \
                         Compose. Create it once with docker network create {name}, or remove \
                         the external setting so that Compose creates it."
                    ),
                )
            });
        }
    }
    out
}
