pub mod bollard_engine;
pub mod client;
pub mod colima;
pub mod colima_parse;
pub mod compose;
pub mod discovery;
mod files;
pub mod git;
pub mod mappers;
pub mod mock;
pub mod projects;

pub use bollard_engine::BollardEngine;
pub use client::{unsupported_stream, Bs, EngineClient, ExecHandle, RegistryAuth};
pub use colima::{ColimaCli, ColimaControl, MockColima};
pub use compose::{compose_args, parse_progress_json, CliCompose, ComposeRunner, MockCompose};
pub use git::GitCli;
pub use mock::{MockEngine, MockWorld};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{engine} is not reachable: {reason}")]
    Unreachable { engine: String, reason: String },
    #[error("{0}")]
    Docker(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0} is not supported by this engine")]
    Unsupported(String),
    #[error("cancelled")]
    Cancelled,
    #[error("timed out after {0:?}")]
    Timeout(std::time::Duration),
    #[error(transparent)]
    Sys(#[from] berth_sys::SysError),
    #[error("could not read {what}: {message}")]
    Parse { what: String, message: String },
}

#[cfg(test)]
mod tests {
    #[test]
    fn compiles() {}
}
