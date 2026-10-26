//! Host side of the plugin protocol (PLAN §6.10, ADR 0008): discovery of
//! installed plugins, process supervision with restart backoff, and
//! dispatch of the requests a plugin may make of its host.
//!
//! Plugins are separate processes speaking JSON-RPC 2.0, one JSON object
//! per line on stdio (`berth-plugin-protocol`). A plugin receives a secret
//! value only through `host/secret.get`, and only for a key its manifest
//! declares *and* the user granted (see [`dispatch::secret_allowed`]).

pub mod discover;
pub mod dispatch;
pub mod process;

pub use discover::{discover, load_manifest, validate_manifest, MANIFEST_FILE};
pub use process::{Backoff, PluginHandle, INIT_TIMEOUT};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginError {
    #[error("manifest: {0}")]
    Manifest(String),
    #[error("{0} is not allowed by the plugin's permissions")]
    Denied(String),
    #[error("plugin exited: {0}")]
    Exited(String),
    #[error("timed out")]
    Timeout,
    #[error("{0}")]
    Rpc(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    Enabled,
    Disabled,
    Error { reason: String },
}

/// One plugin found on disk. `state` and `granted` start as `Disabled` and
/// empty; the app overlays the persisted enablement and the permissions
/// the user approved before calling [`PluginHandle::start`].
#[derive(Debug, Clone, PartialEq)]
pub struct InstalledPlugin {
    pub manifest: berth_plugin_protocol::Manifest,
    pub dir: std::path::PathBuf,
    pub state: PluginState,
    pub granted: Vec<berth_plugin_protocol::Permission>,
}
