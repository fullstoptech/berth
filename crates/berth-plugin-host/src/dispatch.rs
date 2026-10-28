//! Dispatch of plugin → host calls: `host/secret.get` and `host/log`
//! (PLAN §6.10). Everything else a plugin sends as a request is answered
//! "method not found". Notifications other than `host/log` are surfaced to
//! the app through [`crate::PluginHandle::notifications`].
//!
//! Secret values are returned to the plugin (that is the point of the
//! call) but never logged here: log lines name the plugin and the key,
//! never the value.

use std::sync::Arc;

use berth_plugin_protocol::{methods, Permission, Request, Response, RpcError, SecretDecl};
use berth_store::SecretStore;
use secrecy::ExposeSecret;

/// JSON-RPC error codes used in answers to a plugin.
pub const ERR_METHOD_NOT_FOUND: i64 = -32601;
pub const ERR_INVALID_PARAMS: i64 = -32602;
pub const ERR_DENIED: i64 = -32001;
pub const ERR_SECRET_NOT_SET: i64 = -32002;
pub const ERR_SECRET_STORE: i64 = -32003;

/// The store key under which a plugin secret lives, so the Plugins screen
/// writes and this host reads the same key: `plugin/<plugin id>/<key>`.
pub fn secret_store_key(plugin_id: &str, key: &str) -> String {
    format!("plugin/{plugin_id}/{key}")
}

/// The secret gate (PLAN D45): a key must be declared in the manifest's
/// `secrets` list *and* covered by a granted `Secrets` permission.
/// Anything else — undeclared, declared but not granted, granted but not
/// declared — is denied.
pub fn secret_allowed(declared: &[SecretDecl], granted: &[Permission], key: &str) -> bool {
    let declared = declared.iter().any(|d| d.key == key);
    let granted = granted
        .iter()
        .any(|p| matches!(p, Permission::Secrets { keys } if keys.iter().any(|k| k == key)));
    declared && granted
}

/// What dispatch needs to know about one running plugin. Secrets come
/// through the injected [`SecretStore`]; this crate never depends on the
/// app.
pub struct Context {
    pub plugin_id: String,
    pub declared: Vec<SecretDecl>,
    pub granted: Vec<Permission>,
    pub secrets: Arc<dyn SecretStore>,
}

impl Context {
    pub fn new(plugin: &crate::InstalledPlugin, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            plugin_id: plugin.manifest.id.clone(),
            declared: plugin.manifest.secrets.clone(),
            granted: plugin.granted.clone(),
            secrets,
        }
    }
}

fn ok(id: u64, result: serde_json::Value) -> Response {
    Response {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

fn err(id: u64, code: i64, message: String) -> Response {
    Response {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(RpcError { code, message }),
    }
}

/// Answer one plugin → host request.
pub fn handle_request(ctx: &Context, request: &Request) -> Response {
    match request.method.as_str() {
        methods::SECRET_GET => secret_get(ctx, request),
        methods::LOG => {
            handle_log(&ctx.plugin_id, &request.params);
            ok(request.id, serde_json::json!({}))
        }
        other => err(
            request.id,
            ERR_METHOD_NOT_FOUND,
            format!("host has no method {other:?}"),
        ),
    }
}

fn secret_get(ctx: &Context, request: &Request) -> Response {
    let Some(key) = request.params.get("key").and_then(|v| v.as_str()) else {
        return err(
            request.id,
            ERR_INVALID_PARAMS,
            "host/secret.get needs a string \"key\"".into(),
        );
    };
    if !secret_allowed(&ctx.declared, &ctx.granted, key) {
        tracing::warn!(plugin = %ctx.plugin_id, key, "plugin asked for a secret it may not have");
        return err(
            request.id,
            ERR_DENIED,
            format!("secret {key:?} is not declared and granted"),
        );
    }
    match ctx.secrets.get(&secret_store_key(&ctx.plugin_id, key)) {
        Ok(Some(value)) => ok(
            request.id,
            serde_json::json!({ "value": value.expose_secret() }),
        ),
        Ok(None) => err(
            request.id,
            ERR_SECRET_NOT_SET,
            format!("secret {key:?} is not set"),
        ),
        Err(e) => {
            tracing::warn!(plugin = %ctx.plugin_id, key, "secret store read failed: {e}");
            err(
                request.id,
                ERR_SECRET_STORE,
                "secret store unavailable".into(),
            )
        }
    }
}

/// Forward a `host/log` notification (or request params) to tracing.
/// Accepts `{"level": "warn", "message": "..."}`, `{"message": "..."}` or
/// a bare string. The message is plugin-supplied text.
pub fn handle_log(plugin_id: &str, params: &serde_json::Value) {
    let (level, message) = match params {
        serde_json::Value::String(s) => ("info", s.as_str()),
        serde_json::Value::Object(map) => (
            map.get("level").and_then(|v| v.as_str()).unwrap_or("info"),
            map.get("message").and_then(|v| v.as_str()).unwrap_or(""),
        ),
        _ => ("info", ""),
    };
    match level {
        "error" => tracing::error!(plugin = %plugin_id, "{message}"),
        "warn" => tracing::warn!(plugin = %plugin_id, "{message}"),
        "debug" => tracing::debug!(plugin = %plugin_id, "{message}"),
        "trace" => tracing::trace!(plugin = %plugin_id, "{message}"),
        _ => tracing::info!(plugin = %plugin_id, "{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use berth_store::InMemorySecretStore;
    use secrecy::SecretString;
    use serde_json::json;

    fn declared() -> Vec<SecretDecl> {
        vec![
            SecretDecl {
                key: "api_token".into(),
                label: "API token".into(),
            },
            SecretDecl {
                key: "declared_only".into(),
                label: "Not granted".into(),
            },
        ]
    }

    fn granted() -> Vec<Permission> {
        vec![
            Permission::Secrets {
                keys: vec!["api_token".into(), "granted_only".into()],
            },
            Permission::ReadEngines,
        ]
    }

    fn ctx(store: Arc<InMemorySecretStore>) -> Context {
        Context {
            plugin_id: "fixture".into(),
            declared: declared(),
            granted: granted(),
            secrets: store,
        }
    }

    fn request(method: &str, params: serde_json::Value) -> Request {
        Request {
            jsonrpc: "2.0".into(),
            id: 42,
            method: method.into(),
            params,
        }
    }

    #[test]
    fn secret_gate_needs_declared_and_granted() {
        let (d, g) = (declared(), granted());
        assert!(secret_allowed(&d, &g, "api_token"));
        assert!(!secret_allowed(&d, &g, "declared_only"), "not granted");
        assert!(!secret_allowed(&d, &g, "granted_only"), "not declared");
        assert!(!secret_allowed(&d, &g, "anything_else"));
        assert!(!secret_allowed(&d, &[], "api_token"), "nothing granted");
        assert!(!secret_allowed(&[], &g, "api_token"), "nothing declared");
    }

    #[test]
    fn store_key_is_namespaced_per_plugin() {
        assert_eq!(secret_store_key("coolify", "token"), "plugin/coolify/token");
    }

    #[test]
    fn secret_get_returns_the_value_for_a_granted_key() {
        let store = Arc::new(InMemorySecretStore::default());
        store
            .set(
                &secret_store_key("fixture", "api_token"),
                &SecretString::from("test-value".to_string()),
            )
            .unwrap();
        let response = handle_request(
            &ctx(store),
            &request(methods::SECRET_GET, json!({"key": "api_token"})),
        );
        assert_eq!(response.result, Some(json!({"value": "test-value"})));
        assert!(response.error.is_none());
    }

    #[test]
    fn secret_get_denies_undeclared_and_ungranted_keys() {
        let store = Arc::new(InMemorySecretStore::default());
        // Even a value sitting in the store must not leak for these keys.
        for key in ["declared_only", "granted_only", "nope"] {
            store
                .set(
                    &secret_store_key("fixture", key),
                    &SecretString::from("test-value".to_string()),
                )
                .unwrap();
            let response = handle_request(
                &ctx(Arc::clone(&store)),
                &request(methods::SECRET_GET, json!({"key": key})),
            );
            let error = response.error.expect("denied keys get an error");
            assert_eq!(error.code, ERR_DENIED, "{key}");
            assert!(response.result.is_none(), "{key} leaked a value");
        }
    }

    #[test]
    fn secret_get_reports_a_granted_but_unset_key() {
        let response = handle_request(
            &ctx(Arc::new(InMemorySecretStore::default())),
            &request(methods::SECRET_GET, json!({"key": "api_token"})),
        );
        let error = response.error.expect("unset key gets an error");
        assert_eq!(error.code, ERR_SECRET_NOT_SET);
        assert!(error.message.contains("not set"), "{}", error.message);
    }

    #[test]
    fn secret_get_rejects_missing_key_param() {
        let response = handle_request(
            &ctx(Arc::new(InMemorySecretStore::default())),
            &request(methods::SECRET_GET, json!({})),
        );
        let error = response.error.expect("bad params get an error");
        assert_eq!(error.code, ERR_INVALID_PARAMS);
    }

    #[test]
    fn unknown_host_method_is_method_not_found_and_log_is_answered() {
        let c = ctx(Arc::new(InMemorySecretStore::default()));
        let response = handle_request(&c, &request("host/engines.list", json!({})));
        let error = response.error.expect("unknown method gets an error");
        assert_eq!(error.code, ERR_METHOD_NOT_FOUND);

        let response = handle_request(
            &c,
            &request(methods::LOG, json!({"level": "warn", "message": "hi"})),
        );
        assert_eq!(response.result, Some(json!({})));
    }
}
