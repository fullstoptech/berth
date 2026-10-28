//! The real `berth-plugin-host` driving the real `berth-coolify` binary:
//! discovery of the bundled manifest, the permission and secret gate, the
//! `initialize` handshake, and view, action and log traffic against a
//! wiremock Coolify. This is the seam the app's Plugins screen stands on.

#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use berth_plugin_host::dispatch::secret_store_key;
use berth_plugin_host::{discover, InstalledPlugin, PluginHandle, PluginState};
use berth_plugin_protocol::methods;
use berth_store::{InMemorySecretStore, SecretStore};
use secrecy::SecretString;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{bearer_token, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "host-test-token-not-real";
const REQ: Duration = Duration::from_secs(5);

/// The shipped manifest next to the built binary, discovered like an
/// installed plugin, with every permission it asks for granted.
fn install(grant: bool) -> (tempfile::TempDir, InstalledPlugin) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("coolify");
    std::fs::create_dir_all(&dir).expect("plugin dir");
    std::fs::copy(
        env!("CARGO_BIN_EXE_berth-coolify"),
        dir.join("berth-coolify"),
    )
    .expect("copy binary");
    std::fs::write(
        dir.join("berth-plugin.json"),
        include_str!("../berth-plugin.json"),
    )
    .expect("write manifest");
    let mut found = discover(&[tmp.path().to_path_buf()]);
    assert_eq!(found.len(), 1, "the shipped manifest is discoverable");
    let mut plugin = found.pop().expect("one plugin");
    if grant {
        plugin.granted = plugin.manifest.permissions.clone();
    }
    plugin.state = PluginState::Enabled;
    (tmp, plugin)
}

fn secrets(token: Option<&str>) -> Arc<InMemorySecretStore> {
    let store = InMemorySecretStore::default();
    if let Some(token) = token {
        store
            .set(
                &secret_store_key("coolify", "coolify/token"),
                &SecretString::from(token.to_owned()),
            )
            .expect("set secret");
    }
    Arc::new(store)
}

async fn start(
    plugin: &InstalledPlugin,
    store: Arc<InMemorySecretStore>,
    server: &MockServer,
) -> PluginHandle {
    PluginHandle::start(
        plugin,
        store,
        json!({ "base_url": server.uri() }),
        std::env::var_os("PATH").unwrap_or_default(),
        CancellationToken::new(),
    )
    .await
    .expect("the plugin starts and answers initialize")
}

async fn serve_json(server: &MockServer, route: &str, body: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/{route}")))
        .and(bearer_token(TOKEN))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn host_loads_a_view_with_the_token_from_its_secret_store() {
    let server = MockServer::start().await;
    serve_json(
        &server,
        "resources",
        include_str!("../../../fixtures/coolify/resources.json"),
    )
    .await;
    let (_tmp, plugin) = install(true);
    let handle = start(&plugin, secrets(Some(TOKEN)), &server).await;
    let view = handle
        .request(methods::VIEW_LOAD, json!({ "view": "resources" }), REQ)
        .await
        .expect("view/load answers");
    assert_eq!(view["kind"], "table");
    assert_eq!(view["rows"].as_array().map(Vec::len), Some(3));
    handle.stop().await;
}

#[tokio::test]
async fn host_denies_the_token_when_the_user_has_not_granted_the_secret() {
    let server = MockServer::start().await;
    serve_json(&server, "resources", "[]").await;
    let (_tmp, plugin) = install(false);
    let handle = start(&plugin, secrets(Some(TOKEN)), &server).await;
    let view = handle
        .request(methods::VIEW_LOAD, json!({ "view": "resources" }), REQ)
        .await
        .expect("view/load answers");
    assert_eq!(view["kind"], "message");
    let text = view["text"].as_str().unwrap_or_default();
    assert!(text.contains("not declared and granted"), "{text}");
    assert!(!text.contains(TOKEN));
    handle.stop().await;
}

#[tokio::test]
async fn host_runs_an_action_and_streams_a_deployment_log_to_a_final_status() {
    let server = MockServer::start().await;
    serve_json(
        &server,
        "deployments/dep-uuid-00ff",
        include_str!("../../../fixtures/coolify/deployment-failed.json"),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/applications/app-uuid-0001/restart"))
        .and(bearer_token(TOKEN))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"message":"Restart queued."}"#),
        )
        .mount(&server)
        .await;
    let (_tmp, plugin) = install(true);
    let handle = start(&plugin, secrets(Some(TOKEN)), &server).await;
    let mut notes = handle.notifications();

    let result = handle
        .request(
            methods::ACTION_INVOKE,
            json!({ "action": "restart", "params": { "uuid": "app-uuid-0001", "kind": "application" } }),
            REQ,
        )
        .await
        .expect("action/invoke answers");
    assert_eq!(result["message"], "Restart queued.");

    let stream = "deployment/dep-uuid-00ff/logs";
    handle
        .request(
            methods::LOGS_OPEN,
            json!({ "stream": stream, "params": {} }),
            REQ,
        )
        .await
        .expect("logs/open answers");
    let final_status = tokio::time::timeout(REQ, async {
        loop {
            let note = notes.recv().await.expect("notification channel open");
            if note.method == methods::LOG_LINES {
                if let Some(status) = note.params.get("final_status") {
                    return status.clone();
                }
            }
        }
    })
    .await
    .expect("the stream reaches a final status");
    assert_eq!(final_status, "failed");
    handle.stop().await;
}
