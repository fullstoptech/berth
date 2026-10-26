//! Claude Code adapter (PLAN §6.9, D43, ADR 0005).
//!
//! One long-lived `claude -p` process per session speaking stream-json in
//! both directions, with Berth's MCP server as the only tool source. The
//! binary comes from `cfg.harness.binary` (found through
//! `berth_sys::SearchPath` at detection time) and runs with `cfg.path` as
//! its `PATH` in `cfg.workspace`.
//!
//! Interrupt finding (verified against `claude --help`, 2.1.310): no flag
//! or documented stdin control message is named there. The stream-json
//! input protocol does carry control requests, so `interrupt` writes
//! `{"type":"control_request","request":{"subtype":"interrupt"}}` and
//! falls back to killing the child when stdin is already gone.
//! `berth_sys::process` exposes no signal API, so SIGINT is not available
//! without a new dependency; tokio's `start_kill` (SIGKILL) is the
//! fallback. Unknown stdout lines are ignored, never a panic.
//!
//! Timing: with stream-json input the process emits nothing — not even
//! the init line — until the first user message, so `Ready` arrives with
//! the first turn, not at spawn.

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{
    tool, AssistantError, AssistantEvent, HarnessSession, SessionConfig, Started, UserInput,
};

/// The fixed flags every session passes (D43). `--mcp-config`,
/// `--allowedTools`, the session selector and the optional `--model` are
/// appended by [`args`]/`args_with` because they carry values.
pub const FLAGS: &[&str] = &[
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--include-partial-messages",
    "--strict-mcp-config",
    "--tools",
    "",
    "--setting-sources",
    "",
    "--disable-slash-commands",
];

/// Which MCP tools Claude may run without a prompt. `mcp__berth` names
/// the whole `berth` server; the P7-T3 probe verifies this form under
/// `-p` (the fallback spelling is `mcp__berth__*`).
pub const ALLOWED_TOOLS: &str = "mcp__berth";

const STDERR_TAIL_BYTES: usize = 4096;
const CONTEXT_FILE_CAP: usize = 32 * 1024;
const FOLDER_TREE_CAP: usize = 200;

fn args_with(
    cfg: &SessionConfig,
    mcp_config: &std::path::Path,
    allowed_tools: &str,
    model: Option<&str>,
) -> Vec<String> {
    let mut out: Vec<String> = FLAGS.iter().map(|f| (*f).to_string()).collect();
    out.push("--mcp-config".into());
    out.push(mcp_config.to_string_lossy().into_owned());
    out.push("--allowedTools".into());
    out.push(allowed_tools.to_string());
    match &cfg.resume {
        Some(id) => {
            out.push("--resume".into());
            out.push(id.clone());
        }
        None => {
            out.push("--session-id".into());
            out.push(cfg.session.0.to_string());
        }
    }
    if let Some(model) = model.filter(|m| !m.is_empty()) {
        out.push("--model".into());
        out.push(model.to_string());
    }
    out
}

#[must_use]
pub fn args(cfg: &SessionConfig, mcp_config: &std::path::Path) -> Vec<String> {
    args_with(cfg, mcp_config, ALLOWED_TOOLS, None)
}

// ---------------------------------------------------------------------------
// Line parsing. `parse_line` is pure and stateless; the reader patches in
// the two things it cannot know: a finished tool's elapsed time and the
// stderr tail on errors.
// ---------------------------------------------------------------------------

fn text_of(v: Option<&serde_json::Value>) -> String {
    v.and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn parse_assistant(message: &serde_json::Value, out: &mut Vec<AssistantEvent>) {
    let Some(blocks) = message.get("content").and_then(serde_json::Value::as_array) else {
        return;
    };
    let mut saw_text = false;
    for block in blocks {
        match block.get("type").and_then(serde_json::Value::as_str) {
            Some("text") => saw_text = true,
            Some("tool_use") => {
                let wire = text_of(block.get("name"));
                // Map the wire name back to the dotted tool name (AD37).
                let name = tool(&wire).map_or(wire, |t| t.name.to_string());
                out.push(AssistantEvent::ToolStarted {
                    call_id: text_of(block.get("id")),
                    tool: name,
                    args: block
                        .get("input")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({})),
                });
            }
            _ => {}
        }
    }
    // Text itself streamed in as partial-message deltas; the completed
    // assistant message just closes the text run.
    if saw_text {
        out.push(AssistantEvent::TextDone);
    }
}

fn parse_user(message: &serde_json::Value, out: &mut Vec<AssistantEvent>) {
    let Some(blocks) = message.get("content").and_then(serde_json::Value::as_array) else {
        return;
    };
    for block in blocks {
        if block.get("type").and_then(serde_json::Value::as_str) == Some("tool_result") {
            let is_error = block
                .get("is_error")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            out.push(AssistantEvent::ToolFinished {
                call_id: text_of(block.get("tool_use_id")),
                ok: !is_error,
                ms: 0,
            });
        }
    }
}

#[must_use]
pub fn parse_line(line: &str) -> Vec<AssistantEvent> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match v.get("type").and_then(serde_json::Value::as_str) {
        Some("system") => {
            if v.get("subtype").and_then(serde_json::Value::as_str) == Some("init") {
                out.push(AssistantEvent::Ready {
                    harness_session: text_of(v.get("session_id")),
                });
            }
        }
        Some("stream_event") => {
            if let Some(event) = v.get("event") {
                let delta = event.get("delta");
                let is_text_delta = event.get("type").and_then(serde_json::Value::as_str)
                    == Some("content_block_delta")
                    && delta
                        .and_then(|d| d.get("type"))
                        .and_then(serde_json::Value::as_str)
                        == Some("text_delta");
                if is_text_delta {
                    let text = text_of(delta.and_then(|d| d.get("text")));
                    if !text.is_empty() {
                        out.push(AssistantEvent::TextDelta { text });
                    }
                }
            }
        }
        Some("assistant") => {
            if let Some(message) = v.get("message") {
                parse_assistant(message, &mut out);
            }
        }
        Some("user") => {
            if let Some(message) = v.get("message") {
                parse_user(message, &mut out);
            }
        }
        Some("result") => {
            let is_error = v.get("is_error").and_then(serde_json::Value::as_bool) == Some(true)
                || v.get("subtype")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|s| s.starts_with("error"));
            if is_error {
                let mut message = text_of(v.get("result"));
                if message.is_empty() {
                    message = v
                        .get("errors")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|errs| errs.first())
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
                if message.is_empty() {
                    message = text_of(v.get("api_error_status"));
                }
                if message.is_empty() {
                    message = "Claude Code reported an error".into();
                }
                out.push(AssistantEvent::Error {
                    message,
                    stderr_tail: String::new(),
                });
            } else {
                out.push(AssistantEvent::TurnDone {
                    stop_reason: v
                        .get("stop_reason")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string),
                });
            }
        }
        Some("error") => {
            let message = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(serde_json::Value::as_str)
                .map_or_else(
                    || "Claude Code reported an error".to_string(),
                    ToString::to_string,
                );
            out.push(AssistantEvent::Error {
                message,
                stderr_tail: String::new(),
            });
        }
        _ => {}
    }
    out
}

// ---------------------------------------------------------------------------
// The --mcp-config file: written 0600 into the runtime dir because it
// holds the bridge token, deleted when the session closes (AD37). Its
// contents are never logged.
// ---------------------------------------------------------------------------

fn mcp_config_json(cfg: &SessionConfig) -> serde_json::Value {
    serde_json::json!({
        "mcpServers": {
            "berth": {
                "command": cfg.mcp.berth_binary.to_string_lossy(),
                "args": ["mcp", "--bridge", cfg.mcp.bridge.to_string_lossy(),
                         "--session", cfg.session.0.to_string()],
                "env": { "BERTH_BRIDGE_TOKEN": cfg.mcp.token },
            }
        }
    })
}

fn write_mcp_config(cfg: &SessionConfig) -> Result<std::path::PathBuf, AssistantError> {
    let dir = berth_sys::paths::runtime_dir();
    let path = dir.join(format!("mcp-{}.json", cfg.session.0));
    write_mcp_config_at(cfg, &path)?;
    Ok(path)
}

fn write_mcp_config_at(cfg: &SessionConfig, path: &std::path::Path) -> Result<(), AssistantError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AssistantError::Protocol(format!("claude mcp-config dir: {e}")))?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(path)
        .map_err(|e| AssistantError::Protocol(format!("claude mcp-config file: {e}")))?;
    // A pre-existing file keeps its old mode through truncate; pin it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    let text = serde_json::to_string(&mcp_config_json(cfg))
        .map_err(|e| AssistantError::Protocol(format!("claude mcp-config json: {e}")))?;
    file.write_all(text.as_bytes())
        .map_err(|e| AssistantError::Protocol(format!("claude mcp-config write: {e}")))
}

/// Default bodies for the context chips: what the adapter itself can
/// resolve without an engine. Files are read redacted and capped;
/// `Problem` and `Inspect` resolve empty (their headers still name them,
/// and the harness can fetch the bodies with its read tools).
fn resolve_context(item: &berth_core::ContextItem) -> String {
    use berth_core::ContextItem;
    let read_capped = |path: &std::path::Path| -> String {
        let Ok(text) = std::fs::read_to_string(path) else {
            return String::new();
        };
        let mut text = if crate::redact::is_env_file(path) {
            crate::redact::env_file(&text)
        } else {
            text
        };
        if text.len() > CONTEXT_FILE_CAP {
            text.truncate(text.floor_char_boundary(CONTEXT_FILE_CAP));
            text.push_str("\n… (truncated)");
        }
        text
    };
    match item {
        ContextItem::Logs { lines, .. } => lines.join("\n"),
        ContextItem::ComposeFile(path) => crate::redact::compose_text(&read_capped(path)),
        ContextItem::Dockerfile(path) | ContextItem::File(path) => read_capped(path),
        ContextItem::FolderTree(path) => {
            let mut names: Vec<String> = Vec::new();
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    if names.len() >= FOLDER_TREE_CAP {
                        names.push("…".into());
                        break;
                    }
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
            names.sort();
            names.join("\n")
        }
        ContextItem::Problem(_) | ContextItem::Inspect(_) => String::new(),
    }
}

fn user_message_line(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": text },
    })
    .to_string()
}

type SharedStdin = Arc<tokio::sync::Mutex<Option<tokio::process::ChildStdin>>>;

async fn write_line(stdin: &SharedStdin, line: &str) -> Result<(), AssistantError> {
    let mut guard = stdin.lock().await;
    let Some(writer) = guard.as_mut() else {
        return Err(AssistantError::Protocol(
            "claude session stdin is closed".into(),
        ));
    };
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|e| AssistantError::Protocol(format!("claude stdin write: {e}")))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|e| AssistantError::Protocol(format!("claude stdin write: {e}")))?;
    writer
        .flush()
        .await
        .map_err(|e| AssistantError::Protocol(format!("claude stdin flush: {e}")))
}

/// Answers a `can_use_tool` control request, should one ever arrive:
/// Berth's own tools are allowed (the bridge executor gates them, ADR
/// 0006); anything else is denied. Returns true when the line was such a
/// request, whatever the answer.
fn control_reply(line: &str) -> Option<String> {
    let v = serde_json::from_str::<serde_json::Value>(line).ok()?;
    if v.get("type").and_then(serde_json::Value::as_str) != Some("control_request") {
        return None;
    }
    let request = v.get("request")?;
    if request.get("subtype").and_then(serde_json::Value::as_str) != Some("can_use_tool") {
        return None;
    }
    let name = text_of(request.get("tool_name"));
    let behavior = if tool(&name).is_some() {
        "allow"
    } else {
        "deny"
    };
    Some(
        serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": text_of(v.get("request_id")),
                "response": { "behavior": behavior, "updatedInput": request.get("input").cloned().unwrap_or_else(|| serde_json::json!({})) },
            }
        })
        .to_string(),
    )
}

fn push_tail(tail: &Arc<Mutex<String>>, line: &str) {
    if let Ok(mut guard) = tail.lock() {
        guard.push_str(line);
        guard.push('\n');
        if guard.len() > STDERR_TAIL_BYTES {
            let cut = guard.len() - STDERR_TAIL_BYTES;
            let cut = guard.ceil_char_boundary(cut);
            guard.drain(..cut);
        }
    }
}

fn tail_text(tail: &Arc<Mutex<String>>) -> String {
    tail.lock().map_or_else(|_| String::new(), |g| g.clone())
}

struct ClaudeSession {
    stdin: SharedStdin,
    kill: Option<oneshot::Sender<()>>,
    supervisor: Option<tokio::task::JoinHandle<()>>,
    mcp_config: std::path::PathBuf,
    system_prompt: String,
    first_send: bool,
}

#[async_trait::async_trait]
impl HarnessSession for ClaudeSession {
    async fn send(&mut self, input: UserInput) -> Result<(), AssistantError> {
        let mut text = String::new();
        // D43's flag table carries no --system-prompt, so the prompt
        // rides on the first user turn (never on a resumed one: the
        // harness already has it in history).
        if self.first_send && !self.system_prompt.is_empty() {
            text.push_str(&self.system_prompt);
            text.push_str("\n\n");
        }
        self.first_send = false;
        let context = crate::context::render(&input.context, &resolve_context);
        if !context.is_empty() {
            text.push_str(&context);
            text.push('\n');
        }
        text.push_str(&input.text);
        write_line(&self.stdin, &user_message_line(&text)).await
    }

    async fn interrupt(&mut self) -> Result<(), AssistantError> {
        // See the finding in the module docs: control_request interrupt,
        // else kill (stdin gone means the session is ending anyway).
        let line = serde_json::json!({
            "type": "control_request",
            "request_id": uuid::Uuid::now_v7().to_string(),
            "request": { "subtype": "interrupt" },
        })
        .to_string();
        match write_line(&self.stdin, &line).await {
            Ok(()) => Ok(()),
            Err(err) => {
                if let Some(kill) = self.kill.take() {
                    let _ = kill.send(());
                }
                Err(err)
            }
        }
    }

    async fn close(mut self: Box<Self>) {
        // EOF on stdin ends a -p session; the supervisor grants a grace
        // period, then kills.
        self.stdin.lock().await.take();
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(());
        }
        if let Some(handle) = self.supervisor.take() {
            let _ = tokio::time::timeout(Duration::from_secs(10), handle).await;
        }
        let _ = std::fs::remove_file(&self.mcp_config);
    }
}

/// Owns the child: forwards stdout as events (patching tool timings and
/// the stderr tail), buffers stderr, answers `can_use_tool`, records raw
/// lines when `BERTH_RECORD_PATH` is set (fixture capture, AD37), and
/// kills the child on cancel or, after a grace period, on close.
#[allow(clippy::too_many_arguments)]
async fn supervise(
    mut child: tokio::process::Child,
    stdin: SharedStdin,
    events: mpsc::Sender<AssistantEvent>,
    cancel: CancellationToken,
    mut close_rx: oneshot::Receiver<()>,
    record_path: Option<std::path::PathBuf>,
) {
    let tail: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let mut stdout_lines = child.stdout.take().map(|s| BufReader::new(s).lines());
    let mut stderr_lines = child.stderr.take().map(|s| BufReader::new(s).lines());
    let mut stdout_open = stdout_lines.is_some();
    let mut stderr_open = stderr_lines.is_some();
    let mut started_at: HashMap<String, Instant> = HashMap::new();
    let mut closing = false;
    let mut exited: Option<std::process::ExitStatus> = None;
    let grace = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(grace);
    let mut grace_armed = false;

    loop {
        if exited.is_some() && !stdout_open {
            break;
        }
        tokio::select! {
            line = async {
                match stdout_lines.as_mut() {
                    Some(lines) => lines.next_line().await,
                    None => std::future::pending().await,
                }
            }, if stdout_open => {
                match line {
                    Ok(Some(line)) => {
                        if let Some(path) = &record_path {
                            if let Ok(mut f) = std::fs::OpenOptions::new()
                                .create(true).append(true).open(path) {
                                let _ = writeln!(f, "{line}");
                            }
                        }
                        if let Some(reply) = control_reply(&line) {
                            let _ = write_line(&stdin, &reply).await;
                        }
                        for mut event in parse_line(&line) {
                            match &mut event {
                                AssistantEvent::ToolStarted { call_id, .. } => {
                                    started_at.insert(call_id.clone(), Instant::now());
                                }
                                AssistantEvent::ToolFinished { call_id, ms, .. } => {
                                    if let Some(t) = started_at.remove(call_id) {
                                        *ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);
                                    }
                                }
                                AssistantEvent::Error { stderr_tail, .. } => {
                                    *stderr_tail = tail_text(&tail);
                                }
                                _ => {}
                            }
                            if events.send(event).await.is_err() {
                                let _ = child.start_kill();
                                return;
                            }
                        }
                    }
                    _ => stdout_open = false,
                }
            }
            line = async {
                match stderr_lines.as_mut() {
                    Some(lines) => lines.next_line().await,
                    None => std::future::pending().await,
                }
            }, if stderr_open => {
                match line {
                    Ok(Some(line)) => push_tail(&tail, &line),
                    _ => stderr_open = false,
                }
            }
            // Child::wait is cancel safe, so recreating it each pass is fine.
            status = child.wait(), if exited.is_none() => {
                exited = status.ok();
            }
            _ = &mut close_rx, if !closing => {
                closing = true;
                grace_armed = true;
                grace.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(5));
            }
            () = &mut grace, if grace_armed => {
                let _ = child.start_kill();
                grace_armed = false;
            }
            () = cancel.cancelled() => {
                let _ = child.start_kill();
                let _ = events.send(AssistantEvent::Error {
                    message: "cancelled".into(),
                    stderr_tail: tail_text(&tail),
                }).await;
                return;
            }
        }
    }
    if !closing {
        if let Some(status) = exited {
            if !status.success() {
                let _ = events
                    .send(AssistantEvent::Error {
                        message: format!("Claude Code stopped ({status})"),
                        stderr_tail: tail_text(&tail),
                    })
                    .await;
            }
        }
    }
}

/// Starts a Claude Code session per D43.
///
/// # Errors
/// The harness binary is missing, the mcp-config file cannot be written,
/// or the process cannot be spawned.
pub async fn start(
    cfg: SessionConfig,
    cancel: CancellationToken,
) -> Result<Started, AssistantError> {
    let mcp_config = write_mcp_config(&cfg)?;
    let allowed = std::env::var("BERTH_CLAUDE_ALLOWED_TOOLS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| ALLOWED_TOOLS.to_string());
    let model = std::env::var("BERTH_CLAUDE_MODEL").ok();
    let argv = args_with(&cfg, &mcp_config, &allowed, model.as_deref());

    let mut command = tokio::process::Command::new(&cfg.harness.binary);
    command
        .args(&argv)
        .current_dir(&cfg.workspace)
        .env("PATH", &cfg.path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            let _ = std::fs::remove_file(&mcp_config);
            return Err(if err.kind() == std::io::ErrorKind::NotFound {
                AssistantError::NotInstalled(format!("claude ({})", cfg.harness.binary.display()))
            } else {
                AssistantError::Protocol(format!("claude spawn: {err}"))
            });
        }
    };
    let stdin: SharedStdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take()));
    let (events_tx, events_rx) = mpsc::channel(64);
    let (close_tx, close_rx) = oneshot::channel();
    let record_path = std::env::var_os("BERTH_RECORD_PATH")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from);
    let session = ClaudeSession {
        stdin: stdin.clone(),
        kill: Some(close_tx),
        supervisor: None,
        mcp_config: mcp_config.clone(),
        system_prompt: cfg.system_prompt.clone(),
        first_send: cfg.resume.is_none(),
    };
    let handle = tokio::spawn(supervise(
        child,
        stdin,
        events_tx,
        cancel,
        close_rx,
        record_path,
    ));
    let mut session = session;
    session.supervisor = Some(handle);
    Ok((Box::new(session), events_rx))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use crate::{McpLaunch, SessionConfig};
    use berth_core::{Harness, HarnessId, HarnessTransport, SessionId};

    fn cfg_with(binary: std::path::PathBuf, resume: Option<String>) -> SessionConfig {
        SessionConfig {
            harness: Harness {
                id: HarnessId("claude".into()),
                name: "Claude Code".into(),
                binary,
                version: None,
                transport: HarnessTransport::ClaudeStreamJson,
                available: true,
            },
            session: SessionId(uuid::Uuid::from_u128(7)),
            workspace: std::env::temp_dir(),
            resume,
            mcp: McpLaunch {
                berth_binary: "/opt/berth".into(),
                bridge: "/tmp/b.sock".into(),
                token: "tok-test".into(),
            },
            path: std::env::var_os("PATH").unwrap_or_default(),
            system_prompt: String::new(),
        }
    }

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/assistant/claude")
            .join(name)
    }

    fn fixture_lines(name: &str) -> Vec<String> {
        let path = fixture_path(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("fixture {} missing ({e})", path.display()));
        text.lines().map(ToString::to_string).collect()
    }

    #[test]
    fn claude_args_fresh_session_matches_d43() {
        let cfg = cfg_with("claude".into(), None);
        let got = args(&cfg, std::path::Path::new("/tmp/mcp.json"));
        let want: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--strict-mcp-config",
            "--tools",
            "",
            "--setting-sources",
            "",
            "--disable-slash-commands",
            "--mcp-config",
            "/tmp/mcp.json",
            "--allowedTools",
            ALLOWED_TOOLS,
            "--session-id",
            "00000000-0000-0000-0000-000000000007",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn claude_args_resume_replaces_session_id() {
        let cfg = cfg_with("claude".into(), Some("harness-42".into()));
        let got = args(&cfg, std::path::Path::new("/tmp/mcp.json"));
        assert!(
            got.windows(2).any(|w| w == ["--resume", "harness-42"]),
            "{got:?}"
        );
        assert!(!got.iter().any(|a| a == "--session-id"), "{got:?}");
    }

    #[test]
    fn claude_args_model_appended_only_when_set() {
        let cfg = cfg_with("claude".into(), None);
        let got = args_with(
            &cfg,
            std::path::Path::new("/tmp/mcp.json"),
            ALLOWED_TOOLS,
            Some("haiku"),
        );
        assert_eq!(got[got.len() - 2..], ["--model", "haiku"]);
        let plain = args(&cfg, std::path::Path::new("/tmp/mcp.json"));
        assert!(!plain.iter().any(|a| a == "--model"), "{plain:?}");
    }

    #[test]
    fn claude_parse_init_line_becomes_ready() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc-1","model":"haiku"}"#;
        assert_eq!(
            parse_line(line),
            vec![AssistantEvent::Ready {
                harness_session: "abc-1".into()
            }]
        );
    }

    #[test]
    fn claude_parse_text_delta_from_stream_event() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello "}}}"#;
        assert_eq!(
            parse_line(line),
            vec![AssistantEvent::TextDelta {
                text: "Hello ".into()
            }]
        );
        // Non-text deltas (tool input JSON) never surface as text.
        let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}}"#;
        assert_eq!(parse_line(line), vec![]);
    }

    #[test]
    fn claude_parse_tool_use_maps_wire_name_to_dotted() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"mcp__berth__containers_list","input":{"all":true}}]}}"#;
        assert_eq!(
            parse_line(line),
            vec![AssistantEvent::ToolStarted {
                call_id: "toolu_1".into(),
                tool: "containers.list".into(),
                args: serde_json::json!({"all": true}),
            }]
        );
    }

    #[test]
    fn claude_parse_tool_result_ok_and_error() {
        let ok = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"done"}]}}"#;
        assert_eq!(
            parse_line(ok),
            vec![AssistantEvent::ToolFinished {
                call_id: "toolu_1".into(),
                ok: true,
                ms: 0
            }]
        );
        let err = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_2","content":"no","is_error":true}]}}"#;
        assert_eq!(
            parse_line(err),
            vec![AssistantEvent::ToolFinished {
                call_id: "toolu_2".into(),
                ok: false,
                ms: 0
            }]
        );
    }

    #[test]
    fn claude_parse_result_success_and_error() {
        let ok = r#"{"type":"result","subtype":"success","is_error":false,"result":"hi","stop_reason":"end_turn"}"#;
        assert_eq!(
            parse_line(ok),
            vec![AssistantEvent::TurnDone {
                stop_reason: Some("end_turn".into())
            }]
        );
        let err =
            r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"too many"}"#;
        assert_eq!(
            parse_line(err),
            vec![AssistantEvent::Error {
                message: "too many".into(),
                stderr_tail: String::new()
            }]
        );
    }

    #[test]
    fn claude_parse_unknown_and_garbage_lines_are_ignored() {
        for line in [
            "",
            "not json",
            r#"{"type":"system","subtype":"hook_started"}"#,
            r#"{"type":"stream_event","event":{"type":"message_start"}}"#,
            r#"{"type":"control_response","response":{"subtype":"success"}}"#,
            r#"{"something":true}"#,
        ] {
            assert_eq!(parse_line(line), vec![], "line: {line}");
        }
    }

    #[test]
    fn claude_fixture_replays_to_a_full_turn() {
        let lines = fixture_lines("list-containers.jsonl");
        assert!(lines.len() > 3, "fixture too small: {} lines", lines.len());
        let mut events: Vec<AssistantEvent> = lines.iter().flat_map(|l| parse_line(l)).collect();
        assert!(
            matches!(events.first(), Some(AssistantEvent::Ready { harness_session }) if !harness_session.is_empty()),
            "first event: {:?}",
            events.first()
        );
        assert!(
            events.iter().any(|e| matches!(e, AssistantEvent::ToolStarted { tool, .. } if tool == "containers.list")),
            "no containers.list ToolStarted in {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AssistantEvent::ToolFinished { ok: true, .. })),
            "no ok ToolFinished in {events:?}"
        );
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                AssistantEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.contains("shop-api-api-1"), "answer text: {text}");
        assert!(
            matches!(events.pop(), Some(AssistantEvent::TurnDone { .. })),
            "last event is not TurnDone"
        );
    }

    #[test]
    fn claude_mcp_config_file_is_0600_and_names_the_bridge() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("mcp.json");
        let mut cfg = cfg_with("claude".into(), None);
        cfg.mcp.token = "sekret-token".into();
        write_mcp_config_at(&cfg, &path).expect("write config");
        let text = std::fs::read_to_string(&path).expect("read config");
        let v: serde_json::Value = serde_json::from_str(&text).expect("config json");
        let berth = v
            .get("mcpServers")
            .and_then(|s| s.get("berth"))
            .expect("berth server");
        assert_eq!(
            berth.get("command").and_then(|c| c.as_str()),
            Some("/opt/berth")
        );
        let argv: Vec<&str> = berth
            .get("args")
            .and_then(serde_json::Value::as_array)
            .expect("args")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        assert_eq!(
            argv,
            [
                "mcp",
                "--bridge",
                "/tmp/b.sock",
                "--session",
                "00000000-0000-0000-0000-000000000007"
            ]
        );
        assert_eq!(
            berth
                .get("env")
                .and_then(|e| e.get("BERTH_BRIDGE_TOKEN"))
                .and_then(|t| t.as_str()),
            Some("sekret-token")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "mode {mode:o}");
        }
    }

    #[tokio::test]
    async fn claude_missing_binary_is_not_installed() {
        let cfg = cfg_with("/nonexistent/berth-no-claude".into(), None);
        let err = match start(cfg, CancellationToken::new()).await {
            Ok(_) => panic!("a missing binary must not start"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("claude"), "{err}");
        assert!(matches!(err, AssistantError::NotInstalled(_)), "{err:?}");
    }

    // -- Fake harness: a shell script that replays recorded lines. Paths
    // are baked into the script because the child gets no custom env.

    struct FakeHarness {
        cfg: SessionConfig,
        args_file: std::path::PathBuf,
        input_file: std::path::PathBuf,
        _dir: tempfile::TempDir,
    }

    fn fake_harness(replay: &str, resume: Option<String>) -> FakeHarness {
        let dir = tempfile::tempdir().expect("dir");
        // The mcp-config file lands under BERTH_DATA_DIR; keep it in tmp.
        std::env::set_var("BERTH_DATA_DIR", dir.path().join("data"));
        let replay_file = dir.path().join("replay.jsonl");
        // A final newline matters: without it `cat` emits an unterminated
        // line and the reader waits forever.
        let replay = if replay.is_empty() || replay.ends_with('\n') {
            replay.to_string()
        } else {
            format!("{replay}\n")
        };
        std::fs::write(&replay_file, replay).expect("replay");
        let args_file = dir.path().join("args.txt");
        let input_file = dir.path().join("stdin.txt");
        let script = dir.path().join("fake-claude.sh");
        let body = format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$@\" > '{args}'\n\
             printf '%s\\n' '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"fake-1\"}}'\n\
             while IFS= read -r line; do\n\
               printf '%s\\n' \"$line\" >> '{input}'\n\
               case \"$line\" in\n\
                 *interrupt*) printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"stop_reason\":\"interrupted\"}}' ;;\n\
                 *) cat '{replay}' ;;\n\
               esac\n\
             done\n",
            args = args_file.display(),
            input = input_file.display(),
            replay = replay_file.display(),
        );
        std::fs::write(&script, body).expect("script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        FakeHarness {
            cfg: cfg_with(script, resume),
            args_file,
            input_file,
            _dir: dir,
        }
    }

    async fn next_event(rx: &mut mpsc::Receiver<AssistantEvent>, secs: u64) -> AssistantEvent {
        tokio::time::timeout(Duration::from_secs(secs), rx.recv())
            .await
            .unwrap_or_else(|_| panic!("no event within {secs}s"))
            .expect("event channel closed early")
    }

    async fn until_turn_end(
        rx: &mut mpsc::Receiver<AssistantEvent>,
        secs: u64,
    ) -> Vec<AssistantEvent> {
        let mut got = Vec::new();
        loop {
            let event = next_event(rx, secs).await;
            let end = matches!(
                event,
                AssistantEvent::TurnDone { .. } | AssistantEvent::Error { .. }
            );
            got.push(event);
            if end {
                return got;
            }
        }
    }

    #[tokio::test]
    async fn claude_fake_session_end_to_end() {
        // Replay = the recorded fixture minus its init line (the fake
        // prints its own init with a known session id).
        let lines = fixture_lines("list-containers.jsonl");
        let replay: Vec<&String> = lines
            .iter()
            .filter(|l| !l.contains("\"subtype\":\"init\""))
            .collect();
        let fake = fake_harness(
            &replay
                .iter()
                .map(|l| l.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            None,
        );
        let (mut session, mut rx) = start(fake.cfg.clone(), CancellationToken::new())
            .await
            .expect("fake session starts");
        assert_eq!(
            next_event(&mut rx, 10).await,
            AssistantEvent::Ready {
                harness_session: "fake-1".into()
            }
        );
        session
            .send(UserInput {
                text: "List the containers on this engine".into(),
                context: vec![],
            })
            .await
            .expect("send");
        let events = until_turn_end(&mut rx, 10).await;
        assert!(
            events.iter().any(|e| matches!(e, AssistantEvent::ToolStarted { tool, .. } if tool == "containers.list")),
            "{events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AssistantEvent::ToolFinished { ok: true, .. })),
            "{events:?}"
        );
        assert!(
            matches!(events.last(), Some(AssistantEvent::TurnDone { .. })),
            "{events:?}"
        );
        let said = std::fs::read_to_string(&fake.input_file).expect("stdin log");
        assert!(said.contains("List the containers"), "{said}");
        let mcp = berth_sys::paths::runtime_dir().join(format!("mcp-{}.json", fake.cfg.session.0));
        assert!(mcp.exists(), "mcp-config missing during session");
        session.close().await;
        assert!(!mcp.exists(), "mcp-config not deleted on close");
    }

    #[tokio::test]
    async fn claude_fake_session_interrupt_sends_control_request() {
        // Replay holds only a delta: the turn ends only via interrupt.
        let fake = fake_harness(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"counting"}}}"#,
            None,
        );
        let (mut session, mut rx) = start(fake.cfg.clone(), CancellationToken::new())
            .await
            .expect("fake session starts");
        let _ready = next_event(&mut rx, 10).await;
        session
            .send(UserInput {
                text: "count".into(),
                context: vec![],
            })
            .await
            .expect("send");
        assert_eq!(
            next_event(&mut rx, 10).await,
            AssistantEvent::TextDelta {
                text: "counting".into()
            }
        );
        session.interrupt().await.expect("interrupt");
        let events = until_turn_end(&mut rx, 10).await;
        assert!(
            matches!(events.last(), Some(AssistantEvent::TurnDone { stop_reason: Some(r) }) if r == "interrupted"),
            "{events:?}"
        );
        let said = std::fs::read_to_string(&fake.input_file).expect("stdin log");
        assert!(
            said.contains("control_request") && said.contains("interrupt"),
            "{said}"
        );
        session.close().await;
    }

    #[tokio::test]
    async fn claude_fake_session_resume_args_reach_the_binary() {
        let fake = fake_harness("", Some("resume-me-9".into()));
        let (mut session, mut rx) = start(fake.cfg.clone(), CancellationToken::new())
            .await
            .expect("fake session starts");
        let _ready = next_event(&mut rx, 10).await;
        let argv = std::fs::read_to_string(&fake.args_file).expect("args log");
        assert!(argv.contains("--resume\nresume-me-9"), "{argv}");
        assert!(!argv.contains("--session-id"), "{argv}");
        session
            .interrupt()
            .await
            .expect("interrupt on idle session");
        let _ = until_turn_end(&mut rx, 10).await;
        session.close().await;
    }

    // -- Live runs (AD37: two in total, haiku, one-turn prompts). Both
    // are ignored and additionally gated on BERTH_HARNESS_SMOKE=1.

    const SMOKE_TOKEN: &str = "smoke-token-claude-p7t3";

    fn berth_binary() -> std::path::PathBuf {
        if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
            let p = std::path::PathBuf::from(dir).join("debug/berth");
            if p.exists() {
                return p;
            }
        }
        let exe = std::env::current_exe().expect("test exe");
        exe.parent()
            .and_then(std::path::Path::parent)
            .expect("debug dir")
            .join("berth")
    }

    /// Binds a test bridge and answers tool calls with mock-world text.
    /// (berth-app's ToolCore-over-MockEngine path is covered by
    /// `mcp_it` in berth-app; this crate cannot depend on berth-app.)
    async fn live_session(
        dir: &std::path::Path,
    ) -> (
        SessionConfig,
        CancellationToken,
        Arc<std::sync::atomic::AtomicBool>,
    ) {
        use crate::bridge::{BridgeResponse, BridgeServer};
        std::env::set_var("BERTH_DATA_DIR", dir.join("data"));
        std::env::set_var("BERTH_CLAUDE_MODEL", "haiku");
        let socket = dir.join("b.sock");
        let cancel = CancellationToken::new();
        let (_server, mut incoming) =
            BridgeServer::bind_at(socket.clone(), SMOKE_TOKEN.to_string(), cancel.clone())
                .await
                .expect("bind bridge");
        let listed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = listed.clone();
        let answer_cancel = cancel.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = answer_cancel.cancelled() => return,
                    next = incoming.recv() => {
                        let Some(call) = next else { return };
                        let is_list = tool(&call.request.tool)
                            .is_some_and(|t| t.name == "containers.list");
                        if is_list {
                            seen.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        let text = if is_list {
                            "shop-api-api-1 (running, image shop-api:latest)\n\
                             shop-api-db-1 (running, image postgres:16)\n\
                             shop-api-web-1 (exited, image nginx:1.27)"
                                .to_string()
                        } else {
                            format!("{} ok", call.request.tool)
                        };
                        let _ = call.reply.send(BridgeResponse::Ok {
                            id: call.request.id,
                            text,
                            data: None,
                        });
                    }
                }
            }
        });
        let binary = berth_sys::paths::home().join(".local/bin/claude");
        assert!(binary.exists(), "claude not found at {}", binary.display());
        let berth = berth_binary();
        assert!(
            berth.exists(),
            "berth binary missing at {}; build the workspace first",
            berth.display()
        );
        let mut cfg = cfg_with(binary, None);
        cfg.session = SessionId(uuid::Uuid::now_v7());
        cfg.workspace = dir.to_path_buf();
        cfg.mcp = McpLaunch {
            berth_binary: berth,
            bridge: socket,
            token: SMOKE_TOKEN.to_string(),
        };
        cfg.system_prompt =
            "You are Berth's assistant. Use the berth MCP tools to answer questions about the Docker engine. Be brief.".into();
        (cfg, cancel, listed)
    }

    fn smoke_enabled() -> bool {
        if std::env::var("BERTH_HARNESS_SMOKE").ok().as_deref() == Some("1") {
            true
        } else {
            eprintln!("skipped: BERTH_HARNESS_SMOKE is not 1");
            false
        }
    }

    #[tokio::test]
    #[ignore = "live harness run (AD37 budget); needs BERTH_HARNESS_SMOKE=1"]
    async fn claude_live_probe_allowed_tools_and_interrupt() {
        if !smoke_enabled() {
            return;
        }
        let dir = tempfile::tempdir().expect("dir");
        let (cfg, cancel, listed) = live_session(dir.path()).await;
        let (mut session, mut rx) = start(cfg, cancel.clone()).await.expect("session starts");
        // stream-json input emits nothing (not even init) until the
        // first user message — verified by hand 2026-10-26 — so send
        // first, then read.
        // Turn 1: does --allowedTools let the tool run prompt-free?
        session
            .send(UserInput {
                text: "List the containers on this engine".into(),
                context: vec![],
            })
            .await
            .expect("send");
        let events = until_turn_end(&mut rx, 180).await;
        eprintln!("probe turn 1 events: {events:?}");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AssistantEvent::Ready { .. })),
            "no Ready in {events:?}"
        );
        assert!(
            listed.load(std::sync::atomic::Ordering::SeqCst),
            "containers.list never reached the bridge; --allowedTools form denied it"
        );
        // Turn 2: interrupt a long answer.
        session
            .send(UserInput {
                text: "Count from 1 to 400, one number per line, no other text.".into(),
                context: vec![],
            })
            .await
            .expect("send 2");
        let first = next_event(&mut rx, 120).await;
        eprintln!("probe turn 2 first event: {first:?}");
        session.interrupt().await.expect("interrupt");
        let tail = until_turn_end(&mut rx, 60).await;
        eprintln!("probe interrupt outcome: {tail:?}");
        session.close().await;
        cancel.cancel();
    }

    #[tokio::test]
    #[ignore = "live harness run (AD37 budget); needs BERTH_HARNESS_SMOKE=1"]
    async fn smoke_claude() {
        if !smoke_enabled() {
            return;
        }
        let dir = tempfile::tempdir().expect("dir");
        let (cfg, cancel, listed) = live_session(dir.path()).await;
        let record = std::env::var("BERTH_RECORD").ok().as_deref() == Some("1");
        let fixture = fixture_path("list-containers.jsonl");
        if record {
            if let Some(parent) = fixture.parent() {
                std::fs::create_dir_all(parent).expect("fixture dir");
            }
            let _ = std::fs::remove_file(&fixture);
            std::env::set_var("BERTH_RECORD_PATH", &fixture);
        }
        let (mut session, mut rx) = start(cfg, cancel.clone()).await.expect("session starts");
        // Send first: stream-json emits nothing until the first message.
        session
            .send(UserInput {
                text: "List the containers on this engine".into(),
                context: vec![],
            })
            .await
            .expect("send");
        let events = until_turn_end(&mut rx, 180).await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AssistantEvent::Ready { .. })),
            "no Ready in {events:?}"
        );
        assert!(
            matches!(events.last(), Some(AssistantEvent::TurnDone { .. })),
            "turn did not finish cleanly: {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(e, AssistantEvent::ToolStarted { tool, .. } if tool == "containers.list")),
            "no containers.list call in {events:?}"
        );
        assert!(
            listed.load(std::sync::atomic::Ordering::SeqCst),
            "containers.list never reached the bridge"
        );
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                AssistantEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.contains("shop-api-api-1"), "answer: {text}");
        session.close().await;
        cancel.cancel();
        if record {
            let captured = std::fs::read_to_string(&fixture).expect("recorded fixture");
            assert!(
                !captured.contains(SMOKE_TOKEN),
                "bridge token leaked into the fixture"
            );
            assert!(captured.lines().count() > 3, "fixture too small");
        }
    }
}
