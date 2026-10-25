//! The pure part of Review changes: what the sheet says about a plan and the words on the footer
//! buttons. The masking of `.env` files lives with the diff view that draws them
//! (`ui::diff_view`). No signals, no I/O.

use std::collections::BTreeSet;

use berth_core::{ComposeCommand, FileChange, GenerationPlan, HarnessId, PlanSource};

use crate::ui::diff_view::{file_view, is_secret_file, FileView};

/// Every file of the plan, in plan order. `revealed` names the secret files shown unmasked.
#[must_use]
pub fn file_views(plan: &GenerationPlan, revealed: &BTreeSet<String>) -> Vec<FileView> {
    plan.files
        .iter()
        .map(|f| file_view(f, revealed.contains(&f.path.display().to_string())))
        .collect()
}

// ---------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "Adds 3 files and 2 keys to .env in ~/Dev/side/new-app. Writes nothing outside this folder." The
/// counts come from the plan; the last sentence is the plan's own `effect` line. A secret file
/// that only gains lines counts as keys, not as a changed file.
#[must_use]
pub fn summary(plan: &GenerationPlan, folder: &str) -> String {
    let mut adds = 0;
    let mut changes = 0;
    let mut removes = 0;
    let mut keys = 0;
    let mut keys_file = String::new();
    for f in &plan.files {
        match f.change {
            FileChange::Create => adds += 1,
            FileChange::Delete => removes += 1,
            FileChange::Modify if is_secret_file(&f.path) && f.removed == 0 => {
                keys += f.added as usize;
                keys_file = f.path.display().to_string();
            }
            FileChange::Modify => changes += 1,
        }
    }
    let mut clauses: Vec<String> = Vec::new();
    let mut added: Vec<String> = Vec::new();
    if adds > 0 {
        added.push(plural(adds, "file", "files"));
    }
    if keys > 0 {
        added.push(format!("{} to {keys_file}", plural(keys, "key", "keys")));
    }
    if !added.is_empty() {
        clauses.push(format!("adds {}", added.join(" and ")));
    }
    if changes > 0 {
        clauses.push(format!("changes {}", plural(changes, "file", "files")));
    }
    if removes > 0 {
        clauses.push(format!("removes {}", plural(removes, "file", "files")));
    }
    let stays = plan
        .effect()
        .lines
        .iter()
        .find(|l| l.kind == berth_core::EffectKind::Stays)
        .map(|l| l.text.clone())
        .unwrap_or_default();
    if clauses.is_empty() {
        return format!("Nothing to write in {folder}. {stays}");
    }
    let sentence = match clauses.len() {
        1 => clauses.remove(0),
        _ => {
            let last = clauses.pop().unwrap_or_default();
            format!("{} and {last}", clauses.join(", "))
        }
    };
    let mut chars = sentence.chars();
    let sentence = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect::<String>())
        .unwrap_or_default();
    format!("{sentence} in {folder}. {stays}")
}

/// The small line under the summary: where the plan comes from.
#[must_use]
pub fn source_line(source: &PlanSource, harness_name: impl Fn(&HarnessId) -> String) -> String {
    match source {
        PlanSource::Recipe(label) => format!("From the {label} recipe."),
        PlanSource::Template(name) => format!("From the {name} template."),
        PlanSource::Rule(kind) => format!("A fix for {}.", rule_name(kind)),
        PlanSource::Harness(id) => format!("Proposed by {}.", harness_name(id)),
        PlanSource::Editor => "From the editor.".to_string(),
    }
}

fn rule_name(kind: &berth_core::ProblemKind) -> String {
    let debug = format!("{kind:?}");
    let name = debug.split([' ', '{', '(']).next().unwrap_or_default();
    // "PortAllocated" -> "port allocated".
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// What Compose printed when a start failed, as a person reads it. `docker compose --progress json`
/// writes one JSON object per line on stderr (`{"level":"warning","msg":...}`,
/// `{"error":true,"message":...}`): the errors' messages are kept and the warnings dropped; text
/// that is not JSON stays as it is.
#[must_use]
pub fn compose_failure_text(raw: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(serde_json::Value::Object(map)) => {
                let is_error = map.get("error").and_then(serde_json::Value::as_bool) == Some(true)
                    || map.get("level").and_then(serde_json::Value::as_str) == Some("error");
                let text = map
                    .get("message")
                    .or_else(|| map.get("msg"))
                    .and_then(serde_json::Value::as_str);
                if let (true, Some(text)) = (is_error, text) {
                    kept.push(text.to_string());
                }
            }
            _ => kept.push(line.to_string()),
        }
    }
    if kept.is_empty() {
        raw.trim().to_string()
    } else {
        kept.join("\n")
    }
}

/// "start", "recreate db, worker", "restart": the words after "Write N files and" for the command
/// `after_write` carries, or `None` for a plan that only writes.
#[must_use]
pub fn after_write_words(command: Option<&ComposeCommand>) -> Option<String> {
    let list = |services: &[String]| services.join(", ");
    Some(match command? {
        ComposeCommand::Up { services, .. } if services.is_empty() => "start".to_string(),
        ComposeCommand::Up { services, .. } => format!("recreate {}", list(services)),
        ComposeCommand::Start { services } if services.is_empty() => "start".to_string(),
        ComposeCommand::Start { services } => format!("start {}", list(services)),
        ComposeCommand::Restart { services } if services.is_empty() => "restart".to_string(),
        ComposeCommand::Restart { services } => format!("restart {}", list(services)),
        ComposeCommand::Stop { services } if services.is_empty() => "stop".to_string(),
        ComposeCommand::Stop { services } => format!("stop {}", list(services)),
        ComposeCommand::Build { services, .. } if services.is_empty() => "build".to_string(),
        ComposeCommand::Build { services, .. } => format!("build {}", list(services)),
        ComposeCommand::Pull { services } if services.is_empty() => "pull images".to_string(),
        ComposeCommand::Pull { services } => format!("pull {}", list(services)),
        ComposeCommand::Down { .. } => "take the project down".to_string(),
    })
}

/// The footer's buttons, from the plan alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterLabels {
    /// "Write files only": shown only when there is a second button.
    pub write_only: String,
    /// The primary button: "Write 4 files and start", or "Write 2 files" for a plan that runs nothing.
    pub primary: String,
    /// The primary button also runs a command.
    pub runs: bool,
}

#[must_use]
pub fn footer_labels(plan: &GenerationPlan) -> FooterLabels {
    let count = plan.files.len();
    let write = format!("Write {}", plural(count, "file", "files"));
    match after_write_words(plan.after_write.as_ref()) {
        Some(words) => FooterLabels {
            write_only: "Write files only".to_string(),
            primary: format!("{write} and {words}"),
            runs: true,
        },
        None => FooterLabels {
            write_only: "Write files only".to_string(),
            primary: write,
            runs: false,
        },
    }
}
