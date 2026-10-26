//! The assistant's entry points (P7-T6): one place turns a click
//! somewhere in Berth — log lines, a problem, the Generate sheet —
//! into the session, context chips and first turn it opens with.
//!
//! The builders are pure: they take plain values and return an
//! [`Entry`]. [`run`] is the only executor (open, attach, send) and
//! [`show`] the only navigation, so every entry point behaves the
//! same and the builders are testable without a harness.

use std::path::Path;

use berth_core::{
    Container, ContainerId, ContextItem, Detection, EngineId, FoundKind, Harness, Problem, Project,
    ProjectId, SessionId, StackGuess,
};
use dioxus::prelude::WritableExt as _;

use super::assistant::{AssistantService, OpenRequest};
use crate::state::{AppState, BottomTab, Selection};

/// The selection a Logs context item carries, capped (PLAN §6.2).
pub const LOG_LINES_CAP: usize = 400;

/// Manifests a Generate entry attaches when the folder has them.
/// `.env` is never attached (D79): the harness sees key names through
/// its tools, masked.
const MANIFESTS: &[&str] = &[
    "package.json",
    "pyproject.toml",
    "requirements.txt",
    "go.mod",
    "Cargo.toml",
    "Gemfile",
    "composer.json",
    "pom.xml",
    "build.gradle",
];

/// One entry point, ready to run: which session to open, what to
/// attach, and the first turn (with the line shown above it).
#[derive(Debug, Clone)]
pub struct Entry {
    pub request: OpenRequest,
    pub context: Vec<ContextItem>,
    pub prompt: String,
    pub origin: String,
}

fn request(project: &Project, harness: &Harness) -> Option<OpenRequest> {
    Some(OpenRequest {
        harness: harness.clone(),
        project: project.id.clone(),
        project_name: project.name.clone(),
        folder: project.folder.clone()?,
    })
}

/// The project a container belongs to: its own link, else the project
/// whose services list it, else the Compose project its labels name
/// (the problems model's rule, for the Logs view).
#[must_use]
pub fn project_of_container<'a>(
    containers: &'a [Container],
    projects: &'a [Project],
    id: &ContainerId,
) -> Option<&'a Project> {
    let container = containers.iter().find(|c| c.id == *id)?;
    container
        .project
        .as_ref()
        .and_then(|pid| projects.iter().find(|p| p.id == *pid))
        .or_else(|| {
            projects.iter().find(|p| {
                p.engine == container.engine && p.services.iter().any(|s| s.containers.contains(id))
            })
        })
        .or_else(|| {
            let name = container.compose_project.as_ref()?;
            projects
                .iter()
                .find(|p| p.engine == container.engine && p.name == *name)
        })
}

/// Selected log lines as context: one item per container in
/// first-seen order, capped at [`LOG_LINES_CAP`] lines in total.
#[must_use]
pub fn logs_context(lines: &[(ContainerId, String)], containers: &[Container]) -> Vec<ContextItem> {
    let mut out: Vec<ContextItem> = Vec::new();
    let mut left = LOG_LINES_CAP;
    for (id, text) in lines {
        if left == 0 {
            break;
        }
        left -= 1;
        match out
            .iter_mut()
            .find(|item| matches!(item, ContextItem::Logs { container, .. } if container == id))
        {
            Some(ContextItem::Logs { lines, .. }) => lines.push(text.clone()),
            _ => {
                let name = containers
                    .iter()
                    .find(|c| c.id == *id)
                    .map_or_else(|| id.short().to_string(), |c| c.name.clone());
                out.push(ContextItem::Logs {
                    container: id.clone(),
                    name,
                    lines: vec![text.clone()],
                });
            }
        }
    }
    out
}

fn lines_word(n: usize) -> String {
    if n == 1 {
        "1 line".to_string()
    } else {
        format!("{n} lines")
    }
}

/// "Ask about these lines" (Logs selection). When Berth's rules
/// already matched (`rules_say` holds their titles), the turn says so:
/// Explain is rules first, then the assistant.
#[must_use]
pub fn logs_entry(
    project: &Project,
    harness: &Harness,
    lines: &[(ContainerId, String)],
    containers: &[Container],
    rules_say: &[String],
) -> Option<Entry> {
    if lines.is_empty() {
        return None;
    }
    let context = logs_context(lines, containers);
    let attached: usize = context
        .iter()
        .map(|item| match item {
            ContextItem::Logs { lines, .. } => lines.len(),
            _ => 0,
        })
        .sum();
    let prompt = if rules_say.is_empty() {
        "Explain these log lines. Say what is going wrong, and propose a fix if there is one."
            .to_string()
    } else {
        format!(
            "Berth's rules already matched: {}. Explain these log lines in more depth, and propose a fix if there is one.",
            rules_say.join("; ")
        )
    };
    Some(Entry {
        request: request(project, harness)?,
        context,
        prompt,
        origin: format!("From Logs \u{b7} Ask about {}", lines_word(attached)),
    })
}

/// "Fix with <harness>" on a problem: the problem, its evidence lines,
/// the container's inspect, the compose file and the folder tree.
#[must_use]
pub fn problem_entry(
    project: &Project,
    harness: &Harness,
    problem: &Problem,
    container: Option<&Container>,
) -> Option<Entry> {
    let mut context = vec![ContextItem::Problem(problem.id.clone())];
    if let Some(container) = container {
        if !problem.evidence.is_empty() {
            let lines: Vec<(ContainerId, String)> = problem
                .evidence
                .iter()
                .map(|line| (container.id.clone(), line.clone()))
                .collect();
            context.extend(logs_context(&lines, std::slice::from_ref(container)));
        }
        context.push(ContextItem::Inspect(container.id.clone()));
    }
    if let Some(compose) = project.compose_files.first() {
        context.push(ContextItem::ComposeFile(compose.clone()));
    }
    if let Some(folder) = &project.folder {
        context.push(ContextItem::FolderTree(folder.clone()));
    }
    Some(Entry {
        request: request(project, harness)?,
        context,
        prompt: format!(
            "Fix this problem: {}. {}\nRead the logs and the inspect with your tools, then propose the smallest change that fixes it. Propose file changes with files.propose_edit so I can review them; do not apply anything yourself.",
            problem.title, problem.explanation
        ),
        origin: format!("From Problems \u{b7} {}", problem.title),
    })
}

fn stack_name(stack: &StackGuess) -> String {
    match stack {
        StackGuess::Node {
            framework,
            framework_version,
            pm,
            ..
        } => match (framework, framework_version) {
            (Some(framework), Some(version)) => format!("Node ({framework} {version}, {pm})"),
            (Some(framework), None) => format!("Node ({framework}, {pm})"),
            _ => format!("Node ({pm})"),
        },
        StackGuess::Python { framework, .. } => framework
            .as_ref()
            .map_or_else(|| "Python".to_string(), |f| format!("Python ({f})")),
        StackGuess::Go => "Go".to_string(),
        StackGuess::Rust => "Rust".to_string(),
        StackGuess::Php { framework } => framework
            .as_ref()
            .map_or_else(|| "PHP".to_string(), |f| format!("PHP ({f})")),
        StackGuess::Ruby { framework } => framework
            .as_ref()
            .map_or_else(|| "Ruby".to_string(), |f| format!("Ruby ({f})")),
        StackGuess::Java { framework, .. } => framework
            .as_ref()
            .map_or_else(|| "Java".to_string(), |f| format!("Java ({f})")),
        StackGuess::Static => "a static site".to_string(),
    }
}

/// "Generate with <harness>" for a folder with no project yet. The
/// session's project id is the pinned folder's id
/// (`ProjectId::for_folder`), so the conversation is still there when
/// the folder becomes a project. The files land in Review changes
/// through `files.propose_edit`, like every harness proposal.
#[must_use]
pub fn generate_entry(
    folder: &Path,
    engine: &EngineId,
    harness: &Harness,
    detection: &Detection,
) -> Entry {
    let mut context = vec![ContextItem::FolderTree(folder.to_path_buf())];
    for found in &detection.found {
        match found.kind {
            FoundKind::Dockerfile { .. } => {
                context.push(ContextItem::Dockerfile(found.path.clone()));
            }
            FoundKind::Compose => context.push(ContextItem::ComposeFile(found.path.clone())),
            _ => {}
        }
    }
    for name in MANIFESTS {
        let path = folder.join(name);
        if path.is_file() {
            context.push(ContextItem::File(path));
        }
    }
    let stacks: Vec<String> = detection.stack.iter().map(stack_name).collect();
    let services: Vec<String> = detection
        .services_needed
        .iter()
        .map(|s| format!("{:?} ({})", s.kind, s.reason))
        .collect();
    let mut detected = String::new();
    if !stacks.is_empty() {
        detected.push_str(&format!(" Berth detected {}.", stacks.join(", ")));
    }
    if !services.is_empty() {
        detected.push_str(&format!(" It looks like it needs {}.", services.join(", ")));
    }
    let name = folder
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    Entry {
        request: OpenRequest {
            harness: harness.clone(),
            project: ProjectId::for_folder(folder, engine),
            project_name: name,
            folder: folder.to_path_buf(),
        },
        context,
        prompt: format!(
            "Generate the Docker setup for this folder: a Dockerfile, compose.yaml, .dockerignore and .env lines as needed, with healthchecks and host ports in 56000-56999.{detected} Propose every file with files.propose_edit so I can review them before anything is written."
        ),
        origin: "From Generate \u{b7} New Docker setup".to_string(),
    }
}

/// Open the entry's session, attach its context and send its first
/// turn.
///
/// # Errors
/// A sentence when the session cannot open or its harness is gone.
pub async fn run(service: &AssistantService, entry: Entry) -> Result<SessionId, String> {
    let id = service.open(entry.request).await?;
    if !entry.context.is_empty() {
        service.add_context(id, entry.context);
    }
    service.send(id, entry.prompt, Some(entry.origin))?;
    Ok(id)
}

/// Show a project's assistant: select it and open the bottom panel on
/// the Assistant tab.
pub fn show(state: &mut AppState, project: &ProjectId) {
    state.selection.set(Selection::Project(project.clone()));
    let mut bottom = state.bottom.write();
    bottom.open = true;
    bottom.tab = BottomTab::Assistant;
}

/// The harness a click uses when the entry point does not name one:
/// the first installed (the picker's own default order).
#[must_use]
pub fn first_available(harnesses: &[Harness]) -> Option<Harness> {
    harnesses.iter().find(|h| h.available).cloned()
}

/// Why an entry point cannot run, for its button's disabled reason:
/// checked in the order a person would fix them.
#[must_use]
pub fn disabled_reason(
    service: bool,
    harness: Option<&Harness>,
    project: Option<&Project>,
) -> Option<String> {
    if !service {
        return Some("The assistant is not connected".to_string());
    }
    if harness.is_none() {
        return Some("Needs a harness: install Claude Code or muse".to_string());
    }
    match project {
        None => Some("Ask needs a project".to_string()),
        Some(project) if project.folder.is_none() => {
            Some("This project has no folder on this Mac".to_string())
        }
        Some(_) => None,
    }
}

#[cfg(test)]
#[path = "assistant_entry_tests.rs"]
mod tests;
