//! BindPathMissing: a bind mount points at a folder that does not exist on this Mac.

use std::sync::LazyLock;

use berth_core::{Problem, ProblemKind, Severity};

use super::support::{caps, explain_fix, re, Ctx, Re};

static MISSING: Re = LazyLock::new(|| re(r#"bind source path does not exist:?\s*['"]?([^\s'"]+)"#));

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
                        "A volume of this service shares the folder {path} from this Mac, but \
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
