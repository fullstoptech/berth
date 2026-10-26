#![allow(clippy::expect_used)]
//! Conformance tests for the plugin host against the fixture plugin
//! (`tests/fixture-plugin/`), per P9-T2: initialize, view/load,
//! action/invoke, logs/lines, secret allowed and denied, crash and
//! restart, timeout. Backoff is injected at milliseconds so no test
//! sleeps on the production 1 s / 5 s / 30 s schedule, and every wait is
//! deadline-bounded.

use std::sync::Arc;
use std::time::Duration;

use berth_plugin_host::dispatch::secret_store_key;
use berth_plugin_host::{Backoff, InstalledPlugin, PluginError, PluginHandle, PluginState};
use berth_plugin_protocol::{methods, Manifest, Permission, SecretDecl};
use berth_store::{InMemorySecretStore, SecretStore};
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const REQ: Duration = Duration::from_secs(5);

fn manifest() -> Manifest {
    Manifest {
        id: "fixture".into(),
        name: "Fixture".into(),
        version: "0.1.0".into(),
        protocol: berth_plugin_protocol::PROTOCOL_VERSION,
        executable: "fixture-plugin".into(),
        permissions: vec![Permission::Secrets {
            keys: vec!["api_token".into()],
        }],
        contributes: Vec::new(),
        config: Vec::new(),
        secrets: vec![
            SecretDecl {
                key: "api_token".into(),
                label: "API token".into(),
            },
            SecretDecl {
                key: "declared_only".into(),
                label: "Never granted".into(),
            },
        ],
    }
}

fn granted_all() -> Vec<Permission> {
    vec![Permission::Secrets {
        keys: vec!["api_token".into()],
    }]
}

/// Copy the fixture binary into a temp plugin dir and discover it there.
fn install(granted: Vec<Permission>) -> (tempfile::TempDir, InstalledPlugin) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("fixture");
    std::fs::create_dir_all(&dir).expect("plugin dir");
    std::fs::copy(
        env!("CARGO_BIN_EXE_berth-fixture-plugin"),
        dir.join("fixture-plugin"),
    )
    .expect("copy fixture binary");
    std::fs::write(
        dir.join("berth-plugin.json"),
        serde_json::to_string(&manifest()).expect("manifest json"),
    )
    .expect("write manifest");
    let mut found = berth_plugin_host::discover(&[tmp.path().to_path_buf()]);
    assert_eq!(found.len(), 1, "discover finds the fixture");
    let mut plugin = found.pop().expect("one plugin");
    plugin.granted = granted;
    plugin.state = PluginState::Enabled;
    (tmp, plugin)
}

fn secrets(pairs: &[(&str, &str)]) -> Arc<InMemorySecretStore> {
    let store = InMemorySecretStore::default();
    for (key, value) in pairs {
        store
            .set(
                &secret_store_key("fixture", key),
                &SecretString::from((*value).to_string()),
            )
            .expect("set secret");
    }
    Arc::new(store)
}

async fn start(
    plugin: &InstalledPlugin,
    store: Arc<InMemorySecretStore>,
    config: Value,
    cancel: CancellationToken,
) -> Result<PluginHandle, PluginError> {
    PluginHandle::start_with_backoff(
        plugin,
        store,
        config,
        std::env::var_os("PATH").unwrap_or_default(),
        cancel,
        Backoff {
            delays: vec![
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(40),
            ],
        },
    )
    .await
}

async fn wait_until(mut cond: impl FnMut() -> bool, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if cond() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn log_lines(path: &std::path::Path) -> usize {
    std::fs::read_to_string(path).map_or(0, |t| t.lines().count())
}

#[tokio::test]
async fn initialize_handshake_reports_capabilities() {
    let (_tmp, plugin) = install(granted_all());
    let handle = start(&plugin, secrets(&[]), json!({}), CancellationToken::new())
        .await
        .expect("start");
    assert_eq!(handle.state(), PluginState::Enabled);
    assert!(handle.is_running());
    let init = handle.init_result().expect("init result stored");
    let capabilities = init["capabilities"].as_array().expect("capabilities");
    assert!(capabilities.iter().any(|c| c == "views"), "{init}");
    handle.stop().await;
}

#[tokio::test]
async fn view_load_returns_a_table_view() {
    let (_tmp, plugin) = install(granted_all());
    let handle = start(&plugin, secrets(&[]), json!({}), CancellationToken::new())
        .await
        .expect("start");
    let view = handle
        .request(
            methods::VIEW_LOAD,
            json!({"view": "main", "params": {}}),
            REQ,
        )
        .await
        .expect("view/load");
    assert_eq!(view["kind"], "table");
    assert_eq!(view["title"], "Fixture main");
    assert_eq!(view["rows"][0]["cells"]["name"]["value"], "alpha");
    handle.stop().await;
}

#[tokio::test]
async fn action_invoke_answers_and_invalidates() {
    let (_tmp, plugin) = install(granted_all());
    let handle = start(&plugin, secrets(&[]), json!({}), CancellationToken::new())
        .await
        .expect("start");
    let mut notifications = handle.notifications();
    let result = handle
        .request(
            methods::ACTION_INVOKE,
            json!({"action": "deploy", "params": {}}),
            REQ,
        )
        .await
        .expect("action/invoke");
    assert_eq!(result["message"], "did deploy");
    assert_eq!(result["invalidate"], json!(["main"]));
    let notification = tokio::time::timeout(REQ, notifications.recv())
        .await
        .expect("invalidate arrives in time")
        .expect("invalidate notification");
    assert_eq!(notification.method, methods::INVALIDATE);
    assert_eq!(notification.params["view"], "main");
    handle.stop().await;
}

#[tokio::test]
async fn logs_open_streams_lines_then_final_status() {
    let (_tmp, plugin) = install(granted_all());
    let handle = start(&plugin, secrets(&[]), json!({}), CancellationToken::new())
        .await
        .expect("start");
    let mut notifications = handle.notifications();
    handle
        .request(
            methods::LOGS_OPEN,
            json!({"stream": "deploy-1", "params": {}}),
            REQ,
        )
        .await
        .expect("logs/open");
    let first = tokio::time::timeout(REQ, notifications.recv())
        .await
        .expect("lines in time")
        .expect("lines notification");
    assert_eq!(first.method, methods::LOG_LINES);
    assert_eq!(first.params["lines"], json!(["line one", "line two"]));
    let last = tokio::time::timeout(REQ, notifications.recv())
        .await
        .expect("final in time")
        .expect("final notification");
    assert_eq!(last.params["final_status"], "finished");
    handle.stop().await;
}

#[tokio::test]
async fn secret_allowed_for_a_declared_and_granted_key() {
    let (_tmp, plugin) = install(granted_all());
    let handle = start(
        &plugin,
        secrets(&[("api_token", "test-value")]),
        json!({}),
        CancellationToken::new(),
    )
    .await
    .expect("start");
    let result = handle
        .request(
            methods::ACTION_INVOKE,
            json!({"action": "echo-secret", "params": {"key": "api_token"}}),
            REQ,
        )
        .await
        .expect("echo-secret");
    assert_eq!(result["message"], "got:test-value");
    handle.stop().await;
}

#[tokio::test]
async fn secret_denied_for_an_undeclared_key() {
    let (_tmp, plugin) = install(granted_all());
    let handle = start(
        &plugin,
        secrets(&[("other", "test-value")]),
        json!({}),
        CancellationToken::new(),
    )
    .await
    .expect("start");
    let result = handle
        .request(
            methods::ACTION_INVOKE,
            json!({"action": "echo-secret", "params": {"key": "other"}}),
            REQ,
        )
        .await
        .expect("echo-secret");
    let message = result["message"].as_str().expect("message");
    assert!(message.contains("not declared and granted"), "{message}");
    assert!(!message.contains("test-value"), "value leaked: {message}");
    handle.stop().await;
}

#[tokio::test]
async fn secret_denied_when_declared_but_not_granted() {
    // Granted nothing: even the declared api_token must be refused.
    let (_tmp, plugin) = install(Vec::new());
    let handle = start(
        &plugin,
        secrets(&[("api_token", "test-value")]),
        json!({}),
        CancellationToken::new(),
    )
    .await
    .expect("start");
    for key in ["api_token", "declared_only"] {
        let result = handle
            .request(
                methods::ACTION_INVOKE,
                json!({"action": "echo-secret", "params": {"key": key}}),
                REQ,
            )
            .await
            .expect("echo-secret");
        let message = result["message"].as_str().expect("message");
        assert!(
            message.contains("not declared and granted"),
            "{key}: {message}"
        );
        assert!(!message.contains("test-value"), "{key} leaked: {message}");
    }
    handle.stop().await;
}

#[tokio::test]
async fn secret_granted_but_not_set_is_reported() {
    let (_tmp, plugin) = install(granted_all());
    let handle = start(&plugin, secrets(&[]), json!({}), CancellationToken::new())
        .await
        .expect("start");
    let result = handle
        .request(
            methods::ACTION_INVOKE,
            json!({"action": "echo-secret", "params": {"key": "api_token"}}),
            REQ,
        )
        .await
        .expect("echo-secret");
    let message = result["message"].as_str().expect("message");
    assert!(message.contains("not set"), "{message}");
    handle.stop().await;
}

#[tokio::test]
async fn crash_then_restart_recovers() {
    let (tmp, plugin) = install(granted_all());
    let starts = tmp.path().join("starts.log");
    let counter = tmp.path().join("counter");
    let handle = start(
        &plugin,
        secrets(&[]),
        json!({
            "starts_log": starts.to_str().expect("utf8 path"),
            "counter_path": counter.to_str().expect("utf8 path"),
        }),
        CancellationToken::new(),
    )
    .await
    .expect("start");
    let crashed = handle
        .request(
            methods::ACTION_INVOKE,
            json!({"action": "crash-once", "params": {}}),
            REQ,
        )
        .await;
    assert!(
        matches!(crashed, Err(PluginError::Exited(_))),
        "in-flight request fails with Exited: {crashed:?}"
    );
    wait_until(|| handle.restarts() == 1, "one restart scheduled").await;
    let recovered = handle
        .request(
            methods::ACTION_INVOKE,
            json!({"action": "crash-once", "params": {}}),
            REQ,
        )
        .await
        .expect("second generation answers");
    assert_eq!(recovered["message"], "survived");
    assert_eq!(log_lines(&starts), 2, "two generations initialized");
    handle.stop().await;
}

#[tokio::test]
async fn crash_loop_exhausts_restarts_into_error_state() {
    let (tmp, plugin) = install(granted_all());
    let starts = tmp.path().join("starts.log");
    let handle = start(
        &plugin,
        secrets(&[]),
        json!({"starts_log": starts.to_str().expect("utf8 path")}),
        CancellationToken::new(),
    )
    .await
    .expect("start");
    // One initial process plus three restarts = four generations.
    for generation in 1..=4 {
        let crashed = handle
            .request(
                methods::ACTION_INVOKE,
                json!({"action": "crash", "params": {}}),
                REQ,
            )
            .await;
        assert!(
            matches!(crashed, Err(PluginError::Exited(_))),
            "generation {generation}: {crashed:?}"
        );
    }
    wait_until(
        || matches!(handle.state(), PluginState::Error { .. }),
        "Error state",
    )
    .await;
    assert_eq!(handle.restarts(), 3);
    assert_eq!(log_lines(&starts), 4, "initial + three restarts");
    let after = handle
        .request(methods::VIEW_LOAD, json!({"view": "main"}), REQ)
        .await;
    assert!(
        matches!(after, Err(PluginError::Exited(_))),
        "requests fail after Error: {after:?}"
    );
    handle.stop().await;
}

#[tokio::test]
async fn slow_request_times_out() {
    let (_tmp, plugin) = install(granted_all());
    let handle = start(&plugin, secrets(&[]), json!({}), CancellationToken::new())
        .await
        .expect("start");
    let slow = handle
        .request(
            methods::VIEW_LOAD,
            json!({"view": "slow"}),
            Duration::from_millis(200),
        )
        .await;
    assert_eq!(slow, Err(PluginError::Timeout));
    handle.stop().await;
}

#[tokio::test]
async fn cancelling_the_token_stops_the_plugin() {
    let (_tmp, plugin) = install(granted_all());
    let cancel = CancellationToken::new();
    let handle = start(&plugin, secrets(&[]), json!({}), cancel.clone())
        .await
        .expect("start");
    cancel.cancel();
    wait_until(|| !handle.is_running(), "plugin stopped").await;
    assert_eq!(handle.state(), PluginState::Disabled);
    let after = handle
        .request(methods::VIEW_LOAD, json!({"view": "main"}), REQ)
        .await;
    assert!(matches!(after, Err(PluginError::Exited(_))), "{after:?}");
    handle.stop().await;
}

#[tokio::test]
async fn stop_sends_shutdown_before_the_child_exits() {
    let (tmp, plugin) = install(granted_all());
    let shutdowns = tmp.path().join("shutdown.log");
    let handle = start(
        &plugin,
        secrets(&[]),
        json!({"shutdown_log": shutdowns.to_str().expect("utf8 path")}),
        CancellationToken::new(),
    )
    .await
    .expect("start");
    handle.stop().await;
    assert_eq!(log_lines(&shutdowns), 1, "fixture saw shutdown");
}

#[tokio::test]
async fn badges_round_trip() {
    let (_tmp, plugin) = install(granted_all());
    let handle = start(&plugin, secrets(&[]), json!({}), CancellationToken::new())
        .await
        .expect("start");
    let badges = handle
        .request(
            methods::BADGES,
            json!({"projects": [{"id": "p1", "git_remote": "git@x:y"}]}),
            REQ,
        )
        .await
        .expect("badges");
    assert_eq!(badges[0]["id"], "p1");
    assert_eq!(badges[0]["text"], "Fixture: ok");
    handle.stop().await;
}

#[tokio::test]
async fn missing_executable_fails_start() {
    let (_tmp, mut plugin) = install(granted_all());
    plugin.manifest.executable = "no/such-binary".into();
    let started = start(&plugin, secrets(&[]), json!({}), CancellationToken::new()).await;
    match started {
        Err(PluginError::Exited(reason)) => {
            assert!(reason.contains("not found"), "{reason}");
        }
        Err(other) => panic!("expected Exited, got {other:?}"),
        Ok(handle) => {
            handle.stop().await;
            panic!("start must fail for a missing executable");
        }
    }
}

#[tokio::test]
async fn crash_on_initialize_fails_start() {
    let (_tmp, plugin) = install(granted_all());
    let started = start(
        &plugin,
        secrets(&[]),
        json!({"crash_on_init": true}),
        CancellationToken::new(),
    )
    .await;
    match started {
        Err(PluginError::Exited(_)) => {}
        Err(other) => panic!("expected Exited, got {other:?}"),
        Ok(handle) => {
            handle.stop().await;
            panic!("start must fail when initialize never answers");
        }
    }
}
