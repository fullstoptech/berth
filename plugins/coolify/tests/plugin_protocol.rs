//! The plugin side of the protocol, driven the way Berth's host drives it:
//! JSON-RPC lines over in-memory pipes, Coolify played by wiremock. One test
//! (`plugin_binary_*`) runs the real `berth-coolify` executable over stdio.

#![allow(clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use berth_coolify::plugin::{serve, Timing, TOKEN_KEY};
use berth_plugin_protocol::{decode, encode, methods, Message, Notification, Request, Response};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};
use tokio::task::JoinHandle;
use wiremock::matchers::{bearer_token, method, path, query_param};
use wiremock::{Mock, MockServer, Request as HttpRequest, Respond, ResponseTemplate};

const TOKEN: &str = "plugin-test-token-not-real";
const WAIT: Duration = Duration::from_secs(5);

const RESOURCES: &str = include_str!("../../../fixtures/coolify/resources.json");
const APPLICATION: &str = include_str!("../../../fixtures/coolify/application.json");
const APP_DEPLOYMENTS: &str = include_str!("../../../fixtures/coolify/app-deployments.json");
const DEPLOYMENT_FAILED: &str = include_str!("../../../fixtures/coolify/deployment-failed.json");

fn fast() -> Timing {
    Timing {
        deployment_poll: Duration::from_millis(5),
        app_log_poll: Duration::from_millis(5),
        secret_timeout: Duration::from_secs(2),
    }
}

/// The host end of the pipes.
struct Host {
    to_plugin: DuplexStream,
    from_plugin: Lines<BufReader<DuplexStream>>,
    next_id: u64,
    /// `None` makes the host answer `host/secret.get` with an error.
    token: Option<String>,
    secret_requests: Vec<String>,
    notifications: Vec<Notification>,
    serve: JoinHandle<std::io::Result<()>>,
}

impl Host {
    fn start(token: Option<&str>, timing: Timing) -> Self {
        let (to_plugin, plugin_in) = tokio::io::duplex(1 << 16);
        let (plugin_out, from_plugin) = tokio::io::duplex(1 << 16);
        let serve = tokio::spawn(serve(BufReader::new(plugin_in), plugin_out, timing));
        Self {
            to_plugin,
            from_plugin: BufReader::new(from_plugin).lines(),
            next_id: 1,
            token: token.map(str::to_owned),
            secret_requests: Vec::new(),
            notifications: Vec::new(),
            serve,
        }
    }

    async fn send(&mut self, message: &impl serde::Serialize) {
        self.to_plugin
            .write_all(&encode(message))
            .await
            .expect("plugin is listening");
    }

    async fn request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&Request {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        })
        .await;
        id
    }

    /// Next message from the plugin; `host/secret.get` requests are
    /// answered here, the way the real host answers them.
    async fn next_message(&mut self) -> Message {
        loop {
            let line = tokio::time::timeout(WAIT, self.from_plugin.next_line())
                .await
                .expect("the plugin says something in time")
                .expect("pipe reads")
                .expect("the plugin has not closed its output");
            let message = decode(line.as_bytes()).expect("the plugin writes protocol lines only");
            if let Message::Request(request) = &message {
                assert_eq!(request.method, methods::SECRET_GET);
                let key = request.params["key"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                self.secret_requests.push(key);
                let answer = match &self.token {
                    Some(token) => Response {
                        jsonrpc: "2.0".into(),
                        id: request.id,
                        result: Some(json!({ "value": token })),
                        error: None,
                    },
                    None => Response {
                        jsonrpc: "2.0".into(),
                        id: request.id,
                        result: None,
                        error: Some(berth_plugin_protocol::RpcError {
                            code: -32002,
                            message: "secret \"coolify/token\" is not set".into(),
                        }),
                    },
                };
                self.send(&answer).await;
                continue;
            }
            return message;
        }
    }

    async fn call(&mut self, method: &str, params: Value) -> Response {
        let id = self.request(method, params).await;
        loop {
            match self.next_message().await {
                Message::Response(response) if response.id == id => return response,
                Message::Notification(notification) => self.notifications.push(notification),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    /// The result of a call that must succeed.
    async fn result(&mut self, method: &str, params: Value) -> Value {
        let response = self.call(method, params).await;
        assert_eq!(response.error, None, "{method} failed");
        response.result.expect("a result")
    }

    async fn initialize(&mut self, config: Value) {
        let result = self
            .result(
                methods::INITIALIZE,
                json!({ "protocol": 1, "host_version": "test", "config": config }),
            )
            .await;
        assert!(result["capabilities"].is_array());
    }

    async fn view(&mut self, view: &str) -> Value {
        self.result(methods::VIEW_LOAD, json!({ "view": view, "params": {} }))
            .await
    }

    /// Next notification with this method, taking earlier ones into account.
    async fn notification(&mut self, wanted: &str) -> Notification {
        loop {
            if let Some(pos) = self.notifications.iter().position(|n| n.method == wanted) {
                return self.notifications.remove(pos);
            }
            match self.next_message().await {
                Message::Notification(n) => self.notifications.push(n),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    async fn shutdown(mut self) {
        let result = self.result(methods::SHUTDOWN, json!({})).await;
        assert_eq!(result, json!({}));
        let end = tokio::time::timeout(WAIT, self.serve)
            .await
            .expect("serve returns after shutdown")
            .expect("task joins");
        end.expect("serve ends cleanly");
    }
}

fn json_body(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_string(body)
}

async fn get(server: &MockServer, route: &str, body: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/{route}")))
        .and(bearer_token(TOKEN))
        .respond_with(json_body(body))
        .mount(server)
        .await;
}

/// Answers with the bodies in order, repeating the last.
struct Sequence {
    bodies: Vec<String>,
    next: Arc<AtomicUsize>,
}

impl Respond for Sequence {
    fn respond(&self, _request: &HttpRequest) -> ResponseTemplate {
        let at = self.next.fetch_add(1, Ordering::SeqCst);
        json_body(&self.bodies[at.min(self.bodies.len() - 1)])
    }
}

fn config(server: &MockServer) -> Value {
    json!({ "base_url": server.uri() })
}

fn log_text(host: &Host) -> String {
    host.notifications
        .iter()
        .filter(|n| n.method == methods::LOG)
        .map(|n| n.params.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn plugin_initialize_answers_capabilities_and_rejects_another_protocol() {
    let mut host = Host::start(Some(TOKEN), fast());
    let bad = host
        .call(
            methods::INITIALIZE,
            json!({ "protocol": 2, "host_version": "t", "config": {} }),
        )
        .await;
    assert!(bad.error.expect("an error").message.contains("protocol"));
    host.initialize(json!({})).await;
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_view_load_resources_asks_the_host_for_the_token_and_sends_it_as_bearer() {
    let server = MockServer::start().await;
    get(&server, "resources", RESOURCES).await;
    get(&server, "version", "v4.5.0").await;
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(config(&server)).await;
    let view = host.view("resources").await;
    assert_eq!(view["kind"], "table");
    assert_eq!(view["rows"].as_array().map(Vec::len), Some(3));
    // The wiremock route only answers a request carrying the bearer token,
    // so a 3-row table proves the host-supplied secret reached the API.
    assert_eq!(host.secret_requests, [TOKEN_KEY]);
    // A second view reuses the client: the host is asked once.
    host.view("resources").await;
    assert_eq!(host.secret_requests.len(), 1);
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_view_load_without_an_address_is_a_message_not_a_crash() {
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(json!({})).await;
    let view = host.view("resources").await;
    assert_eq!(view["kind"], "message");
    assert_eq!(view["tone"], "warning");
    assert!(view["text"]
        .as_str()
        .unwrap_or_default()
        .contains("Configure"));
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_missing_secret_is_a_message_and_never_echoes_a_token() {
    let server = MockServer::start().await;
    let mut host = Host::start(None, fast());
    host.initialize(config(&server)).await;
    let view = host.view("servers").await;
    assert_eq!(view["kind"], "message");
    assert!(view["text"].as_str().unwrap_or_default().contains("token"));
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_token_invalid_shows_a_danger_message_and_asks_the_host_again_next_time() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/servers"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(config(&server)).await;
    for expected_asks in [1, 2] {
        let view = host.view("servers").await;
        assert_eq!(view["kind"], "message");
        assert_eq!(view["tone"], "danger");
        assert_eq!(view["text"], "token invalid");
        assert_eq!(host.secret_requests.len(), expected_asks);
    }
    assert!(!log_text(&host).contains(TOKEN));
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_unreachable_instance_is_a_danger_message() {
    // A port nothing listens on: bind one, note it, close it.
    let dead = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
        format!("http://{}", listener.local_addr().expect("addr"))
    };
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(json!({ "base_url": dead })).await;
    let view = host.view("resources").await;
    assert_eq!(view["tone"], "danger");
    assert!(
        view["text"]
            .as_str()
            .unwrap_or_default()
            .contains("unreachable"),
        "{view}"
    );
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_unknown_view_and_method_are_errors() {
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(json!({})).await;
    let unknown = host
        .call(methods::VIEW_LOAD, json!({ "view": "nope", "params": {} }))
        .await;
    assert_eq!(unknown.error.expect("error").code, -32602);
    let method = host.call("wat/ever", json!({})).await;
    assert_eq!(method.error.expect("error").code, -32601);
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_application_detail_confirm_names_app_server_and_branch() {
    let server = MockServer::start().await;
    get(&server, "applications/app-uuid-0001", APPLICATION).await;
    get(
        &server,
        "deployments/applications/app-uuid-0001",
        APP_DEPLOYMENTS,
    )
    .await;
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(config(&server)).await;
    let view = host.view("application/app-uuid-0001").await;
    assert_eq!(view["kind"], "detail");
    let deploy = view["actions"]
        .as_array()
        .and_then(|a| a.iter().find(|a| a["id"] == "deploy"))
        .expect("a deploy action");
    let lines = deploy["confirm"]["lines"].to_string();
    assert!(lines.contains("landing"), "{lines}");
    assert!(lines.contains("hetzner-1"), "{lines}");
    assert!(lines.contains("main"), "{lines}");
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_action_invoke_deploy_with_force_sends_force_true() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/deploy"))
        .and(query_param("uuid", "app-uuid-0001"))
        .and(query_param("force", "true"))
        .and(bearer_token(TOKEN))
        .respond_with(json_body(
            r#"{"deployments":[{"message":"queued","resource_uuid":"app-uuid-0001","deployment_uuid":"dep-uuid-0002"}]}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;
    get(&server, "version", "v4.5.0").await;
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(config(&server)).await;
    let result = host
        .result(
            methods::ACTION_INVOKE,
            json!({ "action": "deploy", "params": { "uuid": "app-uuid-0001", "kind": "application", "force": true } }),
        )
        .await;
    assert_eq!(result["open"], "deployment/dep-uuid-0002/logs");
    host.shutdown().await;
    // `expect(1)` is checked when the server drops.
}

#[tokio::test]
async fn plugin_action_invoke_failure_is_an_rpc_error_with_the_reason() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/applications/app-uuid-0001/stop"))
        .respond_with(ResponseTemplate::new(500).set_body_string("proxy is down"))
        .mount(&server)
        .await;
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(config(&server)).await;
    let response = host
        .call(
            methods::ACTION_INVOKE,
            json!({ "action": "stop", "params": { "uuid": "app-uuid-0001", "kind": "application" } }),
        )
        .await;
    let error = response.error.expect("an error");
    assert_eq!(error.code, -32000);
    assert!(error.message.contains("proxy is down"), "{}", error.message);
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_logs_open_streams_a_failed_deployment_to_its_failing_step() {
    let server = MockServer::start().await;
    let in_progress = DEPLOYMENT_FAILED
        .replace("\"status\": \"failed\"", "\"status\": \"in_progress\"")
        .replace(
            "\"logs\": \"[",
            "\"logs\": \"[{\\\"output\\\":\\\"Starting deployment of landing to hetzner-1.\\\"}",
        );
    Mock::given(method("GET"))
        .and(path("/api/v1/deployments/dep-uuid-00ff"))
        .respond_with(Sequence {
            bodies: vec![in_progress, DEPLOYMENT_FAILED.to_owned()],
            next: Arc::new(AtomicUsize::new(0)),
        })
        .mount(&server)
        .await;
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(config(&server)).await;
    let stream = "deployment/dep-uuid-00ff/logs";
    let view = host.view(stream).await;
    assert_eq!(view["kind"], "log_stream");
    assert_eq!(view["stream"], stream);
    host.result(
        methods::LOGS_OPEN,
        json!({ "stream": stream, "params": {} }),
    )
    .await;
    let mut all = Vec::new();
    let final_status = loop {
        let n = host.notification(methods::LOG_LINES).await;
        assert_eq!(n.params["stream"], stream);
        for line in n.params["lines"].as_array().expect("lines") {
            all.push(line.as_str().expect("a string").to_owned());
        }
        if let Some(status) = n.params.get("final_status") {
            break status.clone();
        }
    };
    assert_eq!(final_status, "failed");
    assert!(
        all.last()
            .is_some_and(|l| l.contains("build step exited with code 1")),
        "the stream ends on the failing step: {all:?}"
    );
    assert!(all.iter().any(|l| l.contains("Missing script")), "{all:?}");
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_logs_close_stops_the_application_log_poller() {
    let server = MockServer::start().await;
    get(
        &server,
        "applications/app-uuid-0001/logs",
        r#"{"logs":"one\ntwo\n"}"#,
    )
    .await;
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(config(&server)).await;
    let stream = "application/app-uuid-0001/logs";
    host.result(
        methods::LOGS_OPEN,
        json!({ "stream": stream, "params": {} }),
    )
    .await;
    let first = host.notification(methods::LOG_LINES).await;
    assert_eq!(first.params["lines"], json!(["one", "two"]));
    host.result(methods::LOGS_CLOSE, json!({ "stream": stream }))
        .await;
    // Let a poll that was already in flight land, then count requests.
    tokio::time::sleep(Duration::from_millis(60)).await;
    let before = server
        .received_requests()
        .await
        .expect("recording on")
        .len();
    tokio::time::sleep(Duration::from_millis(80)).await;
    let after = server
        .received_requests()
        .await
        .expect("recording on")
        .len();
    assert_eq!(before, after, "no polling after logs/close");
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_project_badges_link_by_git_remote() {
    let server = MockServer::start().await;
    get(&server, "applications", &format!("[{APPLICATION}]")).await;
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(config(&server)).await;
    let badges = host
        .result(
            methods::BADGES,
            json!({ "projects": [
                { "id": "p1", "git_remote": "git@github.com:Example/Landing.git" },
                { "id": "p2", "git_remote": "https://github.com/other/thing" },
                { "id": "p3", "git_remote": null },
            ] }),
        )
        .await;
    assert_eq!(
        badges,
        json!([{ "id": "p1", "text": "Deployed on Coolify: running", "tone": "success" }])
    );
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_project_badges_are_empty_until_configured() {
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(json!({})).await;
    let badges = host
        .result(
            methods::BADGES,
            json!({ "projects": [{ "id": "p1", "git_remote": "example/landing" }] }),
        )
        .await;
    assert_eq!(badges, json!([]));
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_newer_coolify_major_logs_the_built_for_warning() {
    let server = MockServer::start().await;
    get(&server, "version", "5.0.0").await;
    get(&server, "servers", "[]").await;
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(config(&server)).await;
    host.view("servers").await;
    let log = host.notification(methods::LOG).await;
    assert_eq!(log.params["level"], "warn");
    assert!(
        log.params["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Built for Coolify 4.5; some actions may fail"),
        "{log:?}"
    );
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_mcp_servers_are_offered_only_when_enabled() {
    let server = MockServer::start().await;
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(json!({ "base_url": server.uri(), "mcp_enabled": true }))
        .await;
    let offer = host.notification(methods::MCP_SERVERS).await;
    assert_eq!(offer.params[0]["name"], "coolify");
    assert_eq!(offer.params[0]["url"], format!("{}/mcp", server.uri()));
    assert_eq!(
        offer.params[0]["headers"]["Authorization"],
        format!("Bearer {TOKEN}")
    );
    host.shutdown().await;

    let mut off = Host::start(Some(TOKEN), fast());
    off.initialize(json!({ "base_url": server.uri() })).await;
    // A round trip afterwards gives an unwanted offer time to arrive.
    off.call(methods::VIEW_LOAD, json!({ "view": "nope" }))
        .await;
    assert!(off
        .notifications
        .iter()
        .all(|n| n.method != methods::MCP_SERVERS));
    off.shutdown().await;
}

#[tokio::test]
async fn plugin_secrets_never_appear_in_any_plugin_output() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(config(&server)).await;
    let mut seen = String::new();
    for view in ["resources", "servers", "deployments", "application/x"] {
        seen.push_str(&host.view(view).await.to_string());
    }
    seen.push_str(&log_text(&host));
    assert!(!seen.contains(TOKEN), "{seen}");
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_closing_the_input_ends_serve() {
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(json!({})).await;
    drop(host.to_plugin);
    let end = tokio::time::timeout(WAIT, host.serve)
        .await
        .expect("serve ends when the host goes away")
        .expect("joins");
    end.expect("clean end");
}

#[tokio::test]
async fn plugin_every_sidebar_view_in_the_manifest_loads_and_the_token_key_matches() {
    let manifest: berth_plugin_protocol::Manifest =
        serde_json::from_str(include_str!("../berth-plugin.json")).expect("manifest parses");
    assert!(manifest.secrets.iter().any(|s| s.key == TOKEN_KEY));
    assert!(manifest.permissions.iter().any(|p| matches!(
        p,
        berth_plugin_protocol::Permission::Secrets { keys } if keys.iter().any(|k| k == TOKEN_KEY)
    )));
    assert!(manifest
        .config
        .iter()
        .any(|c| c.key == "base_url" && c.required));

    let server = MockServer::start().await;
    get(&server, "resources", RESOURCES).await;
    get(
        &server,
        "servers",
        include_str!("../../../fixtures/coolify/servers.json"),
    )
    .await;
    get(
        &server,
        "deployments",
        include_str!("../../../fixtures/coolify/deployments.json"),
    )
    .await;
    get(&server, "version", "v4.5.0").await;
    let mut host = Host::start(Some(TOKEN), fast());
    host.initialize(config(&server)).await;
    let mut loaded = 0;
    for contribution in &manifest.contributes {
        if let berth_plugin_protocol::Contribution::SidebarSection { items, .. } = contribution {
            for item in items {
                let view = host.view(&item.view).await;
                assert_eq!(view["kind"], "table", "{} is a table", item.view);
                loaded += 1;
            }
        }
    }
    assert_eq!(loaded, 3);
    host.shutdown().await;
}

#[tokio::test]
async fn plugin_binary_speaks_the_protocol_over_real_stdio() {
    use std::process::Stdio;
    use tokio::process::Command;

    let server = MockServer::start().await;
    get(
        &server,
        "servers",
        include_str!("../../../fixtures/coolify/servers.json"),
    )
    .await;
    let mut child = Command::new(env!("CARGO_BIN_EXE_berth-coolify"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("binary starts");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout")).lines();

    async fn ask(stdin: &mut tokio::process::ChildStdin, id: u64, method: &str, params: Value) {
        let request = Request {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        };
        stdin.write_all(&encode(&request)).await.expect("writes");
    }
    async fn next(lines: &mut Lines<BufReader<tokio::process::ChildStdout>>) -> Message {
        let line = tokio::time::timeout(WAIT, lines.next_line())
            .await
            .expect("in time")
            .expect("reads")
            .expect("not closed");
        decode(line.as_bytes()).expect("a protocol line")
    }

    ask(
        &mut stdin,
        1,
        methods::INITIALIZE,
        json!({ "protocol": 1, "host_version": "t", "config": { "base_url": server.uri() } }),
    )
    .await;
    assert!(
        matches!(next(&mut stdout).await, Message::Response(r) if r.id == 1 && r.error.is_none())
    );

    ask(
        &mut stdin,
        2,
        methods::VIEW_LOAD,
        json!({ "view": "servers", "params": {} }),
    )
    .await;
    // The plugin asks for its token before it can call Coolify.
    let Message::Request(secret) = next(&mut stdout).await else {
        panic!("expected host/secret.get");
    };
    assert_eq!(secret.method, methods::SECRET_GET);
    assert_eq!(secret.params["key"], TOKEN_KEY);
    let answer = Response {
        jsonrpc: "2.0".into(),
        id: secret.id,
        result: Some(json!({ "value": TOKEN })),
        error: None,
    };
    stdin.write_all(&encode(&answer)).await.expect("writes");
    let view = loop {
        match next(&mut stdout).await {
            Message::Response(r) if r.id == 2 => break r.result.expect("a view"),
            _ => {}
        }
    };
    assert_eq!(view["kind"], "table");
    assert_eq!(view["rows"].as_array().map(Vec::len), Some(2));

    ask(&mut stdin, 3, methods::SHUTDOWN, json!({})).await;
    assert!(matches!(next(&mut stdout).await, Message::Response(r) if r.id == 3));
    let status = tokio::time::timeout(WAIT, child.wait())
        .await
        .expect("exits after shutdown")
        .expect("status");
    assert!(status.success());
}
