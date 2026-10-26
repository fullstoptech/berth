//! The MCP server behind `berth mcp` (ADR 0006). Filled in by P7-T2; until
//! then `serve_mcp_stdio` is a stub with the final signature (PLAN §6.9).

use crate::AssistantError;

/// Entry point of `berth mcp --bridge <socket> --session <id>`: an rmcp stdio
/// server exposing `catalog()`, forwarding to the bridge.
///
/// # Errors
/// Always, until P7-T2 implements the server.
pub async fn serve_mcp_stdio(
    _bridge: std::path::PathBuf,
    _session: berth_core::SessionId,
    _token: String,
) -> Result<(), AssistantError> {
    Err(AssistantError::Protocol(
        "the MCP server is not implemented yet (P7-T2)".into(),
    ))
}
