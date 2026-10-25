//! ImageNotFound: the registry has no such image or tag, or refuses to give it out.
//!
//! Classified from the error text alone; whether the tag exists is not checked offline.

use std::sync::LazyLock;

use berth_core::{Problem, ProblemKind, Severity};

use super::support::{caps, compose_services, explain_fix, re, Ctx, Re, Source};

static DENIED: Re = LazyLock::new(|| re(r"pull access denied for ([^\s,]+)"));
static MANIFEST: Re = LazyLock::new(|| re(r"manifest for (\S+) not found"));
static REPOSITORY: Re = LazyLock::new(|| re(r"repository (\S+?) not found"));
static NO_SUCH: Re = LazyLock::new(|| re(r"No such image: (\S+)"));
static HEAD: Re = LazyLock::new(|| {
    re(
        r#"(?:Head|Get) "https?://[^/"]+/v2/([^"]+)/manifests/([^"]+)": (?:denied|unauthorized|manifest unknown)"#,
    )
});
/// A Compose progress line starts with the service: ` ! badimage Error pull access denied ...`.
static LEADING_SERVICE: Re =
    LazyLock::new(|| re(r"^\s*(?:[!✘⠿]\s+)?([A-Za-z0-9][A-Za-z0-9_.-]*)\s+(?:Error|error)\b"));

/// Strips a registry-added `docker.io/` or `library/` so references compare.
fn short(reference: &str) -> &str {
    let r = reference.strip_prefix("docker.io/").unwrap_or(reference);
    r.strip_prefix("library/").unwrap_or(r)
}

fn repo_of(reference: &str) -> &str {
    let no_digest = reference.split('@').next().unwrap_or(reference);
    let after_slash = no_digest.rfind('/').map_or(0, |i| i + 1);
    match no_digest[after_slash..].find(':') {
        Some(i) => &no_digest[..after_slash + i],
        None => no_digest,
    }
}

/// The reference a line complains about, and whether the registry said "denied".
fn reference_in(text: &str) -> Option<(String, bool)> {
    if let Some(c) = caps(&DENIED, text) {
        return Some((c.get(1)?.as_str().to_string(), true));
    }
    if let Some(c) = caps(&MANIFEST, text) {
        return Some((c.get(1)?.as_str().to_string(), false));
    }
    if let Some(c) = caps(&HEAD, text) {
        let denied = !text.contains("manifest unknown");
        return Some((
            format!("{}:{}", c.get(1)?.as_str(), c.get(2)?.as_str()),
            denied,
        ));
    }
    if let Some(c) = caps(&REPOSITORY, text) {
        return Some((c.get(1)?.as_str().to_string(), true));
    }
    if let Some(c) = caps(&NO_SUCH, text) {
        return Some((c.get(1)?.as_str().to_string(), false));
    }
    None
}

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for source in ctx.sources() {
        let mut seen: Vec<String> = Vec::new();
        for text in &source.texts {
            let Some((raw, denied)) = reference_in(text) else {
                continue;
            };
            let (reference, service) = resolve(&source, text, &raw);
            if seen.contains(&reference) {
                continue;
            }
            seen.push(reference.clone());
            out.push(problem(ctx, &source, text, reference, service, denied));
        }
    }
    out
}

/// The full reference and the service it belongs to, from the Compose file when it has the image.
fn resolve(source: &Source<'_>, text: &str, raw: &str) -> (String, Option<String>) {
    let leading = caps(&LEADING_SERVICE, text).map(|c| c[1].to_string());
    if let Some(cf) = source.container {
        let reference = if cf.container.image.repo() == short(repo_of(raw)) || raw.is_empty() {
            cf.container.image.reference.clone()
        } else {
            raw.to_string()
        };
        return (reference, cf.container.service.clone());
    }
    let defs = source
        .project
        .and_then(|p| p.compose_text.as_deref())
        .map(compose_services)
        .unwrap_or_default();
    let by_leading = leading
        .as_deref()
        .and_then(|name| defs.iter().find(|d| d.name == name));
    let by_image = defs.iter().find(|d| {
        d.image
            .as_deref()
            .is_some_and(|i| short(repo_of(i)) == short(repo_of(raw)))
    });
    match by_leading.or(by_image) {
        Some(def) => (
            def.image.clone().unwrap_or_else(|| raw.to_string()),
            Some(def.name.clone()),
        ),
        None => (raw.to_string(), leading),
    }
}

fn problem(
    ctx: &Ctx<'_>,
    source: &Source<'_>,
    text: &str,
    reference: String,
    service: Option<String>,
    denied: bool,
) -> Problem {
    let for_whom = service
        .as_deref()
        .map_or_else(String::new, |s| format!(" for {s}"));
    let why = if denied {
        format!(
            "The registry answered that {reference} does not exist, or that it is private and \
             needs a login. A misspelled name looks exactly like a private image."
        )
    } else {
        format!(
            "The registry knows the name but has nothing under that tag, or the image is missing \
             on this engine. Tags are case sensitive: check {reference} against the registry page."
        )
    };
    Problem {
        evidence: ctx.evidence([text]),
        fixes: vec![explain_fix(format!(
            "Check the name and tag of {reference}, or run docker login"
        ))],
        ..ctx.problem(
            ProblemKind::ImageNotFound {
                reference: reference.clone(),
            },
            Severity::Error,
            source.subject.clone(),
            format!("Image {reference} was not found{for_whom}"),
            format!("{why} Nothing was started from it."),
        )
    }
}
