//! muse adapter (ADR 0005, PLAN §6.9, D43 as amended by AD37).
//!
//! Wire facts, recorded from the installed `muse` 1.4.13 on 2026-10-26
//! (`muse schema generate-json-schema`, stable surface fingerprint
//! `sha256:7fddaf646dbc317e5fcac0785e135eb04df14fbb0db1863089dfd6da21730e5b`,
//! plus an `initialize` probe against `muse serve --provider echo`):
//!
//! - MSP v1 framing is newline-delimited JSON-RPC 2.0 over stdio: one JSON
//!   object per line, no `Content-Length` headers.
//! - Handshake: client `initialize` request `{clientInfo:{name,version},
//!   capabilities:{requestedCapabilities:["sessionMcp"],
//!   userInputDialogs:false}}`, then an `initialized` notification.
//! - `session/start` requires a client-minted UUIDv7 `commandId`; the result
//!   is `{session:{sessionId,…}, viewCursor}` and a `session/started`
//!   notification carrying the same session arrives just before it.
//! - `turn/start` takes `{commandId, sessionId, input:[{type:"text",text}]}`;
//!   `turn/interrupt` takes `{commandId, sessionId, turnId?}`.
//! - Approvals arrive both as the `approval/requested` notification and as
//!   the server-initiated `approval/request` request (same params). The
//!   request is answered with an empty presentation receipt (`{}` result);
//!   the decision itself travels as an `approval/decide` command
//!   `{approvalId, choiceId, commandId, requirementId, sessionId}`, where
//!   `choiceId` is picked from `availableChoices` by its `decision` value.
//! - The MSP stdio MCP entry is `{transport:"stdio", command, args, env,
//!   framing, mode}` with `framing` spelled `lineDelimitedJson` (what rmcp
//!   `transport-io` speaks) and `mode` `required` or `optional`.
//!
//! There is no system-prompt member anywhere in the session or turn params,
//! so the session's system prompt prefixes the first turn's text.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;

use crate::{
    context, tool, AssistantError, AssistantEvent, HarnessSession, SessionConfig, Started,
    UserInput,
};

/// How long a single MSP request may wait for its response.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Lines of stderr kept for an `Exited`/`Error` report.
const STDERR_TAIL_LINES: usize = 20;

/// The flags every `muse serve` host runs with (D43): no shell, no writes.
/// The user's model pick adds `--model <id>` (AD39); without one,
/// `BERTH_MUSE_MODEL` does (the smoke tests use it to pick the cheapest
/// model the installed muse accepts). Every flag here is one
/// `muse serve --help` lists.
#[must_use]
pub fn serve_args(model: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "serve".to_string(),
        "--disable-shell".to_string(),
        "--disable-write".to_string(),
    ];
    let model = model
        .map(str::to_string)
        .filter(|m| !m.trim().is_empty())
        .or_else(|| {
            std::env::var("BERTH_MUSE_MODEL")
                .ok()
                .filter(|m| !m.trim().is_empty())
        });
    if let Some(model) = model {
        args.push("--model".to_string());
        args.push(model);
    }
    args
}

/// The `config` member shared by `session/start` and `session/resume`:
/// Berth's MCP server as the session's only configured server (AD37).
fn mcp_config(cfg: &SessionConfig) -> Value {
    json!({
        "mcpServers": {
            "berth": {
                "transport": "stdio",
                "command": cfg.mcp.berth_binary.to_string_lossy(),
                "args": [
                    "mcp",
                    "--bridge",
                    cfg.mcp.bridge.to_string_lossy(),
                    "--session",
                    cfg.session.0.to_string(),
                ],
                "env": { "BERTH_BRIDGE_TOKEN": cfg.mcp.token },
                "framing": "lineDelimitedJson",
                "mode": "required",
            }
        }
    })
}

/// The `session/start` params for a fresh session (PLAN §6.9). The host
/// mints the session id; `commandId` is the required UUIDv7 idempotency
/// handle. No `modelId`: the model is chosen host-wide at `serve` time
/// (AD39, see [`serve_args`]), so every session of the host runs it.
#[must_use]
pub fn session_start(cfg: &SessionConfig) -> Value {
    json!({
        "commandId": uuid::Uuid::now_v7().to_string(),
        "workspaceRoot": cfg.workspace.to_string_lossy(),
        "config": mcp_config(cfg),
    })
}

/// The `session/resume` params for reopening `session_id`. History stays on
/// the host: `excludeItems` asks for metadata only, since Berth keeps its
/// own transcript.
fn session_resume(cfg: &SessionConfig, session_id: &str) -> Value {
    json!({
        "commandId": uuid::Uuid::now_v7().to_string(),
        "sessionId": session_id,
        "excludeItems": true,
        "config": mcp_config(cfg),
    })
}

/// True when `name` names one of Berth's catalog tools, in any of the forms
/// a harness may present: the dotted name, the wire name, the
/// `mcp__berth__` form (all resolved by [`crate::tool`]), or a
/// server-qualified form whose server segment is `berth`
/// (`berth/containers_list`, `berth:containers_list`). A name qualified by
/// any other server never qualifies, even when its last segment collides
/// with a Berth tool.
fn is_berth_tool(name: &str) -> bool {
    if tool(name).is_some() {
        return true;
    }
    let segments: Vec<&str> = name.split(['/', ':']).flat_map(|s| s.split("__")).collect();
    segments.len() > 1
        && segments[..segments.len() - 1].contains(&"berth")
        && segments.last().is_some_and(|last| tool(last).is_some())
}

/// Maps a tool name as muse presents it back to Berth's dotted catalog name
/// where it resolves; anything else passes through unchanged.
fn catalog_name(raw: &str) -> String {
    if let Some(def) = tool(raw) {
        return def.name.to_string();
    }
    let last = raw.rsplit(['/', ':']).next().unwrap_or(raw);
    let last = last.rsplit("__").next().unwrap_or(last);
    match tool(last) {
        Some(def) if is_berth_tool(raw) => def.name.to_string(),
        _ => raw.to_string(),
    }
}

/// Picks the `choiceId` of the first offered choice applying `decision`,
/// preferring a one-shot grant over a session-wide or persistent one.
fn pick_choice(params: &Value, decision: &str) -> Option<String> {
    let choices = params.get("availableChoices")?.as_array()?;
    let mut fallback: Option<String> = None;
    for choice in choices {
        if choice.get("decision").and_then(Value::as_str) != Some(decision) {
            continue;
        }
        let Some(id) = choice.get("choiceId").and_then(Value::as_str) else {
            continue;
        };
        if choice.get("scope").and_then(Value::as_str) == Some("once") {
            return Some(id.to_string());
        }
        fallback.get_or_insert_with(|| id.to_string());
    }
    fallback
}

/// The answer to `approval/requested`: the `approval/decide` params that
/// allow a Berth tool (Berth gates it itself, ADR 0006) and deny everything
/// else — other servers' tools, shell, writes, network, and any params too
/// malformed to prove they name a Berth tool. Pure and total: fields the
/// params do not carry come back as empty strings, and the caller only
/// sends the decision when it names a real approval.
#[must_use]
pub fn approval_decision(params: &Value) -> Value {
    let subject = params.get("subject");
    let kind = subject
        .and_then(|s| s.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let tool_name = params
        .get("toolName")
        .and_then(Value::as_str)
        .or_else(|| {
            subject
                .and_then(|s| s.get("toolName"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    // Only a `tool` subject (or an unkinded one carrying a tool name, as
    // older hosts sent) can be a Berth tool call; shell, fileAccess,
    // network, process and unknown kinds are denied outright.
    let allow = (kind == "tool" || kind.is_empty()) && is_berth_tool(tool_name);
    let choice = if allow {
        pick_choice(params, "approved")
            .or_else(|| pick_choice(params, "approvedForSession"))
            .or_else(|| pick_choice(params, "approvedPolicyAmendment"))
            // A Berth tool with no approving choice on offer must not fall
            // through to silence: deny it explicitly.
            .or_else(|| pick_choice(params, "denied"))
    } else {
        pick_choice(params, "denied")
            .or_else(|| pick_choice(params, "deniedPolicyAmendment"))
            .or_else(|| pick_choice(params, "abort"))
    };
    let approval_id = params
        .get("approvalId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let requirement = params
        .get("currentRequirementId")
        .cloned()
        .unwrap_or_else(|| json!({ "approvalId": approval_id, "sourceIndex": 0 }));
    json!({
        "approvalId": approval_id,
        "choiceId": choice.unwrap_or_default(),
        "commandId": uuid::Uuid::now_v7().to_string(),
        "requirementId": requirement,
        "sessionId": params.get("sessionId").and_then(Value::as_str).unwrap_or_default(),
    })
}

/// The `args` of a `toolCall` item: model-authored JSON carried verbatim as
/// a string. Parsed back to a value; unparseable text is kept as a string
/// rather than dropped.
fn parse_tool_args(item: &Value) -> Value {
    match item.get("args") {
        Some(Value::String(text)) => {
            serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone()))
        }
        Some(other) => other.clone(),
        None => Value::Null,
    }
}

fn item_call_id(item: &Value) -> String {
    item.get("callId")
        .and_then(Value::as_str)
        .or_else(|| item.get("itemId").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

/// Maps one MSP frame (a whole JSON-RPC notification value) to assistant
/// events. Approvals are not events: the session answers them itself (see
/// [`approval_decision`]), so `approval/requested` and `approval/request`
/// map to nothing here. Unknown methods and malformed frames map to
/// nothing; the session must never fall over on a frame it does not know.
#[must_use]
pub fn parse_notification(v: &Value) -> Vec<AssistantEvent> {
    let Some(method) = v.get("method").and_then(Value::as_str) else {
        return Vec::new();
    };
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "session/started" => match params
            .get("session")
            .and_then(|s| s.get("sessionId"))
            .and_then(Value::as_str)
        {
            Some(id) => vec![AssistantEvent::Ready {
                harness_session: id.to_string(),
            }],
            None => Vec::new(),
        },
        "item/delta" => {
            // An absent `field` means "text"; other fields (tool `output`,
            // summaries) are not assistant prose and never surface here.
            let field = params
                .get("field")
                .and_then(Value::as_str)
                .unwrap_or("text");
            match (field, params.get("delta").and_then(Value::as_str)) {
                ("text", Some(delta)) if !delta.is_empty() => vec![AssistantEvent::TextDelta {
                    text: delta.to_string(),
                }],
                _ => Vec::new(),
            }
        }
        "item/started" => {
            let Some(item) = params.get("item") else {
                return Vec::new();
            };
            if item.get("kind").and_then(Value::as_str) != Some("toolCall") {
                return Vec::new();
            }
            vec![AssistantEvent::ToolStarted {
                call_id: item_call_id(item),
                tool: catalog_name(item.get("tool").and_then(Value::as_str).unwrap_or_default()),
                args: parse_tool_args(item),
            }]
        }
        "item/completed" => {
            let Some(item) = params.get("item") else {
                return Vec::new();
            };
            match item.get("kind").and_then(Value::as_str) {
                Some("toolCall") => vec![AssistantEvent::ToolFinished {
                    call_id: item_call_id(item),
                    ok: item.get("status").and_then(Value::as_str) == Some("completed"),
                    ms: item
                        .get("durationMs")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                }],
                Some("agentMessage") => vec![AssistantEvent::TextDone],
                _ => Vec::new(),
            }
        }
        "turn/completed" => {
            let terminal = params
                .get("terminal")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mut events = Vec::new();
            if let Some(message) = params
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
            {
                events.push(AssistantEvent::Error {
                    message: message.to_string(),
                    stderr_tail: String::new(),
                });
            } else if terminal == "failed" {
                events.push(AssistantEvent::Error {
                    message: params
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("the turn failed")
                        .to_string(),
                    stderr_tail: String::new(),
                });
            }
            events.push(AssistantEvent::TurnDone {
                stop_reason: Some(terminal),
            });
            events
        }
        _ => Vec::new(),
    }
}

/// Records raw protocol lines when `BERTH_RECORD=1` (the fixture capture of
/// P7-T4). The bridge token never reaches the file: every line is scrubbed
/// of it before it is written. Inert in normal runs.
#[derive(Clone)]
struct Recorder {
    file: Option<Arc<Mutex<std::fs::File>>>,
    token: String,
}

impl Recorder {
    fn from_env(token: &str) -> Self {
        let file = match (
            std::env::var("BERTH_RECORD").as_deref(),
            std::env::var_os("BERTH_RECORD_FILE"),
        ) {
            (Ok("1"), Some(path)) if !path.is_empty() => {
                let path = PathBuf::from(path);
                if let Some(dir) = path.parent() {
                    let _created = std::fs::create_dir_all(dir);
                }
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .ok()
                    .map(|f| Arc::new(Mutex::new(f)))
            }
            _ => None,
        };
        Self {
            file,
            token: token.to_string(),
        }
    }

    fn record(&self, line: &str) {
        let Some(file) = &self.file else { return };
        let scrubbed = if self.token.is_empty() {
            line.to_string()
        } else {
            line.replace(&self.token, "<bridge-token>")
        };
        if let Ok(mut f) = file.lock() {
            let _written = writeln!(f, "{}", scrubbed.trim_end());
            let _flushed = f.flush();
        }
    }
}

/// The last lines of the child's stderr, for `Exited`/`Error` reports.
#[derive(Default)]
struct StderrTail {
    lines: Mutex<Vec<String>>,
}

impl StderrTail {
    fn push(&self, line: &str) {
        if let Ok(mut lines) = self.lines.lock() {
            lines.push(line.to_string());
            let excess = lines.len().saturating_sub(STDERR_TAIL_LINES);
            lines.drain(..excess);
        }
    }

    fn text(&self) -> String {
        self.lines
            .lock()
            .map(|lines| lines.join("\n"))
            .unwrap_or_default()
    }
}

type Pending = Arc<AsyncMutex<BTreeMap<i64, oneshot::Sender<Result<Value, AssistantError>>>>>;

/// The client end of the MSP connection: writes frames, matches responses
/// to the requests waiting on them.
struct Link {
    stdin: AsyncMutex<ChildStdin>,
    pending: Pending,
    next_id: AtomicI64,
    recorder: Recorder,
}

impl Link {
    async fn write_frame(&self, frame: &Value) -> Result<(), AssistantError> {
        let line = serde_json::to_string(frame)
            .map_err(|e| AssistantError::Protocol(format!("muse: encoding a frame: {e}")))?;
        self.recorder.record(&line);
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .map_err(|e| AssistantError::Protocol(format!("muse: writing to the harness: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| AssistantError::Protocol(format!("muse: writing to the harness: {e}")))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), AssistantError> {
        self.write_frame(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    /// Sends one request and waits, bounded, for its response.
    async fn request(&self, method: &str, params: Value) -> Result<Value, AssistantError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = self.write_frame(&frame).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_closed)) => Err(AssistantError::Protocol(
                "muse: the harness closed the connection mid-request".into(),
            )),
            Err(_elapsed) => {
                self.pending.lock().await.remove(&id);
                Err(AssistantError::Protocol(format!(
                    "muse: no answer to {method} within {} s",
                    REQUEST_TIMEOUT.as_secs()
                )))
            }
        }
    }

    /// Answers a server-initiated request: a receipt for approvals and user
    /// input, a protocol refusal for anything else. Never leaves one
    /// hanging.
    async fn answer_server_request(&self, frame: &Value) {
        let Some(id) = frame.get("id").cloned() else {
            return;
        };
        let method = frame
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let answer = if method == "approval/request" || method == "userInput/request" {
            // The SS5.3.3 presentation receipt: an empty result. The real
            // decision follows as its own command.
            json!({ "jsonrpc": "2.0", "id": id, "result": {} })
        } else {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not handled by this client" },
            })
        };
        if let Err(e) = self.write_frame(&answer).await {
            tracing::warn!("muse: answering a server request failed: {e}");
        }
    }

    /// Decides one approval with [`approval_decision`]. A decision that
    /// does not name a real approval and choice is not sent; the host's own
    /// timeout then denies it, which is the safe outcome.
    async fn decide_approval(&self, params: &Value) {
        let decision = approval_decision(params);
        let usable = ["approvalId", "choiceId", "sessionId"].iter().all(|key| {
            decision
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        });
        if !usable {
            tracing::warn!(
                "muse: an approval arrived too malformed to decide; left to time out denied"
            );
            return;
        }
        if let Err(e) = self.request("approval/decide", decision).await {
            tracing::warn!("muse: approval/decide failed: {e}");
        }
    }
}

/// Where the read loop stands, so an exit during the handshake does not
/// also surface as a session `Error` event (the handshake error says it).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Handshaking,
    Live,
    Closing,
}

/// The read loop: routes responses to their waiters, answers server
/// requests, decides approvals, and turns notifications into events.
/// Approvals are decided in a spawned task so a slow `approval/decide`
/// round trip never stalls the frames behind it.
async fn read_loop(
    stdout: tokio::process::ChildStdout,
    link: Arc<Link>,
    events: mpsc::Sender<AssistantEvent>,
    phase: Arc<Mutex<Phase>>,
    stderr_tail: Arc<StderrTail>,
    harness_name: String,
) {
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("muse: reading the harness failed: {e}");
                break;
            }
        };
        link.recorder.record(&line);
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            // Never log the line itself: it may carry prompt or tool text.
            tracing::warn!("muse: ignoring a frame that is not JSON");
            continue;
        };
        let method = frame.get("method").and_then(Value::as_str);
        if let (None, Some(id)) = (method, frame.get("id").and_then(Value::as_i64)) {
            let result = if let Some(error) = frame.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("the harness reported an error");
                let kind = error
                    .get("data")
                    .and_then(|d| d.get("kind"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                Err(AssistantError::Protocol(format!(
                    "muse: {message} ({kind})"
                )))
            } else {
                Ok(frame.get("result").cloned().unwrap_or(Value::Null))
            };
            if let Some(waiter) = link.pending.lock().await.remove(&id) {
                let _sent = waiter.send(result);
            }
            continue;
        }
        let Some(method) = method else { continue };
        if frame.get("id").is_some() {
            link.answer_server_request(&frame).await;
        }
        if method == "approval/requested" || method == "approval/request" {
            let params = frame.get("params").cloned().unwrap_or(Value::Null);
            let link = Arc::clone(&link);
            tokio::spawn(async move { link.decide_approval(&params).await });
            continue;
        }
        if method == "userInput/request" {
            // The handshake says this client renders no dialogs; if one is
            // asked anyway there is no answer to give, and the turn fails
            // on the host side rather than hanging here.
            continue;
        }
        for event in parse_notification(&frame) {
            if events.send(event).await.is_err() {
                return;
            }
        }
    }
    // The harness is gone: fail every request still waiting, then say so
    // (unless Berth closed the session itself, or is still handshaking).
    let waiters = std::mem::take(&mut *link.pending.lock().await);
    for (_id, waiter) in waiters {
        let _sent = waiter.send(Err(AssistantError::Protocol(
            "muse: the harness exited".into(),
        )));
    }
    let live = phase.lock().is_ok_and(|p| *p == Phase::Live);
    if live {
        let _sent = events
            .send(AssistantEvent::Error {
                message: format!("{harness_name} stopped"),
                stderr_tail: stderr_tail.text(),
            })
            .await;
    }
}

struct MuseSession {
    link: Arc<Link>,
    child: Arc<AsyncMutex<Child>>,
    phase: Arc<Mutex<Phase>>,
    session_id: String,
    current_turn: Option<String>,
    /// Sent once, as a prefix of the first turn (MSP has no system-prompt
    /// member); a resumed session was already given it in its first life.
    pending_system_prompt: Option<String>,
}

#[async_trait::async_trait]
impl HarnessSession for MuseSession {
    async fn send(&mut self, input: UserInput) -> Result<(), AssistantError> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(system) = self.pending_system_prompt.take() {
            if !system.is_empty() {
                parts.push(system);
            }
        }
        let rendered = context::render(&input.context, &|_item| String::new());
        if !rendered.is_empty() {
            parts.push(rendered);
        }
        parts.push(input.text);
        let result = self
            .link
            .request(
                "turn/start",
                json!({
                    "commandId": uuid::Uuid::now_v7().to_string(),
                    "sessionId": self.session_id,
                    "input": [{ "type": "text", "text": parts.join("\n\n") }],
                }),
            )
            .await?;
        self.current_turn = result
            .get("turnId")
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(())
    }

    async fn interrupt(&mut self) -> Result<(), AssistantError> {
        let mut params = json!({
            "commandId": uuid::Uuid::now_v7().to_string(),
            "sessionId": self.session_id,
        });
        if let (Some(turn), Some(object)) = (&self.current_turn, params.as_object_mut()) {
            object.insert("turnId".to_string(), Value::String(turn.clone()));
        }
        self.link.request("turn/interrupt", params).await?;
        self.current_turn = None;
        Ok(())
    }

    async fn close(self: Box<Self>) {
        if let Ok(mut phase) = self.phase.lock() {
            *phase = Phase::Closing;
        }
        let leave = self.link.request(
            "session/leave",
            json!({
                "commandId": uuid::Uuid::now_v7().to_string(),
                "sessionId": self.session_id,
                "stop": true,
            }),
        );
        let _left = tokio::time::timeout(Duration::from_secs(5), leave).await;
        let mut child = self.child.lock().await;
        let _killed = child.kill().await;
        let _waited = child.wait().await;
    }
}

/// Spawns `muse serve`, performs the MSP handshake, and starts (or
/// resumes) the session. A host that cannot be spawned, exits during the
/// handshake, or refuses the session surfaces as
/// [`AssistantError::Exited`] or [`AssistantError::Protocol`] carrying the
/// stderr tail — there is no `muse exec` fallback behind a live serve
/// session (a one-shot exec cannot answer approvals or take an interrupt,
/// so it cannot honour this trait).
///
/// # Errors
/// The binary cannot be spawned, the handshake or session start fails, or
/// `cancel` fires first.
pub async fn start(
    cfg: SessionConfig,
    cancel: CancellationToken,
) -> Result<Started, AssistantError> {
    let harness_name = cfg.harness.name.clone();
    let mut command = Command::new(&cfg.harness.binary);
    command
        .args(serve_args(cfg.model.as_deref()))
        .current_dir(&cfg.workspace)
        .env("PATH", &cfg.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| {
        AssistantError::Protocol(format!(
            "muse: could not start `{}`: {e}",
            cfg.harness.binary.display()
        ))
    })?;
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _killed = child.kill().await;
        return Err(AssistantError::Protocol(
            "muse: the harness process has no stdio pipes".into(),
        ));
    };
    let child = Arc::new(AsyncMutex::new(child));
    let stderr_tail = Arc::new(StderrTail::default());
    {
        let tail = Arc::clone(&stderr_tail);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tail.push(&line);
            }
        });
    }
    // Cancellation kills the host: the handshake and every later request
    // then fail at once instead of waiting out their timeouts.
    {
        let child = Arc::clone(&child);
        let cancel = cancel.clone();
        tokio::spawn(async move {
            cancel.cancelled().await;
            let _killed = child.lock().await.kill().await;
        });
    }
    let link = Arc::new(Link {
        stdin: AsyncMutex::new(stdin),
        pending: Pending::default(),
        next_id: AtomicI64::new(1),
        recorder: Recorder::from_env(&cfg.mcp.token),
    });
    let phase = Arc::new(Mutex::new(Phase::Handshaking));
    let (events_tx, events_rx) = mpsc::channel(256);
    tokio::spawn(read_loop(
        stdout,
        Arc::clone(&link),
        events_tx.clone(),
        Arc::clone(&phase),
        Arc::clone(&stderr_tail),
        harness_name.clone(),
    ));

    let handshake = async {
        link.request(
            "initialize",
            json!({
                "clientInfo": { "name": "berth", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": {
                    "requestedCapabilities": ["sessionMcp"],
                    "userInputDialogs": false,
                },
            }),
        )
        .await?;
        link.notify("initialized", json!({})).await?;
        let resumed = cfg.resume.is_some();
        let result = match &cfg.resume {
            Some(id) => {
                link.request("session/resume", session_resume(&cfg, id))
                    .await?
            }
            None => link.request("session/start", session_start(&cfg)).await?,
        };
        let session_id = result
            .get("session")
            .and_then(|s| s.get("sessionId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                AssistantError::Protocol("muse: the session start named no session".into())
            })?;
        Ok::<(String, bool), AssistantError>((session_id, resumed))
    };
    let (session_id, resumed) = tokio::select! {
        biased;
        () = cancel.cancelled() => {
            let _killed = child.lock().await.kill().await;
            return Err(AssistantError::Cancelled);
        }
        outcome = handshake => outcome,
    }
    .map_err(|e| match e {
        // The host died or spoke nonsense before the session existed: say
        // so with whatever it left on stderr.
        AssistantError::Protocol(message) => {
            let tail = stderr_tail.text();
            if tail.is_empty() {
                AssistantError::Protocol(message)
            } else {
                AssistantError::Exited {
                    harness: harness_name.clone(),
                    stderr_tail: tail,
                }
            }
        }
        other => other,
    })?;
    if let Ok(mut phase) = phase.lock() {
        *phase = Phase::Live;
    }
    if resumed {
        // `session/started` is never broadcast for a resume, so Ready is
        // stated here; a fresh start already produced it in the read loop.
        let _sent = events_tx
            .send(AssistantEvent::Ready {
                harness_session: session_id.clone(),
            })
            .await;
    }
    let session = MuseSession {
        link,
        child,
        phase,
        session_id,
        current_turn: None,
        pending_system_prompt: (!resumed && !cfg.system_prompt.is_empty())
            .then(|| cfg.system_prompt.clone()),
    };
    Ok((Box::new(session), events_rx))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::{McpLaunch, SessionConfig};
    use berth_core::{Harness, HarnessId, HarnessTransport, SessionId};

    fn cfg(binary: PathBuf, workspace: PathBuf) -> SessionConfig {
        SessionConfig {
            harness: Harness {
                id: HarnessId("muse".into()),
                name: "Muse Code".into(),
                binary,
                version: None,
                transport: HarnessTransport::MuseMsp,
                available: true,
            },
            session: SessionId(uuid::Uuid::nil()),
            workspace,
            resume: None,
            mcp: McpLaunch {
                berth_binary: "/apps/berth".into(),
                bridge: "/run/berth/bridge.sock".into(),
                token: "test-token".into(),
            },
            path: std::env::var_os("PATH").unwrap_or_default(),
            system_prompt: "You are Berth.".into(),
            model: None,
        }
    }

    fn approval_params(subject: Value, tool_name: &str) -> Value {
        json!({
            "approvalId": "appr-1",
            "sessionId": "sess-1",
            "toolName": tool_name,
            "subject": subject,
            "currentRequirementId": { "approvalId": "appr-1", "sourceIndex": 0 },
            "availableChoices": [
                { "choiceId": "allow-session", "decision": "approvedForSession", "label": "Allow for session", "scope": "session" },
                { "choiceId": "allow-once", "decision": "approved", "label": "Allow", "scope": "once" },
                { "choiceId": "deny", "decision": "denied", "label": "Deny", "scope": "once", "acceptsFeedback": true },
            ],
        })
    }

    fn choice_of(decision: &Value) -> &str {
        decision["choiceId"].as_str().expect("choiceId")
    }

    #[test]
    fn muse_serve_args_carry_the_picked_model() {
        let args = serve_args(Some("muse-spark-1.2"));
        let at = args.iter().position(|a| a == "--model").expect("--model");
        assert_eq!(args[at + 1], "muse-spark-1.2");
        assert!(
            !serve_args(Some("  ")).iter().any(|a| a == "--model")
                || std::env::var_os("BERTH_MUSE_MODEL").is_some(),
            "a blank pick adds no model of its own"
        );
    }

    #[test]
    fn muse_serve_args_disable_shell_and_write() {
        let args = serve_args(None);
        assert_eq!(args[0], "serve");
        assert!(args.contains(&"--disable-shell".to_string()), "{args:?}");
        assert!(args.contains(&"--disable-write".to_string()), "{args:?}");
    }

    #[test]
    fn muse_session_start_carries_the_berth_mcp_entry() {
        let params = session_start(&cfg("muse".into(), "/work/shop".into()));
        let command_id = params["commandId"].as_str().expect("commandId");
        assert_eq!(
            uuid::Uuid::parse_str(command_id)
                .expect("uuid")
                .get_version(),
            Some(uuid::Version::SortRand),
            "commandId must be UUIDv7: {command_id}"
        );
        assert_eq!(params["workspaceRoot"], "/work/shop");
        let berth = &params["config"]["mcpServers"]["berth"];
        assert_eq!(berth["transport"], "stdio");
        assert_eq!(berth["command"], "/apps/berth");
        assert_eq!(
            berth["args"],
            json!([
                "mcp",
                "--bridge",
                "/run/berth/bridge.sock",
                "--session",
                uuid::Uuid::nil().to_string()
            ])
        );
        assert_eq!(berth["env"]["BERTH_BRIDGE_TOKEN"], "test-token");
        assert_eq!(berth["framing"], "lineDelimitedJson");
        assert_eq!(berth["mode"], "required");
    }

    #[test]
    fn muse_parse_session_started_is_ready() {
        // A real frame from `muse serve --provider echo` (2026-10-26),
        // trimmed to the fields the mapping reads.
        let frame = json!({
            "jsonrpc": "2.0",
            "method": "session/started",
            "params": { "session": { "sessionId": "01a177fb-246d-72a0-8cd8-86474a0fac6a", "status": "idle" } },
        });
        assert_eq!(
            parse_notification(&frame),
            vec![AssistantEvent::Ready {
                harness_session: "01a177fb-246d-72a0-8cd8-86474a0fac6a".into()
            }]
        );
    }

    #[test]
    fn muse_parse_item_delta_text_only() {
        let text = json!({
            "method": "item/delta",
            "params": { "itemId": "i1", "sessionId": "s", "viewCursor": "v1", "delta": "shop-" },
        });
        assert_eq!(
            parse_notification(&text),
            vec![AssistantEvent::TextDelta {
                text: "shop-".into()
            }]
        );
        let output = json!({
            "method": "item/delta",
            "params": { "itemId": "i1", "sessionId": "s", "viewCursor": "v2", "field": "output", "delta": "raw tool bytes" },
        });
        assert_eq!(
            parse_notification(&output),
            vec![],
            "tool output is not prose"
        );
    }

    #[test]
    fn muse_parse_tool_started_maps_the_name_back_and_parses_args() {
        let frame = json!({
            "method": "item/started",
            "params": { "sessionId": "s", "viewCursor": "v", "item": {
                "itemId": "i9", "kind": "toolCall", "revision": 1, "status": "inProgress",
                "callId": "call_9", "tool": "mcp__berth__containers_list", "args": "{\"project\":\"shop\"}",
            } },
        });
        assert_eq!(
            parse_notification(&frame),
            vec![AssistantEvent::ToolStarted {
                call_id: "call_9".into(),
                tool: "containers.list".into(),
                args: json!({ "project": "shop" }),
            }]
        );
        let message = json!({
            "method": "item/started",
            "params": { "item": { "itemId": "i1", "kind": "agentMessage", "revision": 1, "status": "inProgress" } },
        });
        assert_eq!(parse_notification(&message), vec![]);
    }

    #[test]
    fn muse_parse_tool_completed_reports_ok_and_ms() {
        let done = json!({
            "method": "item/completed",
            "params": { "item": {
                "itemId": "i9", "kind": "toolCall", "revision": 2, "status": "completed",
                "callId": "call_9", "tool": "containers_list", "durationMs": 41,
            } },
        });
        assert_eq!(
            parse_notification(&done),
            vec![AssistantEvent::ToolFinished {
                call_id: "call_9".into(),
                ok: true,
                ms: 41
            }]
        );
        let failed = json!({
            "method": "item/completed",
            "params": { "item": {
                "itemId": "i9", "kind": "toolCall", "revision": 2, "status": "failed", "callId": "call_9",
            } },
        });
        assert_eq!(
            parse_notification(&failed),
            vec![AssistantEvent::ToolFinished {
                call_id: "call_9".into(),
                ok: false,
                ms: 0
            }]
        );
    }

    #[test]
    fn muse_parse_message_and_turn_completion() {
        let message = json!({
            "method": "item/completed",
            "params": { "item": { "itemId": "i1", "kind": "agentMessage", "revision": 3, "status": "completed", "text": "done" } },
        });
        assert_eq!(parse_notification(&message), vec![AssistantEvent::TextDone]);
        let turn = json!({
            "method": "turn/completed",
            "params": { "sessionId": "s", "turnId": "t", "terminal": "completed", "durationMs": 900 },
        });
        assert_eq!(
            parse_notification(&turn),
            vec![AssistantEvent::TurnDone {
                stop_reason: Some("completed".into())
            }]
        );
    }

    #[test]
    fn muse_parse_failed_turn_is_an_error_then_done() {
        let turn = json!({
            "method": "turn/completed",
            "params": {
                "sessionId": "s", "turnId": "t", "terminal": "failed",
                "error": { "kind": "providerError", "message": "the model is unavailable", "retryable": true },
            },
        });
        assert_eq!(
            parse_notification(&turn),
            vec![
                AssistantEvent::Error {
                    message: "the model is unavailable".into(),
                    stderr_tail: String::new()
                },
                AssistantEvent::TurnDone {
                    stop_reason: Some("failed".into())
                },
            ]
        );
    }

    #[test]
    fn muse_parse_ignores_approvals_unknown_and_malformed_frames() {
        for frame in [
            json!({ "method": "approval/requested", "params": { "approvalId": "a" } }),
            json!({ "method": "session/tokenUsage", "params": {} }),
            json!({ "method": "item/started", "params": {} }),
            json!({ "id": 4, "result": {} }),
            json!({}),
            Value::Null,
        ] {
            assert_eq!(parse_notification(&frame), vec![], "{frame}");
        }
    }

    #[test]
    fn muse_approval_allows_a_berth_tool_once() {
        for name in [
            "mcp__berth__containers_list",
            "containers.list",
            "containers_list",
            "berth/containers_list",
            "berth:containers_list",
        ] {
            let params = approval_params(json!({ "kind": "tool", "toolName": name }), name);
            let decision = approval_decision(&params);
            assert_eq!(choice_of(&decision), "allow-once", "{name}");
            assert_eq!(decision["approvalId"], "appr-1", "{name}");
            assert_eq!(decision["sessionId"], "sess-1", "{name}");
            assert_eq!(decision["requirementId"]["sourceIndex"], 0, "{name}");
            assert!(
                uuid::Uuid::parse_str(decision["commandId"].as_str().expect("commandId")).is_ok(),
                "{name}"
            );
        }
    }

    #[test]
    fn muse_approval_denies_other_mcp_servers_tools() {
        for name in [
            "mcp__other__containers_list",
            "other/containers_list",
            "mcp__berth__no_such_tool",
            "fetch",
        ] {
            let params = approval_params(json!({ "kind": "tool", "toolName": name }), name);
            assert_eq!(choice_of(&approval_decision(&params)), "deny", "{name}");
        }
    }

    #[test]
    fn muse_approval_denies_shell_writes_and_network() {
        for subject in [
            json!({ "kind": "shell", "command": "rm -rf /", "stages": [] }),
            json!({ "kind": "fileAccess", "path": "/etc/passwd", "access": "write", "toolName": "mcp__berth__containers_list" }),
            json!({ "kind": "network", "host": "example.com", "toolName": "containers_list" }),
            json!({ "kind": "process", "target": "dockerd", "access": "control" }),
            json!({ "kind": "somethingNew", "toolName": "mcp__berth__containers_list" }),
        ] {
            // Even a Berth tool name on a non-tool subject must not allow it.
            let params = approval_params(subject.clone(), "mcp__berth__containers_list");
            assert_eq!(choice_of(&approval_decision(&params)), "deny", "{subject}");
        }
    }

    #[test]
    fn muse_approval_malformed_params_deny_or_stay_unusable() {
        // Choices on offer but nothing proving a Berth tool: an explicit deny.
        let params = approval_params(json!({}), "");
        assert_eq!(choice_of(&approval_decision(&params)), "deny");
        // Nothing usable at all: no choice can be picked, and the decision
        // carries no approval id, so the session never sends it (the host
        // times the approval out denied).
        for params in [
            json!({}),
            json!({ "availableChoices": "nope" }),
            Value::Null,
        ] {
            let decision = approval_decision(&params);
            assert_eq!(choice_of(&decision), "", "{params}");
            assert_eq!(decision["approvalId"], "", "{params}");
        }
    }

    /// Replays every recorded protocol line in `fixtures/assistant/muse/`
    /// through the parser: the capture must stay parseable, name no token,
    /// and show the whole arc of the smoke question.
    #[test]
    fn muse_replay_of_the_recorded_fixtures_shows_the_smoke_arc() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/assistant/muse");
        let mut lines = Vec::new();
        let mut names: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("fixtures dir")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        names.sort();
        assert!(!names.is_empty(), "no fixtures in {}", dir.display());
        for path in &names {
            let text = std::fs::read_to_string(path).expect("read fixture");
            assert!(!text.contains("test-token"), "{}", path.display());
            for (_before, after) in text
                .match_indices("BERTH_BRIDGE_TOKEN\":\"")
                .map(|(i, m)| (i, &text[i + m.len()..]))
            {
                assert!(
                    after.starts_with("<bridge-token>\""),
                    "an unscrubbed token value reached {}",
                    path.display()
                );
            }
            lines.extend(text.lines().map(str::to_string));
        }
        let events: Vec<AssistantEvent> = lines
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .flat_map(|v| parse_notification(&v))
            .collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AssistantEvent::Ready { .. })),
            "no Ready in {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(e, AssistantEvent::ToolStarted { tool, .. } if tool == "containers.list")),
            "no containers.list in {events:?}"
        );
        let answer: String = events
            .iter()
            .filter_map(|e| match e {
                AssistantEvent::TextDelta { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert!(answer.contains("shop-api-api-1"), "answer was: {answer}");
        assert!(
            events.iter().any(|e| matches!(e, AssistantEvent::TurnDone { stop_reason: Some(r) } if r == "completed")),
            "no completed turn in {events:?}"
        );
    }

    /// The recorded capture carries one real approval (the host's judge
    /// decided it, so no `approval/request` reached the adapter): deciding
    /// its params must pick `allow_once`, and the same params wearing
    /// another server's name must pick the `abort` reject choice — the
    /// real host offers no `denied` choice for MCP tools.
    #[test]
    fn muse_approval_decision_on_the_recorded_real_params() {
        let text = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/assistant/muse/smoke.jsonl"),
        )
        .expect("read fixture");
        let reviewing = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(|v| v["method"] == "approval/reviewing")
            .expect("the capture has an approval");
        let params = reviewing["params"].clone();
        let decision = approval_decision(&params);
        assert_eq!(choice_of(&decision), "allow_once", "{decision}");
        assert_eq!(decision["approvalId"], params["approvalId"]);
        assert_eq!(decision["requirementId"], params["currentRequirementId"]);

        let mut other = params.clone();
        other["toolName"] = json!("mcp__other__containers_list");
        other["subject"]["toolName"] = json!("mcp__other__containers_list");
        assert_eq!(choice_of(&approval_decision(&other)), "abort");
    }

    #[cfg(unix)]
    fn write_fake_harness(dir: &std::path::Path, out: &std::path::Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = format!(
            r#"#!/bin/sh
out='{}'
turn=0
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p' | head -1)
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{}}}}' 
      ;;
    *'"method":"session/start"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","method":"session/started","params":{{"session":{{"sessionId":"fake-session-1","status":"idle"}}}}}}'
      printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"session":{{"sessionId":"fake-session-1"}},"viewCursor":"v0"}}}}' 
      ;;
    *'"method":"approval/decide"'*)
      printf '%s\n' "$line" >> "$out"
      printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"approvalId":"appr-1","status":"accepted","terminal":true}}}}' 
      printf '%s\n' '{{"jsonrpc":"2.0","method":"item/completed","params":{{"sessionId":"fake-session-1","viewCursor":"v5","item":{{"itemId":"i9","kind":"toolCall","revision":2,"status":"completed","callId":"call_9","tool":"containers_list","durationMs":7}}}}}}'
      printf '%s\n' '{{"jsonrpc":"2.0","method":"item/delta","params":{{"sessionId":"fake-session-1","viewCursor":"v6","itemId":"i1","delta":"shop-api-api-1 is running"}}}}'
      printf '%s\n' '{{"jsonrpc":"2.0","method":"item/completed","params":{{"sessionId":"fake-session-1","viewCursor":"v7","item":{{"itemId":"i1","kind":"agentMessage","revision":2,"status":"completed","text":"shop-api-api-1 is running"}}}}}}'
      printf '%s\n' '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"sessionId":"fake-session-1","viewCursor":"v8","turnId":"turn-1","terminal":"completed"}}}}'
      ;;
    *'"method":"turn/start"'*)
      turn=$((turn + 1))
      printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"commandId":"c","status":"accepted","turnId":"turn-'"$turn"'","startedNewTurn":true,"disposition":"started"}}}}' 
      if [ "$turn" = "1" ]; then
        printf '%s\n' '{{"jsonrpc":"2.0","method":"item/started","params":{{"sessionId":"fake-session-1","viewCursor":"v3","item":{{"itemId":"i9","kind":"toolCall","revision":1,"status":"inProgress","callId":"call_9","tool":"mcp__berth__containers_list","args":"{{}}"}}}}}}'
        printf '%s\n' '{{"jsonrpc":"2.0","id":900,"method":"approval/request","params":{{"approvalId":"appr-1","sessionId":"fake-session-1","toolName":"mcp__berth__containers_list","subject":{{"kind":"tool","toolName":"mcp__berth__containers_list"}},"currentRequirementId":{{"approvalId":"appr-1","sourceIndex":0}},"availableChoices":[{{"choiceId":"allow-once","decision":"approved","label":"Allow","scope":"once"}},{{"choiceId":"deny","decision":"denied","label":"Deny","scope":"once"}}]}}}}'
      fi
      ;;
    *'"method":"turn/interrupt"'*)
      printf '%s\n' "$line" >> "$out"
      printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"status":"accepted"}}}}' 
      printf '%s\n' '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"sessionId":"fake-session-1","viewCursor":"v9","turnId":"turn-2","terminal":"cancelled"}}}}'
      ;;
    *'"method":"session/leave"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{}}}}' 
      ;;
  esac
done
"#,
            out.display()
        );
        let path = dir.join("fake-muse.sh");
        std::fs::write(&path, script).expect("write fake harness");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake harness");
        path
    }

    async fn next_event(rx: &mut mpsc::Receiver<AssistantEvent>) -> AssistantEvent {
        tokio::time::timeout(Duration::from_secs(15), rx.recv())
            .await
            .expect("an event within 15 s")
            .expect("the event stream is open")
    }

    /// A full fake session: handshake, a turn whose Berth tool call is
    /// approved by the adapter itself, then a second turn interrupted.
    #[cfg(unix)]
    #[tokio::test]
    async fn muse_fake_harness_session_approves_berth_tools_and_interrupts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("seen.jsonl");
        std::fs::write(&out, "").expect("seen file");
        let binary = write_fake_harness(dir.path(), &out);
        let (mut session, mut events) = start(
            cfg(binary, dir.path().to_path_buf()),
            CancellationToken::new(),
        )
        .await
        .expect("the fake session starts");
        assert_eq!(
            next_event(&mut events).await,
            AssistantEvent::Ready {
                harness_session: "fake-session-1".into()
            }
        );
        session
            .send(UserInput {
                text: "list them".into(),
                context: Vec::new(),
            })
            .await
            .expect("send");
        assert_eq!(
            next_event(&mut events).await,
            AssistantEvent::ToolStarted {
                call_id: "call_9".into(),
                tool: "containers.list".into(),
                args: json!({}),
            }
        );
        assert_eq!(
            next_event(&mut events).await,
            AssistantEvent::ToolFinished {
                call_id: "call_9".into(),
                ok: true,
                ms: 7
            }
        );
        assert_eq!(
            next_event(&mut events).await,
            AssistantEvent::TextDelta {
                text: "shop-api-api-1 is running".into()
            }
        );
        assert_eq!(next_event(&mut events).await, AssistantEvent::TextDone);
        assert_eq!(
            next_event(&mut events).await,
            AssistantEvent::TurnDone {
                stop_reason: Some("completed".into())
            }
        );
        let seen = std::fs::read_to_string(&out).expect("seen");
        assert!(seen.contains("\"choiceId\":\"allow-once\""), "{seen}");

        session
            .send(UserInput {
                text: "and again".into(),
                context: Vec::new(),
            })
            .await
            .expect("send 2");
        session.interrupt().await.expect("interrupt");
        assert_eq!(
            next_event(&mut events).await,
            AssistantEvent::TurnDone {
                stop_reason: Some("cancelled".into())
            }
        );
        let seen = std::fs::read_to_string(&out).expect("seen");
        assert!(seen.contains("turn/interrupt"), "{seen}");
        assert!(seen.contains("\"turnId\":\"turn-2\""), "{seen}");
        tokio::time::timeout(Duration::from_secs(10), session.close())
            .await
            .expect("close within 10 s");
    }

    #[tokio::test]
    async fn muse_start_names_muse_when_the_binary_is_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = start(
            cfg(dir.path().join("no-such-muse"), dir.path().to_path_buf()),
            CancellationToken::new(),
        )
        .await
        .err()
        .expect("a missing binary cannot start");
        assert!(err.to_string().contains("muse"), "{err}");
    }

    /// The one live run (AD37 budget): the real muse, the real `berth mcp`,
    /// a test bridge answering `containers.list` itself (MockEngine lives
    /// in the app's executor, which this crate cannot reach; the bridge
    /// stands in with the mock world's answer). Needs `BERTH_HARNESS_SMOKE=1`
    /// and a built `berth` binary (`cargo build -p berth-app`, or
    /// `BERTH_BIN`). With `BERTH_RECORD=1` the run also captures
    /// `fixtures/assistant/muse/smoke.jsonl`.
    #[tokio::test]
    #[ignore = "live harness run: BERTH_HARNESS_SMOKE=1 cargo nextest run -p berth-assistant --run-ignored only smoke_muse"]
    async fn smoke_muse_lists_containers_through_the_bridge() {
        use crate::bridge::{BridgeResponse, BridgeServer};
        use std::sync::atomic::AtomicBool;

        if std::env::var("BERTH_HARNESS_SMOKE").as_deref() != Ok("1") {
            return;
        }
        if std::env::var_os("BERTH_MUSE_MODEL").is_none() {
            std::env::set_var("BERTH_MUSE_MODEL", "muse-spark-1.2");
        }
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let record_file = manifest.join("../../fixtures/assistant/muse/smoke.jsonl");
        if std::env::var("BERTH_RECORD").as_deref() == Ok("1")
            && std::env::var_os("BERTH_RECORD_FILE").is_none()
        {
            let _removed = std::fs::remove_file(&record_file);
            std::env::set_var("BERTH_RECORD_FILE", &record_file);
        }
        let berth_binary = std::env::var_os("BERTH_BIN")
            .map_or_else(|| manifest.join("../../target/debug/berth"), PathBuf::from);
        assert!(
            berth_binary.exists(),
            "build the berth binary first (cargo build -p berth-app): {}",
            berth_binary.display()
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let token = uuid::Uuid::now_v7().to_string();
        let cancel = CancellationToken::new();
        let (server, mut incoming) =
            BridgeServer::bind_at(dir.path().join("b.sock"), token.clone(), cancel.clone())
                .await
                .expect("bind bridge");
        let saw_list = Arc::new(AtomicBool::new(false));
        {
            let saw_list = Arc::clone(&saw_list);
            tokio::spawn(async move {
                while let Some(call) = incoming.recv().await {
                    let text = if call.request.tool == "containers.list" {
                        saw_list.store(true, Ordering::Relaxed);
                        "shop-api-api-1\trunning\tshop-api (api)\nshop-api-db-1\trunning\tshop-api (db)"
                            .to_string()
                    } else {
                        "{}".to_string()
                    };
                    let _sent = call.reply.send(BridgeResponse::Ok {
                        id: call.request.id,
                        text,
                        data: None,
                    });
                }
            });
        }

        let search = berth_sys::SearchPath::from_process()
            .with_login_shell()
            .await;
        let harnesses = crate::detect::detect(&search, &std::collections::BTreeMap::new()).await;
        let harness = harnesses
            .into_iter()
            .find(|h| h.transport == HarnessTransport::MuseMsp && h.available)
            .expect("muse is detected on this machine");
        let mut config = cfg(harness.binary.clone(), dir.path().to_path_buf());
        config.harness = harness;
        config.mcp = McpLaunch {
            berth_binary,
            bridge: server.socket.clone(),
            token: token.clone(),
        };
        config.path = search.path_var();
        config.system_prompt =
            "You are Berth's assistant. Always use the berth tools to look things up before answering. Be brief."
                .into();

        let (mut session, mut events) =
            tokio::time::timeout(Duration::from_secs(90), start(config, cancel.clone()))
                .await
                .expect("the session starts within 90 s")
                .expect("the session starts");
        let mut answer = String::new();
        let mut saw_tool = false;
        session
            .send(UserInput {
                text: "List the containers using your tools and tell me their names.".into(),
                context: Vec::new(),
            })
            .await
            .expect("send");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
        loop {
            let event = tokio::time::timeout_at(deadline, events.recv())
                .await
                .expect("the turn finishes within 300 s")
                .expect("the event stream is open");
            match event {
                AssistantEvent::TextDelta { text } => answer.push_str(&text),
                AssistantEvent::ToolStarted { tool, .. } if tool == "containers.list" => {
                    saw_tool = true;
                }
                AssistantEvent::Error {
                    message,
                    stderr_tail,
                } => {
                    panic!("the session errored: {message}\n{stderr_tail}");
                }
                AssistantEvent::TurnDone { .. } => break,
                _ => {}
            }
        }
        tokio::time::timeout(Duration::from_secs(15), session.close())
            .await
            .expect("close within 15 s");
        cancel.cancel();
        assert!(saw_tool, "containers.list never started; answer: {answer}");
        assert!(
            saw_list.load(Ordering::Relaxed),
            "the bridge never saw containers.list; answer: {answer}"
        );
        assert!(answer.contains("shop-api-api-1"), "answer was: {answer}");
        if record_file.exists() {
            let recorded = std::fs::read_to_string(&record_file).expect("recorded fixture");
            assert!(
                !recorded.contains(&token),
                "the bridge token reached the fixture"
            );
        }
    }
}
