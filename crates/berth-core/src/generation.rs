use serde::{Deserialize, Serialize};

use crate::compose::ComposeCommand;
use crate::effect::{EffectKind, EffectLine, EffectSummary};
use crate::ids::HarnessId;
use crate::problem::ProblemKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChange {
    Create,
    Modify,
    Delete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffLineKind {
    Added,
    Removed,
    Context,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    pub lines: Vec<DiffLine>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: std::path::PathBuf,
    pub change: FileChange,
    pub old_sha256: Option<String>,
    pub new_text: Option<String>,
    pub hunks: Vec<DiffHunk>,
    pub added: u32,
    pub removed: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlanSource {
    Recipe(String),
    Template(String),
    Rule(ProblemKind),
    Harness(HarnessId),
    Editor,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationPlan {
    pub root: std::path::PathBuf,
    pub files: Vec<FileDiff>,
    pub source: PlanSource,
    pub notes: Vec<String>,
    pub after_write: Option<ComposeCommand>,
}
impl GenerationPlan {
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn effect(&self) -> EffectSummary {
        let mut lines: Vec<EffectLine> = self
            .files
            .iter()
            .map(|f| {
                let (kind, word) = match f.change {
                    FileChange::Create => (EffectKind::Writes, "Adds"),
                    FileChange::Modify => (EffectKind::Changes, "Changes"),
                    FileChange::Delete => (EffectKind::Goes, "Removes"),
                };
                EffectLine {
                    kind,
                    text: format!("{word} {}", f.path.display()),
                }
            })
            .collect();
        lines.push(EffectLine {
            kind: EffectKind::Stays,
            text: "Writes nothing outside this folder.".to_string(),
        });
        EffectSummary {
            verb: "Write".to_string(),
            noun_one: "file".to_string(),
            noun_many: "files".to_string(),
            count: self.files.len() as u32,
            bytes: None,
            lines,
            typed_name: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FoundKind {
    Compose,
    Dockerfile { from: Option<String> },
    Env { keys: u32 },
    EnvExample,
    DockerIgnore,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundFile {
    pub kind: FoundKind,
    pub path: std::path::PathBuf,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StackGuess {
    Node {
        pm: String,
        framework: Option<String>,
        version: Option<String>,
    },
    Python {
        tool: String,
        framework: Option<String>,
    },
    Go,
    Rust,
    Php {
        framework: Option<String>,
    },
    Ruby {
        framework: Option<String>,
    },
    Java {
        build: String,
        framework: Option<String>,
    },
    Static,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ServiceKind {
    Postgres,
    Mysql,
    Mongo,
    Redis,
    Rabbitmq,
    Meilisearch,
    Minio,
    Mailpit,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceGuess {
    pub kind: ServiceKind,
    pub reason: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detection {
    pub folder: std::path::PathBuf,
    pub found: Vec<FoundFile>,
    pub stack: Vec<StackGuess>,
    pub services_needed: Vec<ServiceGuess>,
    pub confidence: Confidence,
    pub git_branch: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> GenerationPlan {
        GenerationPlan {
            root: std::path::PathBuf::from("/Users/me/Dev/new-app"),
            files: vec![
                FileDiff {
                    path: std::path::PathBuf::from("compose.yaml"),
                    change: FileChange::Create,
                    old_sha256: None,
                    new_text: Some("services: {}".to_string()),
                    hunks: vec![],
                    added: 1,
                    removed: 0,
                },
                FileDiff {
                    path: std::path::PathBuf::from(".env"),
                    change: FileChange::Modify,
                    old_sha256: Some("abc".to_string()),
                    new_text: Some("A=1".to_string()),
                    hunks: vec![],
                    added: 1,
                    removed: 0,
                },
            ],
            source: PlanSource::Recipe("node".to_string()),
            notes: vec![],
            after_write: None,
        }
    }

    #[test]
    fn generation_plan_serde_round_trip() {
        let p = plan();
        let json = serde_json::to_string(&p).expect("serialize");
        assert_eq!(
            serde_json::from_str::<GenerationPlan>(&json).expect("deserialize"),
            p
        );
    }

    #[test]
    fn generation_effect_counts_files_and_stays_in_folder() {
        let e = plan().effect();
        assert_eq!(e.button_label(), "Write 2 files");
        assert_eq!(e.lines[0].text, "Adds compose.yaml");
        assert_eq!(e.lines[1].text, "Changes .env");
        assert_eq!(
            e.lines.last().map(|l| l.text.as_str()),
            Some("Writes nothing outside this folder.")
        );
    }
}
