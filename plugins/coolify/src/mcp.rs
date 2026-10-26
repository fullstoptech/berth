//! MCP passthrough data (P9-T4): when the Coolify instance has its MCP
//! server enabled, the plugin hands the host a `host/mcp.servers`
//! notification payload — `[{name, url, headers}]` — so the assistant can
//! reach Coolify's own tools through the `Mcp` permission. This module
//! only builds that payload; sending it is the plugin binary's job
//! (P9-T5). The credential enters the payload only, exactly where the
//! protocol asks for it; it is never logged here.

use secrecy::{ExposeSecret, SecretString};

use crate::client::CoolifyClient;

/// Builds the `host/mcp.servers` payload for the client's instance, or
/// `None` when MCP is disabled. The URL is the instance root plus
/// Coolify's `/mcp` endpoint.
#[must_use]
pub fn mcp_servers_payload(
    client: &CoolifyClient,
    token: &SecretString,
    enabled: bool,
) -> Option<serde_json::Value> {
    if !enabled {
        return None;
    }
    let root = client
        .api_base()
        .strip_suffix("/api/v1")
        .unwrap_or_else(|| client.api_base());
    Some(serde_json::json!([{
        "name": "coolify",
        "url": format!("{root}/mcp"),
        "headers": {
            "Authorization": format!("Bearer {}", token.expose_secret()),
        },
    }]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> CoolifyClient {
        CoolifyClient::new(
            "https://coolify.example.test",
            SecretString::from("test-token-not-real".to_owned()),
        )
        .expect("base builds")
    }

    #[test]
    fn mcp_payload_names_the_instance_mcp_endpoint() {
        let token_text = "test-token-not-real";
        let token = SecretString::from(token_text.to_owned());
        let payload = mcp_servers_payload(&client(), &token, true).expect("enabled");
        let servers = payload.as_array().expect("array payload");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["name"], "coolify");
        assert_eq!(servers[0]["url"], "https://coolify.example.test/mcp");
        assert_eq!(
            servers[0]["headers"]["Authorization"].as_str(),
            Some(format!("Bearer {token_text}").as_str())
        );
    }

    #[test]
    fn mcp_payload_is_none_when_disabled() {
        let token = SecretString::from("test-token-not-real".to_owned());
        assert!(mcp_servers_payload(&client(), &token, false).is_none());
    }
}
