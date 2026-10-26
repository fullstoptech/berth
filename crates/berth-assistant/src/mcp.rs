//! The MCP server behind `berth mcp` (PLAN §6.9, ADR 0006): an rmcp stdio
//! server exposing the 25 catalog tools under their wire names and
//! forwarding every call over the bridge to the running app, which owns
//! execution, approvals and redaction. This process holds no engine state.
//!
//! When nothing answers at the bridge socket, startup fails at once with a
//! "Berth is not running" error (ADR 0006), so a harness never waits on a
//! dead server. Tool arguments are never logged here either.

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};

use crate::bridge::{self, BridgeRequest, BridgeResponse};
use crate::catalog;
use crate::AssistantError;

struct BerthMcp {
    bridge: std::path::PathBuf,
    session: berth_core::SessionId,
    token: String,
}

fn tool_def(def: &catalog::ToolDef) -> Tool {
    let schema = (def.input_schema)();
    let object: JsonObject = match schema {
        serde_json::Value::Object(map) => map,
        _ => JsonObject::new(),
    };
    let annotations = ToolAnnotations::from_raw(
        None,
        Some(def.risk == berth_core::Risk::Read),
        Some(def.risk == berth_core::Risk::Destructive),
        None,
        Some(false),
    );
    Tool::new(def.wire_name(), def.description, Arc::new(object)).with_annotations(annotations)
}

/// The `call_id` a harness may send in `_meta` (`call_id` or `tool_use_id`).
fn call_id_of(request: &CallToolRequestParams) -> Option<String> {
    let meta = request.meta.as_ref()?;
    let value = serde_json::to_value(meta).ok()?;
    ["call_id", "tool_use_id", "toolUseId"]
        .iter()
        .find_map(|key| value.get(key).and_then(serde_json::Value::as_str))
        .map(ToString::to_string)
}

fn text_result(ok: bool, text: String, data: Option<serde_json::Value>) -> CallToolResult {
    let mut result = if ok {
        CallToolResult::success(vec![ContentBlock::text(text)])
    } else {
        CallToolResult::error(vec![ContentBlock::text(text)])
    };
    if data.is_some() {
        result.structured_content = data;
    }
    result
}

impl ServerHandler for BerthMcp {
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: catalog::catalog().iter().map(tool_def).collect(),
            ..ListToolsResult::default()
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        catalog::tool(name).map(tool_def)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(def) = catalog::tool(&request.name) else {
            return Ok(text_result(
                false,
                format!(
                    "unknown tool {}; call tools/list for the tools Berth offers",
                    request.name
                ),
                None,
            )
            .into());
        };
        let call_id = call_id_of(&request);
        let args = request
            .arguments
            .map_or_else(|| serde_json::json!({}), serde_json::Value::Object);
        let forwarded = BridgeRequest {
            id: uuid::Uuid::now_v7(),
            token: self.token.clone(),
            session: self.session,
            tool: def.name.to_string(),
            args,
            call_id,
        };
        let result = match bridge::call(&self.bridge, &forwarded).await {
            Ok(BridgeResponse::Ok { text, data, .. }) => text_result(true, text, data),
            Ok(BridgeResponse::Denied { reason, .. }) => {
                text_result(false, format!("Denied: {reason}"), None)
            }
            Ok(BridgeResponse::Error { message, .. }) => text_result(false, message, None),
            Err(err) => text_result(false, err.to_string(), None),
        };
        Ok(result.into())
    }

    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("berth", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Berth's Docker tools. Reads run at once; changes wait for the user to \
                 approve in Berth; destructive tools need the name typed. Environment \
                 values are always masked as ••••.",
            )
    }
}

/// Whether something accepts connections at `bridge` right now. The probe
/// connection carries no request; the server side just sees it close.
async fn probe(bridge: &std::path::Path) -> Result<(), AssistantError> {
    let connected = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::UnixStream::connect(bridge),
    )
    .await;
    match connected {
        Ok(Ok(_stream)) => Ok(()),
        _ => Err(AssistantError::Bridge(format!(
            "Berth is not running (no answer at {}). Open Berth and try again.",
            bridge.display()
        ))),
    }
}

/// Entry point of `berth mcp --bridge <socket> --session <id>`: an rmcp stdio
/// server exposing `catalog()`, forwarding to the bridge. Serves until the
/// harness closes stdio.
///
/// # Errors
/// At once, when the bridge socket does not answer (ADR 0006); otherwise
/// when the stdio transport fails.
pub async fn serve_mcp_stdio(
    bridge_path: std::path::PathBuf,
    session: berth_core::SessionId,
    token: String,
) -> Result<(), AssistantError> {
    probe(&bridge_path).await?;
    let server = BerthMcp {
        bridge: bridge_path,
        session,
        token,
    };
    let running = server
        .serve(rmcp::transport::io::stdio())
        .await
        .map_err(|e| AssistantError::Protocol(format!("starting the MCP server: {e}")))?;
    running
        .waiting()
        .await
        .map_err(|e| AssistantError::Protocol(format!("the MCP server stopped: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_tool_defs_carry_wire_names_schemas_and_risk_hints() {
        assert_eq!(catalog::catalog().len(), 25);
        for def in catalog::catalog() {
            let tool = tool_def(def);
            assert_eq!(tool.name, def.wire_name());
            assert!(!tool.name.contains('.'), "{}", tool.name);
            assert_eq!(
                tool.input_schema.get("type").and_then(|t| t.as_str()),
                Some("object"),
                "{}",
                tool.name
            );
            let hints = tool.annotations.expect("hints");
            assert_eq!(
                hints.read_only_hint,
                Some(def.risk == berth_core::Risk::Read),
                "{}",
                tool.name
            );
            assert_eq!(
                hints.destructive_hint,
                Some(def.risk == berth_core::Risk::Destructive),
                "{}",
                tool.name
            );
        }
    }

    #[tokio::test]
    async fn mcp_serve_fails_at_once_when_berth_is_not_running() {
        let dir = tempfile::tempdir().expect("dir");
        let socket = dir.path().join("missing.sock");
        let served = serve_mcp_stdio(
            socket,
            berth_core::SessionId(uuid::Uuid::nil()),
            "tok".to_string(),
        );
        let err = tokio::time::timeout(std::time::Duration::from_secs(5), served)
            .await
            .expect("fails fast")
            .expect_err("no bridge");
        assert!(err.to_string().contains("not running"), "{err}");
    }
}
