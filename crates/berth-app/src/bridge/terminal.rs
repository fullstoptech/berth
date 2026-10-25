//! Terminal messages (PLAN 7.5, D41): `berth.term.*` out, `term.*` back.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{base64_bytes, base64_encode, by_id, Call, Theme};

/// The DOM id the terminal bundle mounts into. Render a `div` with this id
/// and class `berth-term` before or right after sending `Create`.
pub fn host_id(id: &str) -> String {
    format!("berth-term-{id}")
}

/// Scrollback lines kept by every terminal (PLAN 7.5).
pub const SCROLLBACK: u32 = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermOptions {
    pub font_size: f32,
    pub theme: Theme,
    pub scrollback: u32,
}

impl TermOptions {
    pub fn new(font_size: f32, theme: Theme) -> Self {
        Self {
            font_size,
            theme,
            scrollback: SCROLLBACK,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TermCommand {
    Create {
        id: String,
        options: TermOptions,
    },
    /// Raw bytes from the session; sent as base64.
    Write {
        id: String,
        data: Vec<u8>,
    },
    Clear {
        id: String,
    },
    Focus {
        id: String,
    },
    /// Copies the selection to the clipboard (`berth.ui.copy`).
    CopySelection {
        id: String,
    },
    SetTheme(Theme),
    Dispose {
        id: String,
    },
}

impl TermCommand {
    pub(super) fn call(&self) -> Call {
        match self {
            TermCommand::Create { id, options } => Call {
                name: "term.create",
                args: vec![json!(id), json!(options)],
            },
            TermCommand::Write { id, data } => Call {
                name: "term.write",
                args: vec![json!(id), json!(base64_encode(data))],
            },
            TermCommand::Clear { id } => by_id("term.clear", id),
            TermCommand::Focus { id } => by_id("term.focus", id),
            TermCommand::CopySelection { id } => by_id("term.copySelection", id),
            TermCommand::SetTheme(theme) => Call {
                name: "term.setTheme",
                args: vec![json!(theme)],
            },
            TermCommand::Dispose { id } => by_id("term.dispose", id),
        }
    }
}

/// JS to Rust. `Input` carries the decoded bytes the user typed.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type")]
pub enum TermEvent {
    /// The terminal is mounted and has reported its first size.
    #[serde(rename = "term.ready")]
    Ready { id: String },
    #[serde(rename = "term.input")]
    Input {
        id: String,
        #[serde(deserialize_with = "base64_bytes")]
        data: Vec<u8>,
    },
    #[serde(rename = "term.resize")]
    Resize { id: String, cols: u16, rows: u16 },
    #[serde(rename = "term.title")]
    Title { id: String, title: String },
}

impl TermEvent {
    pub fn id(&self) -> &str {
        match self {
            TermEvent::Ready { id }
            | TermEvent::Input { id, .. }
            | TermEvent::Resize { id, .. }
            | TermEvent::Title { id, .. } => id,
        }
    }
}
