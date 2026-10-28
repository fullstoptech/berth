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
//!
//! What the instance supports is learned as the plugin uses it (`probe`):
//! the version is read again when a view loads after `version_ttl` or after
//! a run of errors, and an endpoint or action that turns out to be missing
//! loses its buttons and tabs. Either change sends `view/invalidate`, so the
//! host reloads the status banner and the views it shows.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

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
use crate::probe::{Feature, Learned, Probe, VersionChange};
use crate::types::{Application, Deployment, VersionInfo};
use crate::views;

/// The manifest's secret key the API token lives under.
pub const TOKEN_KEY: &str = "coolify/token";
/// The hidden view the host shows as the plugin's banner.
pub const STATUS_VIEW: &str = "status";

/// JSON-RPC codes this plugin answers with.
pub const ERR_FAILED: i64 = -32000;
pub const ERR_METHOD_NOT_FOUND: i64 = -32601;
pub const ERR_INVALID_PARAMS: i64 = -32602;

/// Failed deployments whose logs are read to say why, per view.
const MAX_FAILURE_LOOKUPS: usize = 5;
/// History rows asked of each application for the Deployments view.
const HISTORY_PER_APP: u32 = 10;
/// Rows the Deployments view keeps, newest first (one screen of recent work).
const RECENT_DEPLOYMENTS: usize = 20;
/// History requests in flight at once.
const HISTORY_CONCURRENCY: usize = 6;
/// How many log lines one application-log poll asks Coolify for.
const APP_LOG_LINES: u32 = 200;

/// Intervals the plugin waits on; tests shrink them.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub deployment_poll: Duration,
    pub app_log_poll: Duration,
    /// How long to wait for the host to answer `host/secret.get`.
    pub secret_timeout: Duration,
    /// How long a version read stands before a view load reads it again.
    pub version_ttl: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            deployment_poll: Duration::from_secs(2),
            app_log_poll: Duration::from_secs(2),
            secret_timeout: Duration::from_secs(10),
            version_ttl: Duration::from_secs(5),
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
    /// The feature is missing on this instance; the text says which and why.
    #[error("{0}")]
    Unsupported(String),
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
    probe: Mutex<Probe>,
    /// One version read at a time; the others wait and use its answer.
    version_read: tokio::sync::Mutex<()>,
}

/// What a probed call was asked about, for telling a missing endpoint from
/// a missing resource when it answers 404.
#[derive(Debug, Clone, Copy)]
enum Subject<'a> {
    /// A list endpoint, or a resource the plugin has just read.
    Known,
    /// A resource by uuid: it exists when `GET /resources` lists it.
    Resource(&'a str),
    /// A deployment by uuid: it exists when `GET /deployments/{uuid}` reads.
    Deployment(&'a str),
}

/// Fills in what a history row may omit from the application it was listed
/// under, so the table can name and link it.
fn with_application(mut deployment: Deployment, application: Option<&Application>) -> Deployment {
    if let Some(application) = application {
        deployment.application_id = deployment.application_id.or(application.id);
        if deployment.application_name.is_none() {
            deployment.application_name.clone_from(&application.name);
        }
    }
    deployment
}

/// The running deployments and every application's history as one list,
/// newest first, each deployment once (the running list wins), at most `cap`.
fn merge_recent(
    running: Vec<Deployment>,
    histories: Vec<Vec<Deployment>>,
    cap: usize,
) -> Vec<Deployment> {
    let mut seen = std::collections::HashSet::new();
    let mut merged: Vec<Deployment> = running
        .into_iter()
        .chain(histories.into_iter().flatten())
        .filter(|d| match d.deployment_uuid.as_ref() {
            Some(uuid) => seen.insert(uuid.clone()),
            None => true,
        })
        .collect();
    let started = |d: &Deployment| {
        d.created_at
            .as_deref()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
    };
    // Unreadable times sort last; ids break ties (a later row has a larger id).
    merged.sort_by(|a, b| {
        started(b)
            .cmp(&started(a))
            .then_with(|| b.id.cmp(&a.id))
            .then_with(|| a.deployment_uuid.cmp(&b.deployment_uuid))
    });
    merged.truncate(cap);
    merged
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

    /// The client for the configured instance, built on first use with the
    /// token from the host.
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
        *slot = Some(Arc::clone(&client));
        Ok(client)
    }

    /// The client, with the version read again when the last read is older
    /// than `version_ttl` (what every view load starts with).
    async fn connected(&self) -> Result<Arc<CoolifyClient>, Fail> {
        let client = self.client().await?;
        // A version that cannot be read does not keep the view from loading.
        let _ = self.check_version(&client, false).await;
        Ok(client)
    }

    /// Reads the version when it is due, or now when `force`. A newer major
    /// is warned about the first time it is seen; a different version than
    /// before forgets what was learned and tells the host to reload, so the
    /// banner follows an upgrade without a plugin restart. A failed read
    /// keeps the last version known.
    async fn check_version(
        &self,
        client: &CoolifyClient,
        force: bool,
    ) -> Result<VersionInfo, ClientError> {
        let _one_at_a_time = self.version_read.lock().await;
        let due = lock(&self.probe).version_due(Instant::now(), self.timing.version_ttl);
        if !force && !due {
            if let Some(known) = lock(&self.probe).version() {
                return Ok(known.clone());
            }
        }
        let read = client.version(&self.cancel).await;
        let change = lock(&self.probe).set_version(read.as_ref().ok().cloned(), Instant::now());
        if let Ok(version) = &read {
            if change != VersionChange::Same {
                if let Some(warning) = &version.warning {
                    self.log("warn", &format!("Coolify {}: {warning}", version.raw));
                }
            }
            if change == VersionChange::Changed {
                self.notify(methods::INVALIDATE, json!({ "view": STATUS_VIEW }));
            }
        }
        read
    }

    /// Runs one call of `feature`. One already known to be missing here is
    /// not asked; a success records that it works; an error may show it is
    /// missing (see [`Shared::learn`]), which fails with the reason.
    async fn probed<T>(
        &self,
        client: &CoolifyClient,
        feature: Feature,
        subject: Subject<'_>,
        call: impl Future<Output = Result<T, ClientError>>,
    ) -> Result<T, Fail> {
        if let Some(reason) = lock(&self.probe).unsupported_text(feature) {
            return Err(Fail::Unsupported(reason));
        }
        match call.await {
            Ok(value) => {
                lock(&self.probe).succeeded(feature);
                Ok(value)
            }
            Err(error) => match self.learn(client, feature, subject, &error).await {
                Some(reason) => Err(Fail::Unsupported(reason)),
                None => Err(Fail::Client(error)),
            },
        }
    }

    /// What a failed call of `feature` teaches: a run of errors from
    /// features that worked reads the version at once; a 404 from one that
    /// has not worked here marks it missing when its subject exists; an
    /// answer without the fields Berth reads always does. A new mark tells
    /// the host to reload. Returns the reason when the feature is missing.
    async fn learn(
        &self,
        client: &CoolifyClient,
        feature: Feature,
        subject: Subject<'_>,
        error: &ClientError,
    ) -> Option<String> {
        let mut learned = lock(&self.probe).failed(feature, error);
        if learned == Learned::Recheck {
            let _ = self.check_version(client, true).await;
            learned = lock(&self.probe).classify(feature, error);
        }
        let missing = match learned {
            Learned::Unsupported => true,
            Learned::Missing => self.exists(client, subject).await,
            Learned::Nothing | Learned::Recheck => false,
        };
        if !missing {
            return None;
        }
        let (fresh, reason) = {
            let mut probe = lock(&self.probe);
            let fresh = probe.mark(feature, error);
            (fresh, probe.unsupported_text(feature))
        };
        if fresh {
            if let Some(reason) = &reason {
                self.log("warn", reason);
            }
            self.notify(methods::INVALIDATE, json!({ "view": STATUS_VIEW }));
        }
        reason
    }

    /// Whether the resource a 404 was about exists, which makes the 404 the
    /// endpoint's.
    async fn exists(&self, client: &CoolifyClient, subject: Subject<'_>) -> bool {
        match subject {
            Subject::Known => true,
            Subject::Resource(uuid) => client
                .resources(&self.cancel)
                .await
                .is_ok_and(|all| all.iter().any(|r| r.uuid.as_deref() == Some(uuid))),
            Subject::Deployment(uuid) => client.deployment(uuid, &self.cancel).await.is_ok(),
        }
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
        *lock(&self.probe) = Probe::default();
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
        let mut view = match self.build_view(view).await {
            Ok(view) => view,
            Err(fail @ (Fail::UnknownView(_) | Fail::BadParams(_))) => return Err(fail),
            Err(Fail::Unsupported(reason)) => View::Message {
                tone: Tone::Warning,
                text: format!("{reason} The other Coolify views still work."),
                action: None,
            },
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
        lock(&self.probe).prune(&mut view);
        serde_json::to_value(view).map_err(|e| Fail::BadParams(e.to_string()))
    }

    /// The deployments table with each row's application joined in and, for
    /// the failed ones, why they failed (best effort: a lookup that fails
    /// only leaves a row with fewer details).
    async fn deployments_view(
        &self,
        client: &CoolifyClient,
        deployments: &[Deployment],
        applications: &[Application],
    ) -> View {
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
        views::deployments_table(deployments, applications, &failures)
    }

    /// The recent deployments across every application. `GET /deployments`
    /// lists only what is running now, so the history comes from each
    /// application's own list (`GET /deployments/applications/{uuid}`), asked
    /// a few at a time. An application whose history cannot be read is left
    /// out and noted in the log; the rest still show. When no application's
    /// history reads, what the first error teaches is learned (an instance
    /// without the endpoint shows the running deployments only).
    async fn recent_deployments(
        &self,
        client: &Arc<CoolifyClient>,
        applications: &[Application],
        running: Vec<Deployment>,
    ) -> Vec<Deployment> {
        if lock(&self.probe)
            .unsupported_text(Feature::History)
            .is_some()
        {
            return merge_recent(running, Vec::new(), RECENT_DEPLOYMENTS);
        }
        let gate = Arc::new(tokio::sync::Semaphore::new(HISTORY_CONCURRENCY));
        let mut lookups = tokio::task::JoinSet::new();
        for application in applications {
            let Some(uuid) = application.uuid.clone() else {
                continue;
            };
            let (client, gate, cancel) =
                (Arc::clone(client), Arc::clone(&gate), self.cancel.clone());
            lookups.spawn(async move {
                let _slot = gate.acquire().await;
                let history = client
                    .application_deployments(&uuid, None, Some(HISTORY_PER_APP), &cancel)
                    .await;
                (uuid, history)
            });
        }
        let mut histories = Vec::new();
        let mut failed = 0_usize;
        let mut first_error = None;
        while let Some(joined) = lookups.join_next().await {
            match joined {
                Ok((uuid, Ok(history))) => {
                    let application = applications
                        .iter()
                        .find(|a| a.uuid.as_deref() == Some(uuid.as_str()));
                    histories.push(
                        history
                            .into_iter()
                            .map(|d| with_application(d, application))
                            .collect::<Vec<_>>(),
                    );
                }
                Ok((_, Err(error))) => {
                    failed += 1;
                    first_error.get_or_insert(error);
                }
                Err(_) => failed += 1,
            }
        }
        if !histories.is_empty() {
            lock(&self.probe).succeeded(Feature::History);
        } else if let Some(error) = first_error {
            // The applications came from the list: they exist.
            self.learn(client, Feature::History, Subject::Known, &error)
                .await;
        }
        if failed > 0 {
            self.log(
                "warn",
                &format!("Deployment history could not be read for {failed} application(s)."),
            );
        }
        merge_recent(running, histories, RECENT_DEPLOYMENTS)
    }

    async fn build_view(&self, view: &str) -> Result<View, Fail> {
        let parts: Vec<&str> = view.split('/').collect();
        let unknown = || Fail::UnknownView(view.to_owned());
        match parts.as_slice() {
            ["resources"] => {
                let client = self.connected().await?;
                let resources = self
                    .probed(
                        &client,
                        Feature::Resources,
                        Subject::Known,
                        client.resources(&self.cancel),
                    )
                    .await?;
                Ok(views::resources_table(&resources))
            }
            ["servers"] => {
                let client = self.connected().await?;
                let servers = self
                    .probed(
                        &client,
                        Feature::Servers,
                        Subject::Known,
                        client.servers(&self.cancel),
                    )
                    .await?;
                Ok(views::servers_table(&servers))
            }
            ["deployments"] => {
                let client = self.connected().await?;
                // Without the running list the history still shows.
                let running = match self
                    .probed(
                        &client,
                        Feature::RunningDeployments,
                        Subject::Known,
                        client.deployments(&self.cancel),
                    )
                    .await
                {
                    Ok(running) => running,
                    Err(Fail::Unsupported(_)) => Vec::new(),
                    Err(other) => return Err(other),
                };
                let applications = self.applications(&client).await;
                let deployments = self
                    .recent_deployments(&client, &applications, running)
                    .await;
                Ok(self
                    .deployments_view(&client, &deployments, &applications)
                    .await)
            }
            ["status"] => {
                // Always read afresh: this is the banner.
                let client = self.client().await?;
                let version = self.check_version(&client, true).await?;
                let unavailable = lock(&self.probe).summary();
                Ok(views::version_message(&version, unavailable.as_deref()))
            }
            ["deployments", uuid] => {
                let client = self.connected().await?;
                let deployments = self
                    .probed(
                        &client,
                        Feature::History,
                        Subject::Resource(uuid),
                        client.application_deployments(uuid, None, Some(50), &self.cancel),
                    )
                    .await?;
                let applications = self.applications(&client).await;
                Ok(self
                    .deployments_view(&client, &deployments, &applications)
                    .await)
            }
            ["application", uuid] => {
                let client = self.connected().await?;
                let application = self
                    .probed(
                        &client,
                        Feature::Application,
                        Subject::Resource(uuid),
                        client.application(uuid, &self.cancel),
                    )
                    .await?;
                // The server is not on the application itself; its latest
                // deployment names it. Best effort: the confirm still names
                // the application and branch without it.
                let server = self
                    .probed(
                        &client,
                        Feature::History,
                        Subject::Known,
                        client.application_deployments(uuid, None, Some(1), &self.cancel),
                    )
                    .await
                    .ok()
                    .and_then(|deployments| deployments.into_iter().find_map(|d| d.server_name));
                Ok(views::application_detail(&application, server.as_deref()))
            }
            ["application", uuid, "envs"] => {
                let client = self.connected().await?;
                let envs = self
                    .probed(
                        &client,
                        Feature::Envs,
                        Subject::Resource(uuid),
                        client.envs(uuid, &self.cancel),
                    )
                    .await?;
                Ok(views::envs_table(&envs))
            }
            ["application", uuid, "logs"] => {
                if let Some(reason) = lock(&self.probe).unsupported_text(Feature::AppLogs) {
                    return Err(Fail::Unsupported(reason));
                }
                Ok(View::LogStream {
                    stream: format!("application/{uuid}/logs"),
                    title: "Application logs".to_owned(),
                })
            }
            ["deployment", uuid, "logs"] => Ok(View::LogStream {
                stream: format!("deployment/{uuid}/logs"),
                title: "Deployment log".to_owned(),
            }),
            _ => Err(unknown()),
        }
    }

    /// Every application, for joining rows and Redeploy; an instance where
    /// the list cannot be read gives none (rows show with fewer details).
    async fn applications(&self, client: &CoolifyClient) -> Vec<Application> {
        self.probed(
            client,
            Feature::Applications,
            Subject::Known,
            client.applications(&self.cancel),
        )
        .await
        .unwrap_or_default()
    }

    async fn action_invoke(&self, params: &Value) -> Result<Value, Fail> {
        let action = str_param(params, "action")?;
        let action_params = params.get("params").cloned().unwrap_or(Value::Null);
        let client = self.client().await?;
        let call = actions::invoke(&client, action, &action_params, &self.cancel);
        let uuid = action_params
            .get("uuid")
            .and_then(Value::as_str)
            .filter(|uuid| !uuid.is_empty());
        // Probed only when the params are whole, so a malformed request is
        // never taken for a missing endpoint.
        let result = match (Feature::of_action(action, &action_params), uuid) {
            (Some(feature), Some(uuid)) => {
                let subject = if feature == Feature::CancelDeployment {
                    Subject::Deployment(uuid)
                } else {
                    Subject::Resource(uuid)
                };
                self.probed(&client, feature, subject, call)
                    .await
                    .map_err(|fail| match fail {
                        Fail::Unsupported(reason) => Fail::Unsupported(format!(
                            "{reason} Berth no longer offers it on this instance."
                        )),
                        other => other,
                    })?
            }
            _ => call.await?,
        };
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
        let mut read_any = false;
        let end = follow_application_logs(
            &client,
            &uuid,
            APP_LOG_LINES,
            self.timing.app_log_poll,
            &token,
            |lines| {
                read_any = true;
                self.lines(&stream, lines, None);
            },
        )
        .await;
        if read_any {
            lock(&self.probe).succeeded(Feature::AppLogs);
        } else if !matches!(end, ClientError::Cancelled) {
            if let Some(reason) = self
                .learn(&client, Feature::AppLogs, Subject::Resource(&uuid), &end)
                .await
            {
                lock(&self.streams).remove(&stream);
                self.lines(&stream, vec![reason], Some("error"));
                return;
            }
        }
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
        let applications = match self
            .probed(
                &client,
                Feature::Applications,
                Subject::Known,
                client.applications(&self.cancel),
            )
            .await
        {
            Ok(applications) => applications,
            Err(Fail::Unsupported(_)) => return Ok(json!([])),
            Err(other) => return Err(other),
        };
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
        probe: Mutex::new(Probe::default()),
        version_read: tokio::sync::Mutex::new(()),
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

#[cfg(test)]
#[allow(clippy::expect_used)]
mod merge_tests {
    use super::*;

    fn deployment(uuid: &str, id: i64, created: &str) -> Deployment {
        serde_json::from_value(json!({
            "id": id,
            "deployment_uuid": uuid,
            "created_at": created,
        }))
        .expect("a deployment decodes")
    }

    fn uuids(deployments: &[Deployment]) -> Vec<&str> {
        deployments
            .iter()
            .filter_map(|d| d.deployment_uuid.as_deref())
            .collect()
    }

    #[test]
    fn plugin_merge_recent_is_newest_first_without_repeats_and_capped() {
        let running = vec![deployment("run", 5, "2026-10-24T14:31:00Z")];
        let histories = vec![
            vec![
                deployment("old", 1, "2026-10-24T10:00:00Z"),
                deployment("run", 5, "2026-10-24T14:31:00Z"),
            ],
            vec![
                deployment("mid", 3, "2026-10-24T12:00:00Z"),
                deployment("none", 2, "not a time"),
            ],
        ];
        let all = merge_recent(running.clone(), histories.clone(), 10);
        assert_eq!(uuids(&all), ["run", "mid", "old", "none"]);
        let capped = merge_recent(running, histories, 2);
        assert_eq!(uuids(&capped), ["run", "mid"]);
    }
}
