//! The assistant: harness detection, the tool catalog, redaction, and (in later
//! tasks) the MCP server, the bridge and the Claude Code and muse adapters
//! (PLAN §6.9, ADR 0005, ADR 0006).
//!
//! Every module of §6.9 is declared here so that the adapter tasks fill in
//! their own files and never edit this one.

pub mod bridge;
pub mod catalog;
pub mod claude;
pub mod context;
mod detect;
pub mod mcp;
pub mod muse;
pub mod redact;

pub use catalog::{catalog, tool, ToolDef};
pub use detect::detect;
pub use mcp::serve_mcp_stdio;

use berth_core::{ContextItem, Harness, HarnessTransport, SessionId};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum AssistantError {
    #[error("{0} is not installed")]
    NotInstalled(String),
    #[error("{harness} stopped: {stderr_tail}")]
    Exited {
        harness: String,
        stderr_tail: String,
    },
    #[error("bridge: {0}")]
    Bridge(String),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("cancelled")]
    Cancelled,
    #[error(transparent)]
    Sys(#[from] berth_sys::SysError),
}

/// How a harness reaches Berth's tools: the `berth mcp` child it launches.
#[derive(Debug, Clone)]
pub struct McpLaunch {
    pub berth_binary: std::path::PathBuf,
    pub bridge: std::path::PathBuf,
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub harness: Harness,
    pub session: SessionId,
    pub workspace: std::path::PathBuf,
    pub resume: Option<String>,
    pub mcp: McpLaunch,
    pub path: std::ffi::OsString,
    pub system_prompt: String,
    /// The model the user picked for this harness (AD39); `None` lets
    /// the harness decide. The `BERTH_*_MODEL` env vars remain as a
    /// fallback for the smoke tests.
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UserInput {
    pub text: String,
    pub context: Vec<ContextItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssistantEvent {
    Ready {
        harness_session: String,
    },
    TextDelta {
        text: String,
    },
    TextDone,
    ToolStarted {
        call_id: String,
        tool: String,
        args: serde_json::Value,
    },
    ToolFinished {
        call_id: String,
        ok: bool,
        ms: u64,
    },
    TurnDone {
        stop_reason: Option<String>,
    },
    Error {
        message: String,
        stderr_tail: String,
    },
}

#[async_trait::async_trait]
pub trait HarnessSession: Send {
    async fn send(&mut self, input: UserInput) -> Result<(), AssistantError>;
    /// Stops the running turn (⌘., D77).
    async fn interrupt(&mut self) -> Result<(), AssistantError>;
    async fn close(self: Box<Self>);
}

/// One model a harness offers, as the harness itself reported it
/// (AD39/AD40): `id` is the original model id to pass back, `label`
/// its display name when the catalog carries one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    pub id: String,
    pub label: String,
    pub is_default: bool,
}

/// Asks a harness which models it offers right now — Claude Code's
/// `list_models` control request, muse's MSP `model/list` query
/// (AD40). Nothing is hardcoded: a harness update changes the answer.
/// A query only; no model turn runs.
///
/// # Errors
/// The binary cannot be spawned, the harness does not answer in time,
/// or it reports an error.
pub async fn list_models(
    harness: &Harness,
    path: std::ffi::OsString,
    cancel: CancellationToken,
) -> Result<Vec<ModelChoice>, AssistantError> {
    match harness.transport {
        HarnessTransport::ClaudeStreamJson => claude::list_models(harness, path, cancel).await,
        HarnessTransport::MuseMsp => muse::list_models(harness, path, cancel).await,
    }
}

/// A started session and the stream of events it produces.
pub type Started = (Box<dyn HarnessSession>, mpsc::Receiver<AssistantEvent>);

/// Starts the session for `cfg.harness`, dispatching on its transport.
///
/// # Errors
/// Whatever the adapter reports: the binary is missing, the process exits
/// before it is ready, or the harness speaks something unexpected.
pub async fn start(
    cfg: SessionConfig,
    cancel: CancellationToken,
) -> Result<Started, AssistantError> {
    match cfg.harness.transport {
        HarnessTransport::ClaudeStreamJson => claude::start(cfg, cancel).await,
        HarnessTransport::MuseMsp => muse::start(cfg, cancel).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use berth_core::HarnessId;

    fn config(transport: HarnessTransport) -> SessionConfig {
        SessionConfig {
            harness: Harness {
                id: HarnessId("test".into()),
                name: "Test".into(),
                binary: "test".into(),
                version: None,
                transport,
                available: true,
            },
            session: SessionId(uuid::Uuid::nil()),
            workspace: std::env::temp_dir(),
            resume: None,
            mcp: McpLaunch {
                berth_binary: "berth".into(),
                bridge: "bridge.sock".into(),
                token: "t".into(),
            },
            path: std::ffi::OsString::new(),
            system_prompt: String::new(),
            model: None,
        }
    }

    #[tokio::test]
    async fn start_routes_each_transport_to_its_own_adapter() {
        for (transport, name) in [
            (HarnessTransport::ClaudeStreamJson, "claude"),
            (HarnessTransport::MuseMsp, "muse"),
        ] {
            let Err(err) = start(config(transport), CancellationToken::new()).await else {
                panic!("the adapter stubs must not start a session");
            };
            assert!(
                err.to_string().contains(name),
                "{transport:?} reached the wrong adapter: {err}"
            );
        }
    }
}
