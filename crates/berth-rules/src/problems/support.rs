//! Helpers the problem rules share: the per-engine view of the input, text sources, a small
//! reader for Compose files, and constructors for fixes.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use berth_core::{
    Container, ContainerId, EffectKind, EffectLine, EffectSummary, EngineAction, Fix, FixEffect,
    FixSource, GenerationPlan, PlanSource, Problem, ProblemId, ProblemKind, Project, Severity,
    Subject,
};
use regex::Regex;

use super::{ContainerFacts, ProblemInput, ProjectFacts};
use crate::diff::file_diff;

/// A regex compiled on first use. A pattern that fails to compile never matches (the rules'
/// positive tests fail then).
pub(super) type Re = LazyLock<Option<Regex>>;

pub(super) fn re(pattern: &str) -> Option<Regex> {
    Regex::new(pattern).ok()
}

pub(super) fn caps<'t>(re: &Re, text: &'t str) -> Option<regex::Captures<'t>> {
    re.as_ref()?.captures(text)
}

pub(super) fn is_match(re: &Re, text: &str) -> bool {
    re.as_ref().is_some_and(|r| r.is_match(text))
}

const MASK: &str = "••••";
const EVIDENCE_LINES: usize = 6;
const EVIDENCE_CHARS: usize = 240;

/// The input narrowed to one engine (AD27), plus the env values to keep out of anything shown.
pub(super) struct Ctx<'a> {
    pub input: &'a ProblemInput,
    pub containers: Vec<&'a ContainerFacts>,
    pub projects: Vec<&'a ProjectFacts>,
    secrets: Vec<String>,
}

/// Where a rule looks for error text: one container's, or one project's last failed command.
pub(super) struct Source<'a> {
    pub subject: Subject,
    pub texts: Vec<String>,
    pub container: Option<&'a ContainerFacts>,
    pub project: Option<&'a ProjectFacts>,
}

impl<'a> Ctx<'a> {
    pub fn new(input: &'a ProblemInput) -> Self {
        let engine = &input.engine.id;
        let containers: Vec<&ContainerFacts> = input
            .containers
            .iter()
            .filter(|c| &c.container.engine == engine)
            .collect();
        let projects = input
            .projects
            .iter()
            .filter(|p| &p.project.engine == engine)
            .collect();
        let secrets = containers
            .iter()
            .filter_map(|c| c.detail.as_ref())
            .flat_map(|d| d.env.iter())
            .map(|var| var.value.clone())
            .filter(|value| value.chars().count() >= 4)
            .collect();
        Self {
            input,
            containers,
            projects,
            secrets,
        }
    }

    pub fn project_of(&self, container: &Container) -> Option<&'a ProjectFacts> {
        let id = container.project.as_ref()?;
        self.projects.iter().copied().find(|p| &p.project.id == id)
    }

    pub fn container_by_name(&self, name: &str) -> Option<&'a ContainerFacts> {
        let name = name.trim_start_matches('/');
        self.containers
            .iter()
            .copied()
            .find(|c| c.container.name.trim_start_matches('/') == name)
    }

    /// One source per container, then one per project.
    pub fn sources(&self) -> Vec<Source<'a>> {
        let mut out: Vec<Source<'a>> = self
            .containers
            .iter()
            .map(|cf| Source {
                subject: Subject::Container(cf.container.id.clone()),
                texts: container_texts(cf),
                container: Some(cf),
                project: self.project_of(&cf.container),
            })
            .collect();
        out.extend(self.projects.iter().map(|pf| {
            Source {
                subject: Subject::Project(pf.project.id.clone()),
                texts: pf
                    .failure_lines
                    .iter()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect(),
                container: None,
                project: Some(pf),
            }
        }));
        out
    }

    /// Evidence for a problem card: trimmed, shortened, env values masked, at most six lines.
    pub fn evidence<I, S>(&self, lines: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out: Vec<String> = Vec::new();
        for line in lines {
            let mut text = line.as_ref().trim().to_string();
            if text.is_empty() {
                continue;
            }
            for secret in &self.secrets {
                text = text.replace(secret.as_str(), MASK);
            }
            if text.chars().count() > EVIDENCE_CHARS {
                text = text.chars().take(EVIDENCE_CHARS).collect::<String>() + "…";
            }
            if !out.contains(&text) {
                out.push(text);
            }
            if out.len() == EVIDENCE_LINES {
                break;
            }
        }
        out
    }

    /// A problem without evidence or fixes; the rule fills those in.
    pub fn problem(
        &self,
        kind: ProblemKind,
        severity: Severity,
        subject: Subject,
        title: String,
        explanation: String,
    ) -> Problem {
        Problem {
            id: ProblemId::new(&kind, &subject),
            kind,
            severity,
            subject,
            title,
            explanation,
            evidence: Vec::new(),
            fixes: Vec::new(),
            first_seen: self.input.now,
        }
    }
}

/// What a container says about its own failure: the engine's start error, the error lines of its
/// log, then its last lines.
pub(super) fn container_texts(cf: &ContainerFacts) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let all = state_error(cf)
        .into_iter()
        .chain(cf.error_lines.iter().cloned())
        .chain(cf.last_lines.iter().cloned());
    for line in all {
        let line = line.trim().to_string();
        if !line.is_empty() && !out.contains(&line) {
            out.push(line);
        }
    }
    out
}

/// `State.Error` of the inspect: why the engine could not start the container.
pub(super) fn state_error(cf: &ContainerFacts) -> Option<String> {
    let text = cf
        .detail
        .as_ref()?
        .inspect
        .pointer("/State/Error")?
        .as_str()?
        .trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// `State.OOMKilled` of the inspect.
pub(super) fn oom_killed(cf: &ContainerFacts) -> bool {
    cf.detail
        .as_ref()
        .and_then(|d| d.inspect.pointer("/State/OOMKilled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------------------------
// Compose files
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PortDef {
    pub host: Option<u16>,
    pub container: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ServiceDef {
    pub name: String,
    pub image: Option<String>,
    pub ports: Vec<PortDef>,
    /// The service sets `deploy.resources.limits.memory`, which `mem_limit` would contradict.
    pub deploy_memory: bool,
}

fn port_of(value: &serde_norway::Value) -> Option<PortDef> {
    if let Some(n) = value.as_u64() {
        return Some(PortDef {
            host: None,
            container: u16::try_from(n).ok()?,
        });
    }
    if let Some(spec) = value.as_str() {
        let base = spec.split('/').next()?;
        let parts: Vec<&str> = base.split(':').collect();
        let container = parts.last()?.trim().parse::<u16>().ok()?;
        let host = parts
            .len()
            .checked_sub(2)
            .and_then(|i| parts.get(i))
            .and_then(|h| h.trim().parse::<u16>().ok());
        return Some(PortDef { host, container });
    }
    let target = value.get("target")?.as_u64()?;
    Some(PortDef {
        host: value
            .get("published")
            .and_then(|p| p.as_u64().or_else(|| p.as_str()?.parse().ok()))
            .and_then(|p| u16::try_from(p).ok()),
        container: u16::try_from(target).ok()?,
    })
}

/// The services of a Compose file with the facts the rules need. A file that does not parse has none.
pub(super) fn compose_services(text: &str) -> Vec<ServiceDef> {
    let Ok(doc) = serde_norway::from_str::<serde_norway::Value>(text) else {
        return Vec::new();
    };
    let Some(services) = doc.get("services").and_then(|s| s.as_mapping()) else {
        return Vec::new();
    };
    services
        .iter()
        .filter_map(|(name, def)| {
            let name = name.as_str()?.to_string();
            let ports = def
                .get("ports")
                .and_then(|p| p.as_sequence())
                .map(|items| items.iter().filter_map(port_of).collect())
                .unwrap_or_default();
            Some(ServiceDef {
                name,
                image: def
                    .get("image")
                    .and_then(|i| i.as_str())
                    .map(ToString::to_string),
                ports,
                deploy_memory: def
                    .get("deploy")
                    .and_then(|d| d.get("resources"))
                    .and_then(|r| r.get("limits"))
                    .and_then(|l| l.get("memory"))
                    .is_some(),
            })
        })
        .collect()
}

/// The first Compose file of a project, relative to the project folder.
pub(super) fn compose_rel(project: &Project) -> Option<PathBuf> {
    let first = project.compose_files.first()?;
    if first.is_relative() {
        return Some(first.clone());
    }
    let folder = project.folder.as_ref()?;
    first.strip_prefix(folder).ok().map(Path::to_path_buf)
}

/// A one-file plan that changes `old` into `new`, starting `services` after the write.
pub(super) fn edit_plan(
    project: &Project,
    kind: &ProblemKind,
    rel: &Path,
    old: Option<&str>,
    new: &str,
    services: &[&str],
) -> Option<GenerationPlan> {
    Some(GenerationPlan {
        root: project.folder.clone()?,
        files: vec![file_diff(rel, old, Some(new))],
        source: PlanSource::Rule(kind.clone()),
        notes: Vec::new(),
        after_write: Some(berth_core::ComposeCommand::Up {
            services: services.iter().map(ToString::to_string).collect(),
            build: false,
        }),
    })
}

// ---------------------------------------------------------------------------------------------
// Fixes
// ---------------------------------------------------------------------------------------------

pub(super) fn line(kind: EffectKind, text: impl Into<String>) -> EffectLine {
    EffectLine {
        kind,
        text: text.into(),
    }
}

/// An `EffectSummary` whose verb is one `berth_core` knows the past tense of.
pub(super) fn effect(
    verb: &str,
    noun_one: &str,
    noun_many: &str,
    count: u32,
    lines: Vec<EffectLine>,
) -> EffectSummary {
    EffectSummary {
        verb: verb.to_string(),
        noun_one: noun_one.to_string(),
        noun_many: noun_many.to_string(),
        count,
        bytes: None,
        lines,
        typed_name: None,
    }
}

pub(super) fn edit_fix(label: impl Into<String>, plan: GenerationPlan) -> Fix {
    Fix {
        label: label.into(),
        source: FixSource::Rule,
        effect: FixEffect::EditFiles(plan),
    }
}

pub(super) fn action_fix(
    label: impl Into<String>,
    action: EngineAction,
    effect: EffectSummary,
) -> Fix {
    Fix {
        label: label.into(),
        source: FixSource::Rule,
        effect: FixEffect::RunAction { action, effect },
    }
}

pub(super) fn explain_fix(label: impl Into<String>) -> Fix {
    Fix {
        label: label.into(),
        source: FixSource::Rule,
        effect: FixEffect::Explain,
    }
}

pub(super) fn container_action(
    ctx: &Ctx<'_>,
    id: &ContainerId,
    action: berth_core::ContainerAction,
) -> EngineAction {
    EngineAction::Container {
        engine: ctx.input.engine.id.clone(),
        id: id.clone(),
        action,
    }
}

/// The name a person knows a container by.
pub(super) fn shown_name(container: &Container) -> &str {
    container.name.trim_start_matches('/')
}
