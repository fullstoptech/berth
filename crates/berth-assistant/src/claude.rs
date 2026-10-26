//! Claude Code adapter (ADR 0005). Filled in by P7-T3; until then these are
//! stubs with the final signatures (PLAN §6.9).

use crate::{AssistantError, AssistantEvent, SessionConfig, Started};
use tokio_util::sync::CancellationToken;

/// The flags every session passes, kept in one table (ADR 0005).
pub const FLAGS: &[&str] = &[];

#[must_use]
pub fn args(_cfg: &SessionConfig, _mcp_config: &std::path::Path) -> Vec<String> {
    Vec::new()
}

#[must_use]
pub fn parse_line(_line: &str) -> Vec<AssistantEvent> {
    Vec::new()
}

/// # Errors
/// Always, until P7-T3 implements the adapter.
pub async fn start(
    _cfg: SessionConfig,
    _cancel: CancellationToken,
) -> Result<Started, AssistantError> {
    Err(AssistantError::Protocol(
        "the claude adapter is not implemented yet (P7-T3)".into(),
    ))
}
