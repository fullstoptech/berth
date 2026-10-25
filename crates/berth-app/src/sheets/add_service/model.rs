//! The pure part of the Add service sheet: the rows a template asks for, the checks on them, and the
//! plan built from the answers. The only I/O is [`read_project_files`] and the functions that call it
//! (they block; the sheet runs them off the UI thread).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use berth_core::{Container, GenerationPlan, Project, Template, TemplateInputKind, TemplateKind};
use berth_rules::RulesError;

use crate::services::plans::{PendingPlan, PlanRun};
use crate::services::ports::{port_owner, taken_ports};

/// The category of a template made from an image (Find images, "Add to project…").
pub const IMAGE_CATEGORY: &str = "Image";

/// A compose service name Berth is willing to write: the rule `templates::valid_name` applies.
#[must_use]
pub fn valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Whether `t` was made from an image reference rather than picked from the template list.
#[must_use]
pub fn is_image(t: &Template) -> bool {
    t.category == IMAGE_CATEGORY
}

/// A one-service template for an image: `image:` and a restart policy, no inputs. The reference is
/// checked because it is written into the YAML as it stands.
///
/// # Errors
/// A sentence when the reference has a character an image name does not.
pub fn image_template(reference: &str) -> Result<Template, String> {
    let reference = reference.trim();
    let first_ok = reference
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    let rest_ok = reference
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | ':' | '@'));
    if !first_ok || !rest_ok || reference.len() > 255 {
        return Err(format!(
            "\u{201c}{reference}\u{201d} is not an image name Berth can write into a compose file."
        ));
    }
    Ok(Template {
        id: reference.to_string(),
        name: reference.to_string(),
        kind: TemplateKind::Custom,
        service: format!("image: {reference}\nrestart: unless-stopped\n"),
        env: Vec::new(),
        inputs: Vec::new(),
        category: IMAGE_CATEGORY.to_string(),
        image: reference.to_string(),
        description: format!("Runs {reference} as a service of the project."),
    })
}

/// `postgres` from `docker.io/library/postgres:17-alpine@sha256:…`.
fn image_service_name(reference: &str) -> String {
    let last = reference.rsplit('/').next().unwrap_or(reference);
    let name = last.split(['@', ':']).next().unwrap_or(last);
    sanitize(name)
}

/// Lowercase letters and digits, with `-` between them.
fn sanitize(text: &str) -> String {
    let cleaned: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let cleaned = cleaned.trim_matches('-').to_string();
    if cleaned.is_empty() {
        "service".to_string()
    } else {
        cleaned
    }
}

/// The name a template's service gets before the user changes it: its id, else its name made into one.
#[must_use]
pub fn base_name(t: &Template) -> String {
    if is_image(t) {
        image_service_name(&t.image)
    } else if t.kind == TemplateKind::BuiltIn && valid_name(&t.id) {
        t.id.to_lowercase()
    } else {
        sanitize(&t.name)
    }
}

/// `base`, or `base-2`, `base-3`… when the project has a service of that name.
#[must_use]
pub fn unique_name(base: &str, existing: &[String]) -> String {
    if !existing.iter().any(|e| e == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !existing.contains(candidate))
        .unwrap_or_else(|| base.to_string())
}

/// What is wrong with the service name, or `None`. `existing` are the project's services as the sheet
/// knows them; `rejected` is a name the compose file turned out to have already (found when the plan
/// was built).
#[must_use]
pub fn name_error(name: &str, existing: &[String], rejected: Option<&str>) -> Option<String> {
    if name.is_empty() {
        return Some("Enter a name for the service".to_string());
    }
    if !valid_name(name) {
        return Some(
            "Use letters, digits, - _ and . and start with a letter or a digit".to_string(),
        );
    }
    if existing.iter().any(|e| e == name) || rejected == Some(name) {
        return Some(format!(
            "A service called {name} is already in this project. Pick another name."
        ));
    }
    None
}

/// The names of a project's services as the Projects list knows them.
#[must_use]
pub fn existing_services(project: &Project) -> Vec<String> {
    project.services.iter().map(|s| s.name.clone()).collect()
}

/// The projects a service can be added to: a folder on this Mac with a compose file. By name.
#[must_use]
pub fn eligible_projects(projects: &[Project]) -> Vec<Project> {
    let mut out: Vec<Project> = projects
        .iter()
        .filter(|p| p.folder.is_some() && !p.compose_files.is_empty())
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

// ---------------------------------------------------------------------
// Input rows
// ---------------------------------------------------------------------

/// The control a row draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    Port,
    Version {
        choices: Vec<String>,
    },
    Volume,
    /// Generated by Berth, shown masked, never typed.
    Password,
}

/// One input of the template as the sheet shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputRow {
    pub key: String,
    pub label: String,
    pub kind: RowKind,
    /// What the field starts with: a free port, the first version, the default volume name. Never the
    /// user's own text (an uncontrolled field's initial value must not change while it is typed in).
    pub initial: String,
    /// What the field holds now: the typed text, else `initial`.
    pub current: String,
    pub error: Option<String>,
    /// "5432 is used by pg-scratch" when a running container holds the port: a warning, not an error.
    pub warning: Option<String>,
}

/// The starting value of every input that has one: the free port nearest each default (ports
/// suggested for one template never repeat), the first version, the default volume name.
#[must_use]
pub fn suggested_values(t: &Template, taken: &[u16]) -> BTreeMap<String, String> {
    let mut taken = taken.to_vec();
    let mut out = BTreeMap::new();
    for input in &t.inputs {
        let value = match &input.kind {
            TemplateInputKind::Port { default } => {
                let port = berth_rules::ports::suggest(*default, &taken);
                taken.push(port);
                port.to_string()
            }
            TemplateInputKind::Version { choices } => choices.first().cloned().unwrap_or_default(),
            TemplateInputKind::VolumeName { default } => default.clone(),
            TemplateInputKind::Password => continue,
        };
        out.insert(input.key.clone(), value);
    }
    out
}

fn parse_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|p| *p >= 1)
}

/// The rows of `t`, with the typed `values` checked. `containers` are the ones on the engine the service
/// will run on: a port one of them publishes gets a warning that names it.
#[must_use]
pub fn input_rows(
    t: &Template,
    values: &BTreeMap<String, String>,
    taken: &[u16],
    containers: &[Container],
) -> Vec<InputRow> {
    let suggested = suggested_values(t, taken);
    let current_of = |key: &str| -> String {
        values
            .get(key)
            .cloned()
            .or_else(|| suggested.get(key).cloned())
            .unwrap_or_default()
    };
    let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
    for input in &t.inputs {
        if matches!(input.kind, TemplateInputKind::Port { .. }) {
            if let Some(p) = parse_port(&current_of(&input.key)) {
                *counts.entry(p).or_default() += 1;
            }
        }
    }
    t.inputs
        .iter()
        .map(|input| {
            let initial = suggested.get(&input.key).cloned().unwrap_or_default();
            let current = current_of(&input.key);
            let mut error = None;
            let mut warning = None;
            let kind = match &input.kind {
                TemplateInputKind::Port { .. } => {
                    match parse_port(&current) {
                        None => error = Some("Enter a port from 1 to 65535".to_string()),
                        Some(p) if counts.get(&p).copied().unwrap_or(0) > 1 => {
                            error = Some(format!("{p} is used twice here"));
                        }
                        Some(p) => warning = port_owner(p, containers),
                    }
                    RowKind::Port
                }
                TemplateInputKind::Version { choices } => RowKind::Version {
                    choices: choices.clone(),
                },
                TemplateInputKind::VolumeName { .. } => {
                    if !valid_name(&current) {
                        error = Some("Use letters, digits, - _ and .".to_string());
                    }
                    RowKind::Volume
                }
                TemplateInputKind::Password => RowKind::Password,
            };
            InputRow {
                key: input.key.clone(),
                label: input.label.clone(),
                kind,
                initial,
                current,
                error,
                warning,
            }
        })
        .collect()
}

/// The first reason the sheet cannot go on, or `None`.
#[must_use]
pub fn blocker(
    project: Option<&Project>,
    name_error: Option<&str>,
    rows: &[InputRow],
) -> Option<String> {
    if project.is_none() {
        return Some("Choose a project first".to_string());
    }
    name_error
        .map(str::to_string)
        .or_else(|| rows.iter().find_map(|r| r.error.clone()))
}

/// The values `add_service_plan` takes: what the user typed over the suggestions, and the generated
/// password for every password input.
#[must_use]
pub fn plan_inputs(
    t: &Template,
    values: &BTreeMap<String, String>,
    taken: &[u16],
    password: &str,
) -> BTreeMap<String, String> {
    let mut out = suggested_values(t, taken);
    out.extend(
        values
            .iter()
            .map(|(k, v)| (k.clone(), v.trim().to_string())),
    );
    for input in &t.inputs {
        if matches!(input.kind, TemplateInputKind::Password) {
            out.insert(input.key.clone(), password.to_string());
        }
    }
    out
}

/// The containers on `project`'s engine: the ones whose ports the new service must not take.
#[must_use]
pub fn engine_containers(project: &Project, containers: &[Container]) -> Vec<Container> {
    containers
        .iter()
        .filter(|c| c.engine == project.engine)
        .cloned()
        .collect()
}

/// The host ports the active containers on `project`'s engine hold.
#[must_use]
pub fn engine_taken_ports(project: &Project, containers: &[Container]) -> Vec<u16> {
    taken_ports(&engine_containers(project, containers))
}

// ---------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------

/// Why no plan came out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddError {
    /// The compose file already has a service of that name: the user picks another.
    NameTaken(String),
    /// Anything else, in a sentence.
    Failed(String),
}

impl AddError {
    fn of(err: RulesError) -> Self {
        match err {
            RulesError::ServiceExists(name) => Self::NameTaken(name),
            other => Self::Failed(other.to_string()),
        }
    }
}

/// The compose file and `.env` of a project as they are on disk now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFiles {
    pub folder: PathBuf,
    /// The compose file, relative to the folder.
    pub compose_rel: PathBuf,
    pub compose_text: String,
    pub env_text: Option<String>,
}

/// Reads the first compose file of `project` that exists, and its `.env`. Blocking.
///
/// # Errors
/// A sentence: no folder on this Mac, no compose file there, or a file that cannot be read.
pub fn read_project_files(project: &Project) -> Result<ProjectFiles, String> {
    let folder = project
        .folder
        .clone()
        .ok_or_else(|| "This project has no folder on this Mac.".to_string())?;
    let file = project
        .compose_files
        .iter()
        .find(|f| folder.join(f).is_file())
        .ok_or_else(|| "Berth cannot find this project's compose file on this Mac.".to_string())?;
    let absolute = folder.join(file);
    let compose_text = std::fs::read_to_string(&absolute)
        .map_err(|e| format!("Cannot read {}: {e}", absolute.display()))?;
    let compose_rel = absolute.strip_prefix(&folder).map_or_else(
        |_| PathBuf::from(file.file_name().unwrap_or_default()),
        PathBuf::from,
    );
    let env_path = folder.join(".env");
    let env_text = match std::fs::read_to_string(&env_path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("Cannot read {}: {e}", env_path.display())),
    };
    Ok(ProjectFiles {
        folder,
        compose_rel,
        compose_text,
        env_text,
    })
}

/// The plan that adds `t` as `name` to `project`, from the files as they are now. Blocking.
///
/// # Errors
/// [`AddError::NameTaken`] when the compose file has a service called `name`.
pub fn build_plan(
    project: &Project,
    t: &Template,
    name: &str,
    inputs: &BTreeMap<String, String>,
) -> Result<GenerationPlan, AddError> {
    let files = read_project_files(project).map_err(AddError::Failed)?;
    berth_rules::templates::add_service_plan(
        &files.folder,
        &files.compose_rel,
        &files.compose_text,
        files.env_text.as_deref(),
        t,
        name,
        inputs,
    )
    .map_err(AddError::of)
}

/// [`build_plan`] parked for Review changes: it runs `up` for the new service afterwards, and builds
/// the plan again when a file changes before Write. Blocking.
///
/// # Errors
/// As [`build_plan`].
pub fn pending_plan(
    project: &Project,
    t: &Template,
    name: &str,
    inputs: &BTreeMap<String, String>,
) -> Result<PendingPlan, AddError> {
    let plan = build_plan(project, t, name, inputs)?;
    let mut pending = PendingPlan::new(plan, project.name.clone());
    pending.run = PlanRun::for_project(project);
    let (project, t, name, inputs) = (project.clone(), t.clone(), name.to_string(), inputs.clone());
    pending.regenerate = Some(Arc::new(move || {
        build_plan(&project, &t, &name, &inputs).map_err(|e| match e {
            AddError::NameTaken(n) => RulesError::ServiceExists(n),
            AddError::Failed(m) => RulesError::Read(project.folder.clone().unwrap_or_default(), m),
        })
    }));
    Ok(pending)
}
