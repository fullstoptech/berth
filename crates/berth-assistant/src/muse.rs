//! muse adapter (ADR 0005). Filled in by P7-T4; until then these are stubs
//! with the final signatures (PLAN §6.9).

use crate::{AssistantError, AssistantEvent, SessionConfig, Started};
use tokio_util::sync::CancellationToken;

#[must_use]
pub fn serve_args() -> Vec<String> {
    Vec::new()
}

#[must_use]
pub fn session_start(_cfg: &SessionConfig) -> serde_json::Value {
    serde_json::Value::Null
}

#[must_use]
pub fn parse_notification(_v: &serde_json::Value) -> Vec<AssistantEvent> {
    Vec::new()
}

/// The answer to `approval/requested`: allow for Berth's tools, deny otherwise.
#[must_use]
pub fn approval_decision(_params: &serde_json::Value) -> serde_json::Value {
    serde_json::Value::Null
}

/// # Errors
/// Always, until P7-T4 implements the adapter.
pub async fn start(
    _cfg: SessionConfig,
    _cancel: CancellationToken,
) -> Result<Started, AssistantError> {
    Err(AssistantError::Protocol(
        "the muse adapter is not implemented yet (P7-T4)".into(),
    ))
}
