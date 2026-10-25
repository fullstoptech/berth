//! The pure part of the Generate sheet: what the user may choose, how the choices become
//! `RecipeOptions`, and how a plan is built from them. No signals, no I/O besides `build_plan`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use berth_core::{
    Container, Detection, FoundKind, GenerationPlan, ServiceGuess, ServiceKind, StackGuess,
};
use berth_rules::recipes::{self, RecipeOptions};
use berth_rules::RulesError;

use crate::services::ports::port_owner;

/// The databases the sheet offers (one at a time); every other service is a checkbox.
pub const DATABASES: [ServiceKind; 3] = [
    ServiceKind::Postgres,
    ServiceKind::Mysql,
    ServiceKind::Mongo,
];

/// The services that are a checkbox each.
pub const EXTRAS: [ServiceKind; 5] = [
    ServiceKind::Redis,
    ServiceKind::Rabbitmq,
    ServiceKind::Meilisearch,
    ServiceKind::Minio,
    ServiceKind::Mailpit,
];

/// The name the sheet gives a service.
#[must_use]
pub fn kind_name(kind: ServiceKind) -> &'static str {
    match kind {
        ServiceKind::Postgres => "PostgreSQL",
        ServiceKind::Mysql => "MySQL",
        ServiceKind::Mongo => "MongoDB",
        ServiceKind::Redis => "Redis",
        ServiceKind::Rabbitmq => "RabbitMQ",
        ServiceKind::Meilisearch => "Meilisearch",
        ServiceKind::Minio => "S3 storage",
        ServiceKind::Mailpit => "Mailpit",
    }
}

/// What the user has picked: the database, the extra services, the ports as typed and the Node
/// version as typed. Ports are kept as text so a half-typed number is not lost.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Choices {
    pub database: Option<ServiceKind>,
    pub extras: Vec<ServiceKind>,
    /// Port overrides by `RecipeOptions::ports` key; a key not here uses the suggested port.
    pub ports: BTreeMap<String, String>,
    pub node_version: Option<String>,
}

impl Choices {
    /// The choices a detection implies: its first database, its other services, the Node version
    /// the recipe would use.
    #[must_use]
    pub fn from_detection(d: &Detection, taken: &[u16]) -> Self {
        let database = d
            .services_needed
            .iter()
            .map(|s| s.kind)
            .find(|k| DATABASES.contains(k));
        let extras = d
            .services_needed
            .iter()
            .map(|s| s.kind)
            .filter(|k| EXTRAS.contains(k))
            .collect::<Vec<_>>();
        let extras = EXTRAS.into_iter().filter(|k| extras.contains(k)).collect();
        let node_version = recipes::options_for(d, taken).node_version;
        Self {
            database,
            extras,
            ports: BTreeMap::new(),
            node_version,
        }
    }

    /// Pick the database (or none). The ports start over from the suggestions: the fields are
    /// drawn again for the new set of services.
    pub fn set_database(&mut self, kind: Option<ServiceKind>) {
        self.database = kind;
        self.ports.clear();
    }

    /// Tick or untick an extra service; the list keeps the sheet's order. The ports start over
    /// from the suggestions, as for [`Self::set_database`].
    pub fn set_extra(&mut self, kind: ServiceKind, on: bool) {
        self.ports.clear();
        self.extras.retain(|k| *k != kind);
        if on {
            self.extras.push(kind);
            self.extras
                .sort_by_key(|k| EXTRAS.iter().position(|e| e == k));
        }
    }

    /// The services chosen, databases first.
    #[must_use]
    pub fn services(&self) -> Vec<ServiceKind> {
        self.database
            .into_iter()
            .chain(self.extras.iter().copied())
            .collect()
    }
}

/// `d` with its services replaced by the chosen ones (a service the detection found keeps its
/// reason), so `options_for` offers a port for each.
#[must_use]
pub fn with_choices(d: &Detection, choices: &Choices) -> Detection {
    let mut out = d.clone();
    out.services_needed = choices
        .services()
        .into_iter()
        .map(|kind| {
            d.services_needed
                .iter()
                .find(|s| s.kind == kind)
                .cloned()
                .unwrap_or_else(|| ServiceGuess {
                    kind,
                    reason: "chosen here".to_string(),
                })
        })
        .collect();
    out
}

/// One port the sheet asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortRow {
    /// The `RecipeOptions::ports` key (`app`, `db`, `rabbitmq.management_port`).
    pub key: String,
    pub label: String,
    /// What the field starts with: the free port suggested. Never the user's own text: an
    /// uncontrolled field's initial value must not change while it is typed in.
    pub value: String,
    /// "Not a port number" when the text is not 1 to 65535, else nothing.
    pub error: Option<String>,
    /// "5432 is used by pg-scratch" when a running container holds it; a warning, not an error.
    pub warning: Option<String>,
}

fn label_for(key: &str, choices: &Choices) -> String {
    let (service, input) = key
        .split_once('.')
        .map_or((key, None), |(s, i)| (s, Some(i)));
    let base = match service {
        "app" => "App".to_string(),
        "db" => choices
            .database
            .map_or_else(|| "Database".to_string(), |k| kind_name(k).to_string()),
        "mongo" => kind_name(ServiceKind::Mongo).to_string(),
        "redis" => kind_name(ServiceKind::Redis).to_string(),
        "rabbitmq" => kind_name(ServiceKind::Rabbitmq).to_string(),
        "meilisearch" => kind_name(ServiceKind::Meilisearch).to_string(),
        "s3" => kind_name(ServiceKind::Minio).to_string(),
        "mailpit" => kind_name(ServiceKind::Mailpit).to_string(),
        other => other.to_string(),
    };
    match input {
        Some(input) => {
            let input = input.trim_end_matches("_port").replace('_', " ");
            format!("{base} {input} port")
        }
        None => format!("{base} port"),
    }
}

fn parse_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|p| *p >= 1)
}

/// The ports to ask for, in the order `RecipeOptions` lists them (app first).
#[must_use]
pub fn port_rows(
    d: &Detection,
    choices: &Choices,
    taken: &[u16],
    containers: &[Container],
) -> Vec<PortRow> {
    let suggested = recipes::options_for(&with_choices(d, choices), taken);
    let mut keys: Vec<&String> = suggested.ports.keys().collect();
    keys.sort_by_key(|k| (k.as_str() != "app", (*k).clone()));
    let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
    let parsed: BTreeMap<&String, Option<u16>> = keys
        .iter()
        .map(|k| {
            let value = choices.ports.get(*k).map_or_else(
                || suggested.ports.get(*k).map(u16::to_string),
                |t| Some(t.clone()),
            );
            (*k, value.as_deref().and_then(parse_port))
        })
        .collect();
    for port in parsed.values().flatten() {
        *counts.entry(*port).or_default() += 1;
    }
    keys.into_iter()
        .map(|key| {
            let value = suggested
                .ports
                .get(key)
                .map(u16::to_string)
                .unwrap_or_default();
            let port = parsed.get(key).copied().flatten();
            let error = match port {
                None => Some("Enter a port from 1 to 65535".to_string()),
                Some(p) if counts.get(&p).copied().unwrap_or(0) > 1 => {
                    Some(format!("{p} is used twice here"))
                }
                Some(_) => None,
            };
            let warning = port.and_then(|p| port_owner(p, containers));
            PortRow {
                label: label_for(key, choices),
                key: key.clone(),
                value,
                error,
                warning,
            }
        })
        .collect()
}

/// The options the chosen values make, or the first reason they cannot.
///
/// # Errors
/// A sentence naming the port or version that is wrong.
pub fn options(d: &Detection, choices: &Choices, taken: &[u16]) -> Result<RecipeOptions, String> {
    let mut options = recipes::options_for(&with_choices(d, choices), taken);
    for (key, text) in &choices.ports {
        if !options.ports.contains_key(key) {
            continue;
        }
        let port = parse_port(text).ok_or_else(|| format!("{text} is not a port number"))?;
        options.ports.insert(key.clone(), port);
    }
    let mut seen = BTreeSet::new();
    for port in options.ports.values() {
        if !seen.insert(*port) {
            return Err(format!("Port {port} is used twice"));
        }
    }
    if let Some(version) = &choices.node_version {
        let version = version.trim();
        if options.node_version.is_some() {
            if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
                return Err(format!("{version} is not a Node version"));
            }
            options.node_version = Some(version.to_string());
        }
    }
    Ok(options)
}

/// "Next.js 15 · pnpm · Node 22": the first stack the detection found.
#[must_use]
pub fn stack_line(d: &Detection) -> String {
    let Some(stack) = d.stack.first() else {
        return if d
            .found
            .iter()
            .any(|f| matches!(f.kind, FoundKind::Dockerfile { .. }))
        {
            "A Dockerfile".to_string()
        } else {
            "No known stack".to_string()
        };
    };
    match stack {
        StackGuess::Node {
            pm,
            framework,
            framework_version,
            version,
        } => {
            let mut parts = vec![match (framework, framework_version) {
                (Some(f), Some(v)) => format!("{f} {v}"),
                (Some(f), None) => f.clone(),
                (None, _) => "Node".to_string(),
            }];
            parts.push(pm.clone());
            if let Some(v) = version {
                parts.push(format!("Node {v}"));
            }
            parts.join(" \u{b7} ")
        }
        StackGuess::Python { tool, framework } => join_with(framework.as_deref(), "Python", tool),
        StackGuess::Go => "Go".to_string(),
        StackGuess::Rust => "Rust".to_string(),
        StackGuess::Php { framework } => framework.clone().unwrap_or_else(|| "PHP".to_string()),
        StackGuess::Ruby { framework } => framework.clone().unwrap_or_else(|| "Ruby".to_string()),
        StackGuess::Java { build, framework } => join_with(framework.as_deref(), "Java", build),
        StackGuess::Static => "Static site".to_string(),
    }
}

fn join_with(framework: Option<&str>, language: &str, tool: &str) -> String {
    format!("{} \u{b7} {tool}", framework.unwrap_or(language))
}

/// Whether the folder already has a Compose file (Generate then has nothing to add).
#[must_use]
pub fn has_compose(d: &Detection) -> bool {
    d.found.iter().any(|f| matches!(f.kind, FoundKind::Compose))
}

/// The plan for `d` with `options`: a compose file for a folder that has a Dockerfile, the full
/// recipe otherwise. Reads the folder.
///
/// # Errors
/// What the rules say (no recognised stack, an unreadable file).
pub fn build_plan(d: &Detection, options: &RecipeOptions) -> Result<GenerationPlan, RulesError> {
    let has_dockerfile = d
        .found
        .iter()
        .any(|f| matches!(f.kind, FoundKind::Dockerfile { .. }));
    if has_dockerfile {
        recipes::compose_for_dockerfile(d, options)
    } else {
        recipes::plan(d, options)
    }
}

/// Detect the folder again and build the plan, for Regenerate.
///
/// # Errors
/// What `detect` or the rules say.
pub fn replan(
    folder: &Path,
    choices: &Choices,
    taken: &[u16],
) -> Result<GenerationPlan, RulesError> {
    let d = berth_rules::detect::detect(folder)?;
    let options =
        options(&d, choices, taken).map_err(|m| RulesError::Read(folder.to_path_buf(), m))?;
    build_plan(&d, &options)
}
