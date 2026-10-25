//! Editor messages (PLAN 7.5): `berth.editor.*` out, `editor.*` back.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{by_id, Call, Theme};

/// The DOM id the editor bundle mounts into. Render a `div` with this id and
/// class `berth-editor` before or right after sending `Create`.
pub fn host_id(id: &str) -> String {
    format!("berth-editor-{id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Yaml,
    Dockerfile,
    Text,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorOptions {
    pub lang: Lang,
    pub value: String,
    pub read_only: bool,
    pub theme: Theme,
}

/// A problem on one line (1-based), drawn as a gutter mark and underline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Mark {
    pub line: u32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditorCommand {
    Create {
        id: String,
        options: EditorOptions,
    },
    /// Replaces the whole text without echoing an `editor.change`.
    SetValue {
        id: String,
        text: String,
    },
    /// Answered with `EditorEvent::Value`.
    GetValue {
        id: String,
    },
    SetMarks {
        id: String,
        marks: Vec<Mark>,
    },
    SetTheme(Theme),
    Dispose {
        id: String,
    },
}

impl EditorCommand {
    pub(super) fn call(&self) -> Call {
        match self {
            EditorCommand::Create { id, options } => Call {
                name: "editor.create",
                args: vec![json!(id), json!(options)],
            },
            EditorCommand::SetValue { id, text } => Call {
                name: "editor.setValue",
                args: vec![json!(id), json!(text)],
            },
            EditorCommand::GetValue { id } => by_id("editor.getValue", id),
            EditorCommand::SetMarks { id, marks } => Call {
                name: "editor.setMarks",
                args: vec![json!(id), json!(marks)],
            },
            EditorCommand::SetTheme(theme) => Call {
                name: "editor.setTheme",
                args: vec![json!(theme)],
            },
            EditorCommand::Dispose { id } => by_id("editor.dispose", id),
        }
    }
}

/// JS to Rust. The text of `Change` and `Value` may hold secrets (an `.env`
/// open in the editor): never log it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type")]
pub enum EditorEvent {
    #[serde(rename = "editor.ready")]
    Ready { id: String },
    /// The user edited; debounced 150 ms in the bundle.
    #[serde(rename = "editor.change")]
    Change { id: String, text: String },
    /// The answer to `GetValue`.
    #[serde(rename = "editor.value")]
    Value { id: String, text: String },
    /// Cmd+S inside the editor.
    #[serde(rename = "editor.save")]
    Save { id: String },
}

impl EditorEvent {
    pub fn id(&self) -> &str {
        match self {
            EditorEvent::Ready { id }
            | EditorEvent::Change { id, .. }
            | EditorEvent::Value { id, .. }
            | EditorEvent::Save { id } => id,
        }
    }
}
