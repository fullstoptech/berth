//! BindPermission: the engine, or a program in the container, may not use a shared folder.

use std::sync::LazyLock;

use berth_core::{MountKind, Problem, ProblemKind, Severity};

use super::support::{caps, explain_fix, is_match, re, Ctx, Re};

/// The engine cannot create or reach the source of a mount.
static MOUNT_SOURCE: Re = LazyLock::new(|| {
    re(
        r"(?i)creating mount source path '([^']+)'.*(?:permission denied|operation not permitted|read-only file system)",
    )
});
static NOT_SHARED: Re = LazyLock::new(|| re(r"(?i)mounts denied.*?path (\S+) is not shared"));
static DENIED: Re = LazyLock::new(|| re(r"(?i)permission denied|operation not permitted|EACCES"));

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for source in ctx.sources() {
        let mut seen: Vec<String> = Vec::new();
        for text in &source.texts {
            let path = caps(&MOUNT_SOURCE, text)
                .or_else(|| caps(&NOT_SHARED, text))
                .map(|c| c[1].to_string())
                .or_else(|| inside_container(&source, text));
            let Some(path) = path else { continue };
            if seen.contains(&path) {
                continue;
            }
            seen.push(path.clone());
            out.push(Problem {
                evidence: ctx.evidence([text]),
                fixes: vec![explain_fix(format!("Allow access to {path}"))],
                ..ctx.problem(
                    ProblemKind::BindPermission { path: path.clone() },
                    Severity::Error,
                    source.subject.clone(),
                    format!("No permission to use {path}"),
                    format!(
                        "The folder {path} is shared into a container, but it may not be used. \
                         Either the engine's machine does not share that folder (Colima shares \
                         your home folder by default), macOS asks for your consent for folders \
                         such as Documents and Desktop, or the files belong to a user the \
                         program in the container cannot act as. Share the folder, grant access \
                         in System Settings, or run the container as the owner of the files."
                    ),
                )
            });
        }
    }
    out
}

/// A "permission denied" from the program itself, about a path inside a bind mount: the host
/// folder behind that mount is the one to fix.
fn inside_container(source: &super::support::Source<'_>, text: &str) -> Option<String> {
    if !is_match(&DENIED, text) {
        return None;
    }
    let cf = source.container?;
    cf.container
        .mounts
        .iter()
        .filter(|m| m.kind == MountKind::Bind && m.destination.len() > 1)
        .filter(|m| {
            let d = m.destination.trim_end_matches('/');
            text.contains(&format!("{d}/")) || text.contains(&format!("'{d}'"))
        })
        .max_by_key(|m| m.destination.len())
        .map(|m| m.source.clone())
}
