//! Rules: folder detection, generation recipes, templates, text edits that keep
//! comments, diffs, guarded writes, port suggestions, `docker run` conversion and
//! problem rules. PLAN section 6.7.

pub mod compose_edit;
pub mod detect;
pub mod diff;
pub mod ports;
pub mod problems;
pub mod recipes;
pub mod run_to_compose;
pub mod templates;
pub mod write;

#[derive(Debug, thiserror::Error)]
pub enum RulesError {
    #[error("{0} is outside the project folder")]
    OutsideRoot(std::path::PathBuf),
    #[error("{0} changed since the plan was made")]
    Conflict(std::path::PathBuf),
    #[error("compose file is not valid YAML at line {line}: {message}")]
    Yaml { line: usize, message: String },
    #[error("cannot read {0}: {1}")]
    Read(std::path::PathBuf, String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
