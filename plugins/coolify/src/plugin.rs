//! The plugin side of the Berth plugin protocol (PLAN §6.10): reads
//! JSON-RPC lines from the host, answers `initialize`, `view/load`,
//! `action/invoke`, `logs/open`, `logs/close`, `project/badges` and
//! `shutdown`, and asks the host for the API token with `host/secret.get`.
//!
//! [`serve`] is generic over its reader and writer so tests run the whole
//! protocol over in-memory pipes; `main.rs` hands it stdin and stdout.
//! Nothing but protocol lines is ever written to the writer. The token is
//! requested from the host the first time a client is needed, held in a
//! `SecretString`, and never put in a log line, a view or an error.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use berth_plugin_protocol::{
    decode, encode, methods, Message, Notification, Request, Response, RpcError, Tone, View,
    PROTOCOL_VERSION,
};
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::actions;
use crate::badges::{project_badges, ProjectRef};
use crate::client::{ClientError, CoolifyClient};
use crate::logs::{
    deployment_log_lines, failure_line, follow_application_logs, is_final_status, poll_deployment,
};
use crate::mcp::mcp_servers_payload;
use crate::types::Deployment;
use crate::views;

/// The manifest's secret key the API token lives under.
pub const TOKEN_KEY: &str = "coolify/token";

/// JSON-RPC codes this plugin answers with.
pub const ERR_FAILED: i64 = -32000;
pub const ERR_METHOD_NOT_FOUND: i64 = -32601;
pub const ERR_INVALID_PARAMS: i64 = -32602;

/// Failed deployments whose logs are read to say why, per view.
const MAX_FAILURE_LOOKUPS: usize = 5;
/// How many log lines one application-log poll asks Coolify for.
const APP_LOG_LINES: u32 = 200;

/// Intervals the plugin waits on; tests shrink them.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub deployment_poll: Duration,
    pub app_log_poll: Duration,
    /// How long to wait for the host to answer `host/secret.get`.
    pub secret_timeout: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            deployment_poll: Duration::from_secs(2),
            app_log_poll: Duration::from_secs(2),
            secret_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum Fail {
    #[error("Set the Coolify address in Plugins > Configure.")]
    NotConfigured,
    #[error("The Coolify API token is not available: {0}")]
    Token(String),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("This plugin has no view called {0}.")]
    UnknownView(String),
    #[error("{0}")]
    BadParams(String),
}

impl Fail {
    fn code(&self) -> i64 {
        match self {
            Fail::UnknownView(_) | Fail::BadParams(_) => ERR_INVALID_PARAMS,
            _ => ERR_FAILED,
        }
    }
}

#[derive(Debug, Default, Clone)]
struct Config {
    base_url: Option<String>,
    mcp_enabled: bool,
}

struct Shared {
    out: mpsc::UnboundedSender<Vec<u8>>,
    cancel: CancellationToken,
    timing: Timing,
    next_id: AtomicU64,
    waiting: Mutex<HashMap<u64, oneshot::Sender<Response>>>,
    config: Mutex<Config>,
    client: tokio::sync::Mutex<Option<Arc<CoolifyClient>>>,
    streams: Mutex<HashMap<String, CancellationToken>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn ok(id: u64, result: Value) -> Response {
    Response {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

fn error(id: u64, code: i64, message: String) -> Response {
    Response {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(RpcError { code, message }),
    }
}

fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, Fail> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Fail::BadParams(format!("missing {key}")))
}

impl Shared {
    fn send(&self, msg: &impl serde::Serialize) {
        let _ = self.out.send(encode(msg));
    }

    fn notify(&self, method: &str, params: Value) {
        self.send(&Notification {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        });
    }

    fn log(&self, level: &str, message: &str) {
        self.notify(methods::LOG, json!({ "level": level, "message": message }));
    }

    fn lines(&self, stream: &str, lines: Vec<String>, final_status: Option<&str>) {
        let mut params = json!({ "stream": stream, "lines": lines });
        if let Some(status) = final_status {
            params["final_status"] = json!(status);
        }
        self.notify(methods::LOG_LINES, params);
    }

    fn resolve(&self, response: Response) {
        if let Some(waiter) = lock(&self.waiting).remove(&response.id) {
            let _ = waiter.send(response);
        }
    }

    /// `host/secret.get`: the only way the token reaches this process.
    async fn secret(&self, key: &str) -> Result<SecretString, Fail> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        lock(&self.waiting).insert(id, tx);
        self.send(&Request {
            jsonrpc: "2.0".into(),
            id,
            method: methods::SECRET_GET.into(),
            params: json!({ "key": key }),
        });
        let answer = tokio::time::timeout(self.timing.secret_timeout, rx).await;
        lock(&self.waiting).remove(&id);
        let response = answer
            .map_err(|_| Fail::Token("the host did not answer".into()))?
            .map_err(|_| Fail::Token("the host went away".into()))?;
        if let Some(e) = response.error {
            return Err(Fail::Token(e.message));
        }
        response
            .result
            .as_ref()
            .and_then(|r| r.get("value"))
            .and_then(Value::as_str)
            .map(|value| SecretString::from(value.to_owned()))
            .ok_or_else(|| Fail::Token("the host sent no value".into()))
    }

    /// The client for the configured instance, built on first use (token
    /// from the host, version read once and warned about).
    async fn client(&self) -> Result<Arc<CoolifyClient>, Fail> {
        let mut slot = self.client.lock().await;
        if let Some(client) = slot.as_ref() {
            return Ok(Arc::clone(client));
        }
        let base_url = lock(&self.config)
            .base_url
            .clone()
            .ok_or(Fail::NotConfigured)?;
        let token = self.secret(TOKEN_KEY).await?;
        let client = Arc::new(CoolifyClient::new(&base_url, token)?);
        if let Ok(version) = client.version(&self.cancel).await {
            if let Some(warning) = &version.warning {
                self.log("warn", &format!("Coolify {}: {warning}", version.raw));
            }
        }
        *slot = Some(Arc::clone(&client));
        Ok(client)
    }

    /// A rejected token may have been replaced; forget the client so the
    /// next call asks the host again.
    async fn forget_if_rejected(&self, error: &Fail) {
        if matches!(error, Fail::Client(ClientError::TokenInvalid)) {
            *self.client.lock().await = None;
        }
    }

    async fn handle(self: &Arc<Self>, request: Request) -> Response {
        let id = request.id;
        let result = match request.method.as_str() {
            methods::INITIALIZE => self.initialize(&request.params).await,
            methods::VIEW_LOAD => self.view_load(&request.params).await,
            methods::ACTION_INVOKE => self.action_invoke(&request.params).await,
            methods::LOGS_OPEN => self.logs_open(&request.params).await,
            methods::LOGS_CLOSE => self.logs_close(&request.params),
            methods::BADGES => self.badges(&request.params).await,
            other => {
                return error(
                    id,
                    ERR_METHOD_NOT_FOUND,
                    format!("this plugin has no method {other:?}"),
                )
            }
        };
        match result {
            Ok(value) => ok(id, value),
            Err(fail) => {
                self.forget_if_rejected(&fail).await;
                error(id, fail.code(), fail.to_string())
            }
        }
    }

    async fn initialize(self: &Arc<Self>, params: &Value) -> Result<Value, Fail> {
        let protocol = params.get("protocol").and_then(Value::as_u64);
        if protocol != Some(u64::from(PROTOCOL_VERSION)) {
            return Err(Fail::BadParams(format!(
                "this plugin speaks protocol {PROTOCOL_VERSION}, the host asked for {protocol:?}"
            )));
        }
        let settings = params.get("config").cloned().unwrap_or(Value::Null);
        let config = Config {
            base_url: settings
                .get("base_url")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .map(str::to_owned),
            mcp_enabled: settings
                .get("mcp_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        let offer_mcp = config.mcp_enabled && config.base_url.is_some();
        *lock(&self.config) = config;
        *self.client.lock().await = None;
        if offer_mcp {
            let this = Arc::clone(self);
            tokio::spawn(async move { this.offer_mcp().await });
        }
        Ok(json!({ "capabilities": ["views", "actions", "logs", "badges", "mcp"] }))
    }

    /// Tells the host about Coolify's own MCP endpoint (`Mcp` permission).
    async fn offer_mcp(&self) {
        let Ok(client) = self.client().await else {
            return;
        };
        let Ok(token) = self.secret(TOKEN_KEY).await else {
            return;
        };
        if let Some(payload) = mcp_servers_payload(&client, &token, true) {
            self.notify(methods::MCP_SERVERS, payload);
        }
    }

    async fn view_load(&self, params: &Value) -> Result<Value, Fail> {
        let view = str_param(params, "view")?;
        let view = match self.build_view(view).await {
            Ok(view) => view,
            Err(fail @ (Fail::UnknownView(_) | Fail::BadParams(_))) => return Err(fail),
            Err(fail) => {
                self.forget_if_rejected(&fail).await;
                let tone = match fail {
                    Fail::Client(_) => Tone::Danger,
                    _ => Tone::Warning,
                };
                View::Message {
                    tone,
                    text: fail.to_string(),
                    action: None,
                }
            }
        };
        serde_json::to_value(view).map_err(|e| Fail::BadParams(e.to_string()))
    }

    /// The deployments table with each row's application joined in and, for
    /// the failed ones, why they failed (best effort: a lookup that fails
    /// only leaves a row with fewer details).
    async fn deployments_view(&self, client: &CoolifyClient, deployments: &[Deployment]) -> View {
        let applications = client.applications(&self.cancel).await.unwrap_or_default();
        let mut failures = BTreeMap::new();
        let failed = deployments
            .iter()
            .filter(|d| d.status.as_deref().is_some_and(|s| s.contains("fail")))
            .filter_map(|d| d.deployment_uuid.as_deref())
            .take(MAX_FAILURE_LOOKUPS);
        for uuid in failed {
            if let Some(reason) = client
                .deployment(uuid, &self.cancel)
                .await
                .ok()
                .as_ref()
                .and_then(failure_line)
            {
                failures.insert(uuid.to_owned(), reason);
            }
        }
        views::deployments_table(deployments, &applications, &failures)
    }

    async fn build_view(&self, view: &str) -> Result<View, Fail> {
        let parts: Vec<&str> = view.split('/').collect();
        let unknown = || Fail::UnknownView(view.to_owned());
        match parts.as_slice() {
            ["resources"] => {
                let client = self.client().await?;
                Ok(views::resources_table(
                    &client.resources(&self.cancel).await?,
                ))
            }
            ["servers"] => {
                let client = self.client().await?;
                Ok(views::servers_table(&client.servers(&self.cancel).await?))
            }
            ["deployments"] => {
                let client = self.client().await?;
                let deployments = client.deployments(&self.cancel).await?;
                Ok(self.deployments_view(&client, &deployments).await)
            }
            ["status"] => {
                let client = self.client().await?;
                Ok(views::version_message(&client.version(&self.cancel).await?))
            }
            ["deployments", uuid] => {
                let client = self.client().await?;
                let deployments = client
                    .application_deployments(uuid, None, Some(50), &self.cancel)
                    .await?;
                Ok(self.deployments_view(&client, &deployments).await)
            }
            ["application", uuid] => {
                let client = self.client().await?;
                let application = client.application(uuid, &self.cancel).await?;
                // The server is not on the application itself; its latest
                // deployment names it. Best effort: the confirm still names
                // the application and branch without it.
                let server = client
                    .application_deployments(uuid, None, Some(1), &self.cancel)
                    .await
                    .ok()
                    .and_then(|deployments| deployments.into_iter().find_map(|d| d.server_name));
                Ok(views::application_detail(&application, server.as_deref()))
            }
            ["application", uuid, "envs"] => {
                let client = self.client().await?;
                Ok(views::envs_table(&client.envs(uuid, &self.cancel).await?))
            }
            ["application", uuid, "logs"] => Ok(View::LogStream {
                stream: format!("application/{uuid}/logs"),
                title: "Application logs".to_owned(),
            }),
            ["deployment", uuid, "logs"] => Ok(View::LogStream {
                stream: format!("deployment/{uuid}/logs"),
                title: "Deployment log".to_owned(),
            }),
            _ => Err(unknown()),
        }
    }

    async fn action_invoke(&self, params: &Value) -> Result<Value, Fail> {
        let action = str_param(params, "action")?;
        let action_params = params.get("params").cloned().unwrap_or(Value::Null);
        let client = self.client().await?;
        let result = actions::invoke(&client, action, &action_params, &self.cancel).await?;
        serde_json::to_value(result).map_err(|e| Fail::BadParams(e.to_string()))
    }

    async fn logs_open(self: &Arc<Self>, params: &Value) -> Result<Value, Fail> {
        let stream = str_param(params, "stream")?.to_owned();
        let parts: Vec<&str> = stream.split('/').collect();
        let (kind, uuid) = match parts.as_slice() {
            [kind @ ("deployment" | "application"), uuid, "logs"] => (*kind, (*uuid).to_owned()),
            _ => return Err(Fail::BadParams(format!("unknown log stream {stream}"))),
        };
        let client = self.client().await?;
        let token = self.cancel.child_token();
        if let Some(previous) = lock(&self.streams).insert(stream.clone(), token.clone()) {
            previous.cancel();
        }
        let this = Arc::clone(self);
        if kind == "deployment" {
            tokio::spawn(async move { this.stream_deployment(client, stream, uuid, token).await });
        } else {
            tokio::spawn(async move { this.stream_application(client, stream, uuid, token).await });
        }
        Ok(json!({}))
    }

    fn logs_close(&self, params: &Value) -> Result<Value, Fail> {
        let stream = str_param(params, "stream")?;
        if let Some(token) = lock(&self.streams).remove(stream) {
            token.cancel();
        }
        Ok(json!({}))
    }

    async fn stream_deployment(
        &self,
        client: Arc<CoolifyClient>,
        stream: String,
        uuid: String,
        token: CancellationToken,
    ) {
        let mut sent = 0usize;
        let result = poll_deployment(
            &client,
            &uuid,
            self.timing.deployment_poll,
            &token,
            |deployment| {
                let lines = deployment_log_lines(deployment);
                let fresh = lines.get(sent..).unwrap_or_default().to_vec();
                sent = lines.len().max(sent);
                let status = deployment
                    .status
                    .as_deref()
                    .filter(|s| is_final_status(Some(s)));
                if !fresh.is_empty() || status.is_some() {
                    self.lines(&stream, fresh, status);
                }
            },
        )
        .await;
        self.finish_stream(&stream, result.err(), "Could not read the deployment");
    }

    async fn stream_application(
        &self,
        client: Arc<CoolifyClient>,
        stream: String,
        uuid: String,
        token: CancellationToken,
    ) {
        let end = follow_application_logs(
            &client,
            &uuid,
            APP_LOG_LINES,
            self.timing.app_log_poll,
            &token,
            |lines| self.lines(&stream, lines, None),
        )
        .await;
        self.finish_stream(&stream, Some(end), "Could not read the logs");
    }

    /// Ends a stream: a cancelled one ends quietly, a failed one tells the
    /// host why with a final `error` status.
    fn finish_stream(&self, stream: &str, end: Option<ClientError>, context: &str) {
        lock(&self.streams).remove(stream);
        if let Some(end) = end {
            if !matches!(end, ClientError::Cancelled) {
                self.lines(stream, vec![format!("{context}: {end}")], Some("error"));
            }
        }
    }

    async fn badges(&self, params: &Value) -> Result<Value, Fail> {
        let projects: Vec<ProjectRef> = params
            .get("projects")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        Some(ProjectRef {
                            id: entry.get("id")?.as_str()?.to_owned(),
                            git_remote: entry
                                .get("git_remote")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                        })
                    })
                    .collect()
            })
            .ok_or_else(|| Fail::BadParams("missing projects".into()))?;
        let client = match self.client().await {
            Ok(client) => client,
            Err(Fail::NotConfigured) => return Ok(json!([])),
            Err(other) => return Err(other),
        };
        let applications = client.applications(&self.cancel).await?;
        let badges: Vec<Value> = project_badges(&projects, &applications)
            .into_iter()
            .map(|badge| json!({ "id": badge.id, "text": badge.text, "tone": badge.tone }))
            .collect();
        Ok(Value::Array(badges))
    }
}

async fn write_loop<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
    cancel: CancellationToken,
) {
    loop {
        // Biased: everything queued before shutdown is written first.
        let bytes = tokio::select! {
            biased;
            queued = rx.recv() => match queued {
                Some(bytes) => bytes,
                None => break,
            },
            () = cancel.cancelled() => break,
        };
        if writer.write_all(&bytes).await.is_err() || writer.flush().await.is_err() {
            break;
        }
    }
}

/// Runs the plugin until the host sends `shutdown` or closes the reader.
pub async fn serve<R, W>(reader: R, writer: W, timing: Timing) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let cancel = CancellationToken::new();
    let (out, rx) = mpsc::unbounded_channel();
    let writer_task = tokio::spawn(write_loop(writer, rx, cancel.clone()));
    let shared = Arc::new(Shared {
        out,
        cancel: cancel.clone(),
        timing,
        next_id: AtomicU64::new(1),
        waiting: Mutex::new(HashMap::new()),
        config: Mutex::new(Config::default()),
        client: tokio::sync::Mutex::new(None),
        streams: Mutex::new(HashMap::new()),
    });

    let mut lines = reader.lines();
    let mut outcome = Ok(());
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                outcome = Err(e);
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match decode(line.as_bytes()) {
            Ok(Message::Request(request)) if request.method == methods::SHUTDOWN => {
                shared.send(&ok(request.id, json!({})));
                break;
            }
            Ok(Message::Request(request)) => {
                let shared = Arc::clone(&shared);
                tokio::spawn(async move {
                    let response = shared.handle(request).await;
                    shared.send(&response);
                });
            }
            Ok(Message::Response(response)) => shared.resolve(response),
            Ok(Message::Notification(_)) => {}
            Err(e) => shared.log(
                "warn",
                &format!("ignored a line that is not a message: {e}"),
            ),
        }
    }
    cancel.cancel();
    drop(shared);
    let _ = tokio::time::timeout(Duration::from_secs(1), writer_task).await;
    outcome
}
