//! BindPathMissing: a bind mount points at a folder that does not exist on this Mac.

use std::sync::LazyLock;

use berth_core::{Problem, ProblemKind, Severity};

use super::support::{caps, explain_fix, re, Ctx, Re};

static MISSING: Re = LazyLock::new(|| re(r#"bind source path does not exist:?\s*['"]?([^\s'"]+)"#));

/// Compose prefixes an error with the service it came from: `service:badmount:1 Error response ...`.
static SERVICE: Re = LazyLock::new(|| re(r"\bservice:([A-Za-z0-9._-]+):\d+"));

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for source in ctx.sources() {
        let mut seen: Vec<String> = Vec::new();
        for text in &source.texts {
            let Some(c) = caps(&MISSING, text) else {
                continue;
            };
            let path = c[1].to_string();
            if seen.contains(&path) {
                continue;
            }
            seen.push(path.clone());
            let service = source
                .container
                .and_then(|cf| cf.container.service.clone())
                .or_else(|| caps(&SERVICE, text).map(|s| s[1].to_string()));
            let whose = service.map_or_else(
                || "A volume of this service".to_string(),
                |s| format!("A volume of the service {s}"),
            );
            out.push(Problem {
                evidence: ctx.evidence([text]),
                fixes: vec![explain_fix(format!(
                    "Create {path} on this Mac, or point the volume at a folder that exists"
                ))],
                ..ctx.problem(
                    ProblemKind::BindPathMissing { path: path.clone() },
                    Severity::Error,
                    source.subject.clone(),
                    format!("The folder {path} does not exist"),
                    format!(
                        "{whose} shares the folder {path} from this Mac, but \
                         that folder is not there, and the setting says not to create it. \
                         Create the folder, or change the path in the Compose file to one that \
                         exists. Nothing was started."
                    ),
                )
            });
        }
    }
    out
}
