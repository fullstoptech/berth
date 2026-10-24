pub mod paths;
pub mod process;
pub mod search_path;

pub use process::{run_capture, spawn_lines, Output, ProcessLine, SpawnOpts};
pub use search_path::SearchPath;

#[derive(Debug, thiserror::Error)]
pub enum SysError {
    #[error("{program} was not found on this Mac's PATH")]
    NotFound { program: String },
    #[error("{program} failed to start: {message}")]
    Spawn { program: String, message: String },
    #[error("{program} timed out after {secs} s")]
    Timeout { program: String, secs: u64 },
    #[error("cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    #[test]
    fn compiles() {}
}
