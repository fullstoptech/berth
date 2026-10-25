//! BindPathMissing: a bind mount points at a folder that does not exist on this Mac.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use berth_core::{Problem, ProblemId, ProblemKind, Severity, Subject};

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
            out.push(raise(
                ctx,
                &path,
                service,
                source.subject.clone(),
                ctx.evidence([text]),
            ));
        }
    }
    // AD35: binds the service found missing by a stat, before any command failed. The same path
    // already raised from a container or a failure line is not raised twice.
    for project in &ctx.projects {
        for (service, path) in &project.missing_binds {
            let path = path.to_string_lossy().into_owned();
            let subject = Subject::Project(project.project.id.clone());
            let kind = ProblemKind::BindPathMissing { path: path.clone() };
            let id = ProblemId::new(&kind, &subject);
            if out.iter().any(|p| p.id == id) {
                continue;
            }
            let evidence = ctx.evidence([format!("{service}: {path} does not exist")]);
            out.push(raise(ctx, &path, Some(service.clone()), subject, evidence));
        }
    }
    out
}

fn raise(
    ctx: &Ctx<'_>,
    path: &str,
    service: Option<String>,
    subject: Subject,
    evidence: Vec<String>,
) -> Problem {
    let whose = service.map_or_else(
        || "A volume of this service".to_string(),
        |s| format!("A volume of the service {s}"),
    );
    Problem {
        evidence,
        fixes: vec![explain_fix(format!(
            "Create {path} on this Mac, or point the volume at a folder that exists"
        ))],
        ..ctx.problem(
            ProblemKind::BindPathMissing {
                path: path.to_string(),
            },
            Severity::Error,
            subject,
            format!("The folder {path} does not exist"),
            format!(
                "{whose} shares the folder {path} from this Mac, but \
                 that folder is not there, and the setting says not to create it. \
                 Create the folder, or change the path in the Compose file to one that \
                 exists. Nothing was started."
            ),
        )
    }
}

/// The long-syntax bind mounts of a Compose file that say `create_host_path: false`, as
/// (service, absolute source). Pure: the service stats the sources (AD35, D36). A source that
/// needs the environment (`$`) or the home folder (`~`) to resolve is left out.
#[must_use]
pub fn strict_binds(folder: &Path, compose_text: &str) -> Vec<(String, PathBuf)> {
    let Ok(doc) = serde_norway::from_str::<serde_norway::Value>(compose_text) else {
        return Vec::new();
    };
    let Some(services) = doc.get("services").and_then(|s| s.as_mapping()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, def) in services {
        let Some(name) = name.as_str() else {
            continue;
        };
        let Some(volumes) = def.get("volumes").and_then(|v| v.as_sequence()) else {
            continue;
        };
        for volume in volumes {
            let strict = volume.get("type").and_then(|t| t.as_str()) == Some("bind")
                && volume
                    .get("bind")
                    .and_then(|b| b.get("create_host_path"))
                    .and_then(serde_norway::Value::as_bool)
                    == Some(false);
            let Some(source) = volume.get("source").and_then(|s| s.as_str()) else {
                continue;
            };
            if !strict || source.contains('$') || source.starts_with('~') {
                continue;
            }
            let path: PathBuf = folder.join(source).components().collect();
            if !out.iter().any(|(n, p)| n == name && p == &path) {
                out.push((name.to_string(), path));
            }
        }
    }
    out
}
