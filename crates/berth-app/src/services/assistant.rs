//! Live assistant sessions (P7-T5, PLAN §6.9/§6.11, ADR 0005, AD37).
//!
//! One session per project folder. The core here is Dioxus-free like
//! `tools.rs`: it owns the harness processes (one task per session over
//! `berth_assistant::start`), applies their event stream to the session
//! values, persists on every turn end through `Store::save_session`, and
//! turns the executor's [`ApprovalRequest`]s into [`Approval`]s the UI
//! answers. Tasks never touch signals; they send [`UiMsg`]s that
//! [`AssistantService::start`] applies to `state.assistant` /
//! `state.approvals` on the UI side, so tests drive `new` and drain the
//! receiver (or read the snapshots) directly.
//!
//! Nothing here logs a prompt, a turn's text or a token. A persisted turn
//! holds a proposed change only as its AD37 summary, never a plan body.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use berth_assistant::{AssistantEvent, McpLaunch, SessionConfig, UserInput};
use berth_core::{
    AssistantSession, ContextItem, EffectKind, EffectLine, EffectSummary, Harness, ProjectId,
    ProposalState, Risk, SessionId, SessionState, ToolCall, ToolStatus, Turn, TurnPart, TurnRole,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

use super::actions::spawn_io;
use super::plans::PlanStore;
use super::tools::{ApprovalRequest, Decision, ProposalNotice, ToolCore};
use crate::state::{AppState, Approval};

// ---------------------------------------------------------------------------
// Plain words for a tool call (pure)
// ---------------------------------------------------------------------------

fn arg_str(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn arg_u64(args: &serde_json::Value, key: &str) -> Option<u64> {
    args.get(key).and_then(serde_json::Value::as_u64)
}

fn arg_list(args: &serde_json::Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The dotted catalog name of a tool as a harness spelled it
/// (`mcp__berth__containers_logs` and friends resolve through the catalog).
#[must_use]
pub fn canonical_tool(name: &str) -> String {
    berth_assistant::tool(name).map_or_else(|| name.to_string(), |def| def.name.to_string())
}

/// The risk the catalog gives a tool a harness named; unknown tools read.
#[must_use]
pub fn tool_risk(name: &str) -> Risk {
    berth_assistant::tool(name).map_or(Risk::Read, |def| def.risk)
}

/// One plain sentence naming what a tool call did, for its row in the
/// turn: "Read 200 lines of shop-api-worker-1 logs", "Inspected
/// shop-api-db-1". Built from names and counts only, never values.
#[must_use]
pub fn tool_summary(tool: &str, args: &serde_json::Value) -> String {
    let tool = canonical_tool(tool);
    let container = arg_str(args, "container").unwrap_or_else(|| "the container".to_string());
    let project = arg_str(args, "project");
    match tool.as_str() {
        "containers.logs" => {
            format!(
                "Read {} lines of {container} logs",
                arg_u64(args, "tail").unwrap_or(200)
            )
        }
        "containers.inspect" => format!("Inspected {container}"),
        "containers.stats" => format!("Checked CPU and memory of {container}"),
        "containers.list" => project.map_or_else(
            || "Listed containers".to_string(),
            |p| format!("Listed containers of {p}"),
        ),
        "containers.restart" => format!("Restarted {container}"),
        "containers.start" => format!("Started {container}"),
        "containers.stop" => format!("Stopped {container}"),
        "containers.remove" => format!(
            "Removed container {}",
            arg_str(args, "name").unwrap_or(container)
        ),
        "files.read" => format!(
            "Read {}",
            arg_str(args, "path").unwrap_or_else(|| "a file".to_string())
        ),
        "files.propose_edit" => format!(
            "Proposed a change to {}",
            arg_str(args, "path").unwrap_or_else(|| "a file".to_string())
        ),
        "compose.config" => project.map_or_else(
            || "Read the Compose configuration".to_string(),
            |p| format!("Read the Compose configuration of {p}"),
        ),
        "compose.up" | "compose.restart" => {
            let verb = if tool == "compose.up" {
                "Started"
            } else {
                "Restarted"
            };
            let services = arg_list(args, "services");
            match (project, services.is_empty()) {
                (Some(p), true) => format!("{verb} {p}"),
                (Some(p), false) => format!("{verb} {} in {p}", services.join(", ")),
                (None, _) => format!("{verb} the project"),
            }
        }
        "problems.list" => project.map_or_else(
            || "Listed problems".to_string(),
            |p| format!("Listed problems of {p}"),
        ),
        "projects.list" => "Listed projects".to_string(),
        "engines.list" => "Listed engines".to_string(),
        "images.list" => "Listed images".to_string(),
        "volumes.list" => "Listed volumes".to_string(),
        "networks.list" => "Listed networks".to_string(),
        "activity.list" => "Read recent activity".to_string(),
        "images.pull" => format!(
            "Pulled {}",
            arg_str(args, "reference").unwrap_or_else(|| "an image".to_string())
        ),
        "images.build" => format!(
            "Built {}",
            arg_str(args, "tag").unwrap_or_else(|| "an image".to_string())
        ),
        "images.remove" => format!(
            "Removed image {}",
            arg_str(args, "name").unwrap_or_else(|| "an image".to_string())
        ),
        "volumes.remove" => format!(
            "Removed volume {}",
            arg_str(args, "name").unwrap_or_else(|| "a volume".to_string())
        ),
        "system.prune" => format!(
            "Pruned {}",
            arg_str(args, "name").unwrap_or_else(|| "the engine".to_string())
        ),
        other => other.replace(['.', '_'], " "),
    }
}

/// The command a gated call stands for, for the approval card's mono
/// block ("docker compose up -d db worker"). Names only, never values.
#[must_use]
pub fn command_line(call: &ToolCall) -> String {
    let args = &call.args;
    let services = arg_list(args, "services");
    let services = if services.is_empty() {
        String::new()
    } else {
        format!(" {}", services.join(" "))
    };
    let container = arg_str(args, "container")
        .or_else(|| arg_str(args, "name"))
        .unwrap_or_default();
    match canonical_tool(&call.tool).as_str() {
        "compose.up" => format!("docker compose up -d{services}"),
        "compose.restart" => format!("docker compose restart{services}"),
        "containers.restart" => format!("docker restart {container}"),
        "containers.start" => format!("docker start {container}"),
        "containers.stop" => format!("docker stop {container}"),
        "containers.remove" => format!("docker rm {container}"),
        "images.pull" => format!(
            "docker pull {}",
            arg_str(args, "reference").unwrap_or_default()
        ),
        "images.build" => format!(
            "docker build -t {} {}",
            arg_str(args, "tag").unwrap_or_default(),
            arg_str(args, "context").unwrap_or_else(|| ".".to_string())
        ),
        "images.remove" => format!("docker image rm {container}"),
        "volumes.remove" => format!("docker volume rm {container}"),
        "system.prune" => "docker system prune".to_string(),
        _ => tool_summary(&call.tool, args),
    }
}

/// The effect an approval card renders from: its first line is the
/// summary sentence, then (for a "Then:" step) how it follows the
/// proposed change. The typed name rides along for Destructive calls.
#[must_use]
pub fn effect_for(
    call: &ToolCall,
    summary: &str,
    typed_name: Option<String>,
    after_plan: bool,
) -> EffectSummary {
    let mut lines = vec![EffectLine {
        kind: EffectKind::Changes,
        text: summary.to_string(),
    }];
    if after_plan {
        lines.push(EffectLine {
            kind: EffectKind::Changes,
            text: "Runs automatically with 'Write and recreate', or approve it here after \
                   'Write files only'."
                .to_string(),
        });
    }
    let verb = summary
        .split_whitespace()
        .next()
        .unwrap_or("Run")
        .to_string();
    let _ = call;
    EffectSummary {
        verb,
        noun_one: "change".to_string(),
        noun_many: "changes".to_string(),
        count: 1,
        bytes: None,
        lines,
        typed_name,
    }
}

/// Why Approve must wait, or `None` when it may run: an approval that
/// follows a proposed plan (`after_plan`) waits until that plan is
/// Written in Review changes (AD37).
#[must_use]
pub fn approve_blocked(approval: &Approval, session: Option<&AssistantSession>) -> Option<String> {
    let plan = approval.after_plan?;
    let state = session.and_then(|s| proposal_state(s, &plan));
    match state {
        Some(ProposalState::Written) => None,
        Some(ProposalState::Discarded | ProposalState::Gone) => Some(
            "The proposed change was not written, so this step has nothing to run on.".to_string(),
        ),
        _ => Some("Review the proposed change first — this runs after it.".to_string()),
    }
}

/// The state of one proposed change in a session's turns.
#[must_use]
pub fn proposal_state(session: &AssistantSession, change_id: &uuid::Uuid) -> Option<ProposalState> {
    session.turns.iter().rev().find_map(|turn| {
        turn.parts.iter().rev().find_map(|part| match part {
            TurnPart::ProposedChange {
                change_id: id,
                state,
                ..
            } if id == change_id => Some(*state),
            _ => None,
        })
    })
}

/// The newest still-pending proposed change of a session: a gated call
/// that arrives while one waits is its "Then:" step.
#[must_use]
pub fn pending_proposal(session: &AssistantSession) -> Option<uuid::Uuid> {
    session.turns.iter().rev().find_map(|turn| {
        turn.parts.iter().rev().find_map(|part| match part {
            TurnPart::ProposedChange {
                change_id,
                state: ProposalState::Pending,
                ..
            } => Some(*change_id),
            _ => None,
        })
    })
}

/// What Berth tells a harness before the first question. It names the
/// redaction guarantee (D79) so the model never asks for a value it
/// cannot be shown.
#[must_use]
pub fn system_prompt(project_name: &str) -> String {
    format!(
        "You are the assistant inside Berth, a desktop Docker manager for Colima on macOS. \
         You are helping with the project {project_name}; your working folder is its folder. \
         You reach Docker only through Berth's tools: reads run at once, changes wait for the \
         user to click Approve, destructive actions need the item's name typed, and file edits \
         go through files_propose_edit, which opens Berth's Review changes screen — a proposed \
         edit is never written until the user writes it there, so never claim a file changed. \
         .env values and container environment values are masked before they reach you and \
         stay on this Mac: you see key names and ports only, so never ask for a secret or try \
         to work around the masking. Answer plainly and briefly, say what you checked, and \
         prefer Berth's tools over guessing."
    )
}

// ---------------------------------------------------------------------------
// Folding the event stream into a session (pure)
// ---------------------------------------------------------------------------

fn assistant_turn(session: &mut AssistantSession, now: chrono::DateTime<chrono::Utc>) -> &mut Turn {
    if !matches!(session.turns.last(), Some(t) if t.role == TurnRole::Assistant) {
        session.turns.push(Turn {
            role: TurnRole::Assistant,
            parts: Vec::new(),
            at: now,
            origin: None,
        });
    }
    let index = session.turns.len() - 1;
    &mut session.turns[index]
}

fn find_tool<'a>(
    session: &'a mut AssistantSession,
    call_id: &str,
) -> Option<(&'a ToolCall, &'a mut ToolStatus)> {
    for turn in session.turns.iter_mut().rev() {
        for part in turn.parts.iter_mut().rev() {
            if let TurnPart::Tool { call, status } = part {
                if call.call_id == call_id {
                    return Some((call, status));
                }
            }
        }
    }
    None
}

fn mark_running_tools(session: &mut AssistantSession, message: &str) {
    for turn in &mut session.turns {
        for part in &mut turn.parts {
            if let TurnPart::Tool { status, .. } = part {
                if matches!(status, ToolStatus::Running | ToolStatus::WaitingForApproval) {
                    *status = ToolStatus::Failed {
                        message: message.to_string(),
                    };
                }
            }
        }
    }
}

/// Fold one harness event into the session. Streaming text appends to
/// the running text part; a tool call becomes a row whose finish carries
/// its milliseconds; an interrupt that a harness reports as an error
/// (the Claude adapter, P7-T3) rests the session instead of erroring it.
pub fn apply_event(
    session: &mut AssistantSession,
    event: &AssistantEvent,
    now: chrono::DateTime<chrono::Utc>,
) {
    match event {
        AssistantEvent::Ready { harness_session } => {
            session.harness_session = Some(harness_session.clone());
        }
        AssistantEvent::TextDelta { text } => {
            if text.is_empty() {
                return;
            }
            if session.state == SessionState::Idle {
                session.state = SessionState::Thinking;
            }
            let turn = assistant_turn(session, now);
            match turn.parts.last_mut() {
                Some(TurnPart::Text(existing)) => existing.push_str(text),
                _ => turn.parts.push(TurnPart::Text(text.clone())),
            }
        }
        AssistantEvent::TextDone => {}
        AssistantEvent::ToolStarted {
            call_id,
            tool,
            args,
        } => {
            if find_tool(session, call_id).is_some() {
                return;
            }
            if session.state == SessionState::Idle {
                session.state = SessionState::Thinking;
            }
            let turn = assistant_turn(session, now);
            turn.parts.push(TurnPart::Tool {
                call: ToolCall {
                    call_id: call_id.clone(),
                    tool: canonical_tool(tool),
                    args: args.clone(),
                    risk: tool_risk(tool),
                },
                status: ToolStatus::Running,
            });
        }
        AssistantEvent::ToolFinished { call_id, ok, ms } => {
            if let Some((call, status)) = find_tool(session, call_id) {
                if matches!(status, ToolStatus::Denied) && !ok {
                    return;
                }
                let summary = tool_summary(&call.tool, &call.args);
                *status = if *ok {
                    ToolStatus::Done { ms: *ms, summary }
                } else {
                    ToolStatus::Failed {
                        message: format!("{summary} — failed"),
                    }
                };
            }
        }
        AssistantEvent::TurnDone { .. } => {
            if !matches!(session.state, SessionState::WaitingForApproval(_)) {
                session.state = SessionState::Idle;
            }
        }
        AssistantEvent::Error {
            message,
            stderr_tail,
        } => {
            let text = format!("{message} {stderr_tail}").to_ascii_lowercase();
            mark_running_tools(session, "stopped");
            if text.contains("interrupt") {
                session.state = SessionState::Idle;
            } else {
                session.state = SessionState::Error {
                    stderr: if stderr_tail.trim().is_empty() {
                        message.clone()
                    } else {
                        stderr_tail.clone()
                    },
                };
            }
        }
    }
}

/// A session read back from the store: a turn Berth quit in the middle
/// of is not still thinking, and its running tools did not finish.
pub fn normalize_loaded(session: &mut AssistantSession) {
    if matches!(
        session.state,
        SessionState::Thinking | SessionState::WaitingForApproval(_)
    ) {
        session.state = SessionState::Idle;
    }
    mark_running_tools(session, "stopped when Berth quit");
}

/// Append a parked plan's card to the running assistant turn (AD37
/// summary only). A notice for a change already on the turn is ignored.
pub fn apply_proposal(
    session: &mut AssistantSession,
    notice: &ProposalNotice,
    now: chrono::DateTime<chrono::Utc>,
) {
    if proposal_state(session, &notice.plan_id).is_some() {
        return;
    }
    let turn = assistant_turn(session, now);
    turn.parts.push(TurnPart::ProposedChange {
        change_id: notice.plan_id,
        title: notice.title.clone(),
        files: notice.files.clone(),
        state: ProposalState::Pending,
    });
}

fn set_proposal_state(session: &mut AssistantSession, change_id: &uuid::Uuid, next: ProposalState) {
    for turn in &mut session.turns {
        for part in &mut turn.parts {
            if let TurnPart::ProposedChange {
                change_id: id,
                state,
                ..
            } = part
            {
                if id == change_id {
                    *state = next;
                }
            }
        }
    }
}

/// Bring proposal states in line with the PlanStore: a pending change
/// whose plan is gone is `Gone`; one marked gone whose plan is back
/// (seeded, or loaded before its store was) is pending again. Written
/// and Discarded never move.
pub fn reconcile_proposals(session: &mut AssistantSession, exists: &dyn Fn(&uuid::Uuid) -> bool) {
    for turn in &mut session.turns {
        for part in &mut turn.parts {
            if let TurnPart::ProposedChange {
                change_id, state, ..
            } = part
            {
                match state {
                    ProposalState::Pending if !exists(change_id) => {
                        *state = ProposalState::Gone;
                    }
                    ProposalState::Gone if exists(change_id) => {
                        *state = ProposalState::Pending;
                    }
                    _ => {}
                }
            }
        }
    }
}

/// The turn Watch and fix sends after a change is written (D71): the
/// harness re-reads the logs through its tools and may only propose.
/// `None` unless the session watches, rests idle, and the written
/// plan is one of its proposals. This is the whole watch loop's reach:
/// it produces a question, never an approval or a write.
#[must_use]
pub fn watch_follow_up(
    session: &AssistantSession,
    plan_id: &uuid::Uuid,
) -> Option<(String, String)> {
    if !session.watch || session.state != SessionState::Idle {
        return None;
    }
    let title = session
        .turns
        .iter()
        .flat_map(|turn| &turn.parts)
        .find_map(|part| match part {
            TurnPart::ProposedChange {
                change_id,
                title,
                state: ProposalState::Written,
                ..
            } if change_id == plan_id => Some(title.clone()),
            _ => None,
        })?;
    Some((
        format!(
            "The change \"{title}\" was written. Watch what happens now: read the newest logs and the problems list with your tools. If the problem is gone, say so and stop. If it is still there, propose the next smallest fix with files.propose_edit. Propose only; nothing is applied without my click."
        ),
        format!("Watch \u{b7} after writing {title}"),
    ))
}

/// The session of one project in the state's map, if it has been opened.
#[must_use]
pub fn session_for_project<'a>(
    sessions: &'a BTreeMap<SessionId, AssistantSession>,
    project: &ProjectId,
) -> Option<&'a AssistantSession> {
    sessions
        .values()
        .find(|s| s.project.as_ref() == Some(project))
}

// ---------------------------------------------------------------------------
// The service
// ---------------------------------------------------------------------------

/// What changed, for the UI-side applier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiMsg {
    Session(SessionId),
    Approvals,
}

/// Everything the service needs, none of it Dioxus.
pub struct AssistantDeps {
    pub store: Option<Arc<berth_store::Store>>,
    /// The executor core: session folders are registered on it.
    pub core: Option<Arc<ToolCore>>,
    /// How a harness reaches the tools; `None` runs no live harness.
    pub mcp: Option<McpLaunch>,
    pub approvals: Option<tokio::sync::mpsc::Receiver<ApprovalRequest>>,
    pub proposals: Option<UnboundedReceiver<ProposalNotice>>,
    pub cancel: CancellationToken,
}

/// Which project a session is opened for.
#[derive(Debug, Clone)]
pub struct OpenRequest {
    pub harness: Harness,
    pub project: ProjectId,
    pub project_name: String,
    pub folder: PathBuf,
    /// The model picked for this harness (AD39); `None` = its default.
    pub model: Option<String>,
}

/// A harness to run in a terminal the user drives (P7-T6): the same
/// bridge and tools, their hands on the keyboard. `cleanup` is the
/// 0600 mcp-config file to remove when the terminal session ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLaunch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub cleanup: Option<PathBuf>,
}

/// The user's answer to one approval card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Approve { typed: Option<String> },
    Deny,
}

enum Cmd {
    Send(UserInput),
    Interrupt,
}

struct LiveHandle {
    cmd: tokio::sync::mpsc::Sender<Cmd>,
    cancel: CancellationToken,
}

struct Pending {
    approval: Approval,
    request: ApprovalRequest,
}

struct Inner {
    store: Option<Arc<berth_store::Store>>,
    core: Option<Arc<ToolCore>>,
    mcp: Option<McpLaunch>,
    cancel: CancellationToken,
    ui: UnboundedSender<UiMsg>,
    sessions: Mutex<BTreeMap<SessionId, AssistantSession>>,
    by_folder: Mutex<BTreeMap<PathBuf, SessionId>>,
    live: Mutex<BTreeMap<SessionId, LiveHandle>>,
    pending: Mutex<Vec<Pending>>,
    /// Model catalogs by harness id, fetched once per app run (AD40).
    models: Mutex<BTreeMap<String, Vec<berth_assistant::ModelChoice>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Inner {
    fn notify_session(&self, id: SessionId) {
        let _ = self.ui.send(UiMsg::Session(id));
    }

    fn notify_approvals(&self) {
        let _ = self.ui.send(UiMsg::Approvals);
    }

    fn snapshot(&self, id: &SessionId) -> Option<AssistantSession> {
        lock(&self.sessions).get(id).cloned()
    }

    /// Save a session snapshot. Called from the session's own task at
    /// turn boundaries, so saves stay in order.
    fn persist(&self, id: &SessionId) {
        let (Some(store), Some(session)) = (&self.store, self.snapshot(id)) else {
            return;
        };
        if let Err(err) = store.save_session(&session) {
            tracing::warn!(%err, "saving the assistant session failed");
        }
    }

    /// Rest the session's waiting state on its oldest pending approval.
    fn sync_waiting(&self, id: &SessionId) {
        let first = lock(&self.pending)
            .iter()
            .find(|p| &p.approval.session == id)
            .map(|p| p.approval.call.clone());
        let mut sessions = lock(&self.sessions);
        let Some(session) = sessions.get_mut(id) else {
            return;
        };
        match first {
            Some(call) => session.state = SessionState::WaitingForApproval(call),
            None => {
                if matches!(session.state, SessionState::WaitingForApproval(_)) {
                    let running = session.turns.last().is_some_and(|turn| {
                        turn.parts.iter().any(|part| {
                            matches!(
                                part,
                                TurnPart::Tool {
                                    status: ToolStatus::Running,
                                    ..
                                }
                            )
                        })
                    });
                    session.state = if running {
                        SessionState::Thinking
                    } else {
                        SessionState::Idle
                    };
                }
            }
        }
    }
}

/// The assistant service. Cheap to clone; provide it as context from the
/// App root next to `Services`.
#[derive(Clone)]
pub struct AssistantService {
    inner: Arc<Inner>,
}

impl AssistantService {
    /// A service plus the receiver of its [`UiMsg`]s. [`start`] wires the
    /// receiver to the signals; tests drain it by hand or read snapshots.
    ///
    /// [`start`]: AssistantService::start
    #[must_use]
    pub fn new(deps: AssistantDeps) -> (Self, UnboundedReceiver<UiMsg>) {
        let (ui, rx) = unbounded_channel();
        let service = Self {
            inner: Arc::new(Inner {
                store: deps.store,
                core: deps.core,
                mcp: deps.mcp,
                cancel: deps.cancel,
                ui,
                sessions: Mutex::new(BTreeMap::new()),
                by_folder: Mutex::new(BTreeMap::new()),
                live: Mutex::new(BTreeMap::new()),
                pending: Mutex::new(Vec::new()),
                models: Mutex::new(BTreeMap::new()),
            }),
        };
        if let Some(approvals) = deps.approvals {
            let inner = Arc::clone(&service.inner);
            spawn_io(async move { run_approvals(inner, approvals).await });
        }
        if let Some(proposals) = deps.proposals {
            let inner = Arc::clone(&service.inner);
            spawn_io(async move { run_proposals(inner, proposals).await });
        }
        (service, rx)
    }

    /// A service that holds state but runs nothing (a scene root has no
    /// boot; the panel falls back to plain signal writes without one —
    /// this exists for tests that want the type).
    #[must_use]
    pub fn inert() -> Self {
        let (service, _rx) = Self::new(AssistantDeps {
            store: None,
            core: None,
            mcp: None,
            approvals: None,
            proposals: None,
            cancel: CancellationToken::new(),
        });
        service
    }

    /// Start from the App root (call inside `use_hook`): builds the
    /// service and spawns the Dioxus task that applies each [`UiMsg`]
    /// to `state.assistant` and `state.approvals`.
    #[must_use]
    pub fn start(mut state: AppState, deps: AssistantDeps) -> Self {
        use dioxus::prelude::{spawn, ReadableExt, WritableExt};
        let (service, mut rx) = Self::new(deps);
        let applier = service.clone();
        spawn(async move {
            while let Some(msg) = rx.recv().await {
                match msg {
                    UiMsg::Session(id) => {
                        if let Some(session) = applier.session_snapshot(&id) {
                            if state.assistant.peek().get(&id) != Some(&session) {
                                state.assistant.write().insert(id, session);
                            }
                        }
                    }
                    UiMsg::Approvals => {
                        let approvals = applier.approvals_snapshot();
                        if *state.approvals.peek() != approvals {
                            state.approvals.set(approvals);
                        }
                    }
                }
            }
        });
        service
    }

    #[must_use]
    pub fn session_snapshot(&self, id: &SessionId) -> Option<AssistantSession> {
        self.inner.snapshot(id)
    }

    /// Pending approvals, oldest first — the value `state.approvals`
    /// mirrors.
    #[must_use]
    pub fn approvals_snapshot(&self) -> Vec<Approval> {
        lock(&self.inner.pending)
            .iter()
            .map(|p| p.approval.clone())
            .collect()
    }

    /// The models a harness offers, asked of the harness itself and
    /// cached for the app run (AD40). An error leaves the picker with
    /// Default + Custom.
    ///
    /// # Errors
    /// A sentence when the harness cannot be asked.
    pub async fn list_models(
        &self,
        harness: &Harness,
    ) -> Result<Vec<berth_assistant::ModelChoice>, String> {
        if let Some(cached) = lock(&self.inner.models).get(&harness.id.0).cloned() {
            return Ok(cached);
        }
        let path = super::login_path::search_path().await.path_var();
        let models = berth_assistant::list_models(harness, path, self.inner.cancel.child_token())
            .await
            .map_err(|e| e.to_string())?;
        lock(&self.inner.models).insert(harness.id.0.clone(), models.clone());
        Ok(models)
    }

    /// Open (or reopen) the session for a project folder: the stored
    /// conversation comes back when there is one, and its harness
    /// session id resumes the harness side. Idempotent per folder.
    ///
    /// # Errors
    /// A sentence when no live harness can run (no bridge in the deps).
    pub async fn open(&self, req: OpenRequest) -> Result<SessionId, String> {
        let known = lock(&self.inner.by_folder).get(&req.folder).copied();
        if let Some(id) = known {
            self.ensure_live(&req, id).await;
            return Ok(id);
        }
        let mut session = self
            .inner
            .store
            .as_ref()
            .and_then(|store| store.sessions(Some(&req.project)).ok())
            .and_then(|records| records.into_iter().next().map(|r| r.session))
            .unwrap_or_else(|| AssistantSession {
                id: SessionId(uuid::Uuid::now_v7()),
                harness: req.harness.id.clone(),
                harness_session: None,
                project: Some(req.project.clone()),
                context: Vec::new(),
                turns: Vec::new(),
                state: SessionState::Idle,
                watch: false,
            });
        normalize_loaded(&mut session);
        // A stored session may name another harness than the picked one;
        // a fresh harness cannot resume it, so only the turns carry over.
        if session.harness != req.harness.id {
            session.harness = req.harness.id.clone();
            session.harness_session = None;
        }
        let id = session.id;
        lock(&self.inner.by_folder).insert(req.folder.clone(), id);
        lock(&self.inner.sessions).insert(id, session);
        self.inner.notify_session(id);
        self.ensure_live(&req, id).await;
        Ok(id)
    }

    /// "Open in Terminal": the harness TUI for this project with
    /// Berth's tools attached through the same bridge, resuming the
    /// harness conversation when it left an id. The session is opened
    /// first so its folder is registered with the tool executor.
    /// muse's interactive TUI has no verified way to attach the
    /// `berth` MCP server (its MSP session config serves `muse
    /// serve`, not the TUI), so it says so instead of launching a
    /// harness without the tools.
    ///
    /// # Errors
    /// A sentence when no bridge runs or the harness cannot take the
    /// tools into a terminal.
    pub async fn terminal_launch(&self, req: OpenRequest) -> Result<TerminalLaunch, String> {
        let id = self.open(req.clone()).await?;
        let Some(mcp) = self.inner.mcp.clone() else {
            return Err("the assistant bridge is not running".to_string());
        };
        if req.harness.transport != berth_core::HarnessTransport::ClaudeStreamJson {
            return Err(format!(
                "{} cannot take Berth's tools into a terminal yet",
                req.harness.name
            ));
        }
        let resume = self.inner.snapshot(&id).and_then(|s| s.harness_session);
        let cfg = SessionConfig {
            harness: req.harness.clone(),
            session: id,
            workspace: req.folder.clone(),
            resume: resume.clone(),
            mcp,
            path: super::login_path::search_path().await.path_var(),
            system_prompt: system_prompt(&req.project_name),
            model: req.model.clone(),
        };
        let config =
            berth_assistant::claude::write_terminal_mcp_config(&cfg).map_err(|e| e.to_string())?;
        let mut args = vec![
            "--mcp-config".to_string(),
            config.to_string_lossy().into_owned(),
            "--strict-mcp-config".to_string(),
        ];
        if let Some(model) = req.model.clone().filter(|m| !m.trim().is_empty()) {
            args.push("--model".to_string());
            args.push(model);
        }
        if let Some(resume) = resume {
            args.push("--resume".to_string());
            args.push(resume);
        }
        Ok(TerminalLaunch {
            program: req.harness.binary.clone(),
            args,
            cwd: req.folder.clone(),
            cleanup: Some(config),
        })
    }

    async fn ensure_live(&self, req: &OpenRequest, id: SessionId) {
        if lock(&self.inner.live).contains_key(&id) {
            return;
        }
        let Some(mcp) = self.inner.mcp.clone() else {
            return;
        };
        if let Some(core) = &self.inner.core {
            core.register_session_folder(id, req.folder.clone());
        }
        let resume = self.inner.snapshot(&id).and_then(|s| s.harness_session);
        let cfg = SessionConfig {
            harness: req.harness.clone(),
            session: id,
            workspace: req.folder.clone(),
            resume,
            mcp,
            path: super::login_path::search_path().await.path_var(),
            system_prompt: system_prompt(&req.project_name),
            model: req.model.clone(),
        };
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(8);
        let child = self.inner.cancel.child_token();
        lock(&self.inner.live).insert(
            id,
            LiveHandle {
                cmd: cmd_tx,
                cancel: child.clone(),
            },
        );
        let inner = Arc::clone(&self.inner);
        spawn_io(async move { run_live(inner, id, cfg, cmd_rx, child).await });
    }

    /// Ask a question: the user turn lands at once, the answer streams.
    /// `origin` is the entry-point line shown above the turn (AD37).
    ///
    /// # Errors
    /// A sentence when the session is unknown or its harness is gone.
    pub fn send(
        &self,
        session: SessionId,
        text: String,
        origin: Option<String>,
    ) -> Result<(), String> {
        let context = {
            let mut sessions = lock(&self.inner.sessions);
            let Some(found) = sessions.get_mut(&session) else {
                return Err("this conversation is gone".to_string());
            };
            found.turns.push(Turn {
                role: TurnRole::User,
                parts: vec![TurnPart::Text(text.clone())],
                at: chrono::Utc::now(),
                origin,
            });
            found.state = SessionState::Thinking;
            found.context.clone()
        };
        self.inner.notify_session(session);
        let cmd = lock(&self.inner.live)
            .get(&session)
            .map(|live| live.cmd.clone());
        let Some(cmd) = cmd else {
            self.set_error(
                session,
                "the assistant is not running — use Retry".to_string(),
            );
            return Err("the assistant is not running".to_string());
        };
        cmd.try_send(Cmd::Send(UserInput { text, context }))
            .map_err(|_| "the assistant is busy".to_string())
    }

    /// Stop the running turn (⌘., D77). A session that is not running
    /// ignores it.
    pub fn interrupt(&self, session: SessionId) {
        let cmd = lock(&self.inner.live)
            .get(&session)
            .map(|live| live.cmd.clone());
        if let Some(cmd) = cmd {
            let _ = cmd.try_send(Cmd::Interrupt);
        }
    }

    /// Restart a session's harness after an error, resuming the harness
    /// conversation when it left an id behind.
    ///
    /// # Errors
    /// A sentence when the session is unknown.
    pub async fn retry(&self, session: SessionId, req: OpenRequest) -> Result<(), String> {
        if self.inner.snapshot(&session).is_none() {
            return Err("this conversation is gone".to_string());
        }
        if let Some(live) = lock(&self.inner.live).remove(&session) {
            live.cancel.cancel();
        }
        {
            let mut sessions = lock(&self.inner.sessions);
            if let Some(found) = sessions.get_mut(&session) {
                if matches!(found.state, SessionState::Error { .. }) {
                    found.state = SessionState::Idle;
                }
            }
        }
        self.inner.notify_session(session);
        self.ensure_live(&req, session).await;
        Ok(())
    }

    fn set_error(&self, session: SessionId, stderr: String) {
        let mut sessions = lock(&self.inner.sessions);
        if let Some(found) = sessions.get_mut(&session) {
            found.state = SessionState::Error { stderr };
        }
        drop(sessions);
        self.inner.notify_session(session);
    }

    /// Answer one approval card. Approving a "Then:" step before its
    /// plan is written, and approving a Destructive call whose typed
    /// name does not match, are refused here and never reach the
    /// executor as an approval.
    ///
    /// # Errors
    /// A sentence naming why the answer was refused; the card stays.
    pub fn answer(&self, id: uuid::Uuid, answer: Answer) -> Result<(), String> {
        let approval = lock(&self.inner.pending)
            .iter()
            .find(|p| p.approval.id == id)
            .map(|p| p.approval.clone())
            .ok_or_else(|| "this approval is gone".to_string())?;
        if let Answer::Approve { typed } = &answer {
            if let Some(expected) = &approval.typed_name {
                if typed.as_deref() != Some(expected.as_str()) {
                    return Err(format!("Type {expected} to confirm"));
                }
            }
            let session = self.inner.snapshot(&approval.session);
            if let Some(reason) = approve_blocked(&approval, session.as_ref()) {
                return Err(reason);
            }
        }
        let pending = {
            let mut pending = lock(&self.inner.pending);
            let position = pending.iter().position(|p| p.approval.id == id);
            position.map(|index| pending.remove(index))
        };
        let Some(pending) = pending else {
            return Err("this approval is gone".to_string());
        };
        let decision = match &answer {
            Answer::Approve { typed } => Decision::Approved {
                typed_name: typed.clone(),
            },
            Answer::Deny => Decision::Denied {
                reason: "the user denied it in Berth".to_string(),
            },
        };
        pending.request.answer(decision);
        // The tool row follows the answer: denied stays denied (the
        // harness's own finish must not overwrite it), approved runs.
        let mut sessions = lock(&self.inner.sessions);
        if let Some(session) = sessions.get_mut(&approval.session) {
            if let Some((_, status)) = find_tool(session, &approval.call.call_id) {
                *status = match answer {
                    Answer::Approve { .. } => ToolStatus::Running,
                    Answer::Deny => ToolStatus::Denied,
                };
            }
        }
        drop(sessions);
        self.inner.sync_waiting(&approval.session);
        self.inner.notify_session(approval.session);
        self.inner.notify_approvals();
        Ok(())
    }

    /// The Watch and fix switch (the watch loop itself is P7-T6).
    pub fn set_watch(&self, session: SessionId, on: bool) {
        let mut sessions = lock(&self.inner.sessions);
        if let Some(found) = sessions.get_mut(&session) {
            found.watch = on;
        }
        drop(sessions);
        self.inner.notify_session(session);
    }

    /// Drop one attached context item (its chip's ×).
    pub fn remove_context(&self, session: SessionId, index: usize) {
        let mut sessions = lock(&self.inner.sessions);
        if let Some(found) = sessions.get_mut(&session) {
            if index < found.context.len() {
                found.context.remove(index);
            }
        }
        drop(sessions);
        self.inner.notify_session(session);
    }

    /// Attach context to a session (the P7-T6 entry points land here).
    pub fn add_context(&self, session: SessionId, items: Vec<ContextItem>) {
        let mut sessions = lock(&self.inner.sessions);
        if let Some(found) = sessions.get_mut(&session) {
            found.context.extend(items);
        }
        drop(sessions);
        self.inner.notify_session(session);
    }

    /// Review changes wrote this plan: its card rests as Written, which
    /// is what unblocks a "Then:" approval. A watching session then
    /// gets its follow-up turn (D71: it proposes, never applies).
    pub fn plan_written(&self, plan_id: uuid::Uuid) {
        let mut changed = Vec::new();
        let mut follow_ups = Vec::new();
        let mut sessions = lock(&self.inner.sessions);
        for (id, session) in &mut *sessions {
            if proposal_state(session, &plan_id).is_some() {
                set_proposal_state(session, &plan_id, ProposalState::Written);
                changed.push(*id);
                if let Some(follow_up) = watch_follow_up(session, &plan_id) {
                    follow_ups.push((*id, follow_up));
                }
            }
        }
        drop(sessions);
        for id in changed {
            self.inner.notify_session(id);
        }
        for (id, (text, origin)) in follow_ups {
            if let Err(reason) = self.send(id, text, Some(origin)) {
                tracing::warn!(%reason, "the watch follow-up could not be sent");
            }
        }
    }

    /// Line proposal states up with the plans that still exist.
    pub fn reconcile_plans(&self, plans: &PlanStore) {
        let mut changed = Vec::new();
        let mut sessions = lock(&self.inner.sessions);
        for (id, session) in &mut *sessions {
            let before: Vec<ProposalState> = session
                .turns
                .iter()
                .flat_map(|t| &t.parts)
                .filter_map(|p| match p {
                    TurnPart::ProposedChange { state, .. } => Some(*state),
                    _ => None,
                })
                .collect();
            reconcile_proposals(session, &|change_id| plans.contains(change_id));
            let after: Vec<ProposalState> = session
                .turns
                .iter()
                .flat_map(|t| &t.parts)
                .filter_map(|p| match p {
                    TurnPart::ProposedChange { state, .. } => Some(*state),
                    _ => None,
                })
                .collect();
            if before != after {
                changed.push(*id);
            }
        }
        drop(sessions);
        for id in changed {
            self.inner.notify_session(id);
        }
    }
}

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

/// The executor's gated calls become `state.approvals` entries, oldest
/// first. A call that arrives while a proposed change of the same
/// session is still pending is that change's "Then:" step.
async fn run_approvals(
    inner: Arc<Inner>,
    mut requests: tokio::sync::mpsc::Receiver<ApprovalRequest>,
) {
    while let Some(request) = requests.recv().await {
        let session_id = request.session;
        let after_plan = inner
            .snapshot(&session_id)
            .and_then(|s| pending_proposal(&s));
        let approval = Approval {
            id: request.id,
            session: session_id,
            call: request.call.clone(),
            effect: effect_for(
                &request.call,
                &request.summary,
                request.typed_name.clone(),
                after_plan.is_some(),
            ),
            typed_name: request.typed_name.clone(),
            after_plan,
        };
        let call_id = approval.call.call_id.clone();
        lock(&inner.pending).push(Pending { approval, request });
        {
            let mut sessions = lock(&inner.sessions);
            if let Some(session) = sessions.get_mut(&session_id) {
                if let Some((_, status)) = find_tool(session, &call_id) {
                    *status = ToolStatus::WaitingForApproval;
                }
            }
        }
        inner.sync_waiting(&session_id);
        inner.notify_session(session_id);
        inner.notify_approvals();
    }
}

/// Parked plans become proposed-change cards on the running turn.
async fn run_proposals(inner: Arc<Inner>, mut notices: UnboundedReceiver<ProposalNotice>) {
    while let Some(notice) = notices.recv().await {
        let mut sessions = lock(&inner.sessions);
        if let Some(session) = sessions.get_mut(&notice.session) {
            apply_proposal(session, &notice, chrono::Utc::now());
        }
        drop(sessions);
        inner.notify_session(notice.session);
    }
}

/// One harness process: commands in, events folded into the session,
/// persisted when a turn ends (and when the harness names its session,
/// so a quit mid-turn still resumes).
async fn run_live(
    inner: Arc<Inner>,
    id: SessionId,
    cfg: SessionConfig,
    mut cmd_rx: tokio::sync::mpsc::Receiver<Cmd>,
    cancel: CancellationToken,
) {
    let started = berth_assistant::start(cfg, cancel.clone()).await;
    let (mut harness, mut events) = match started {
        Ok(pair) => pair,
        Err(err) => {
            let stderr = match &err {
                berth_assistant::AssistantError::Exited { stderr_tail, .. } => stderr_tail.clone(),
                other => other.to_string(),
            };
            let mut sessions = lock(&inner.sessions);
            if let Some(session) = sessions.get_mut(&id) {
                session.state = SessionState::Error { stderr };
            }
            drop(sessions);
            inner.persist(&id);
            inner.notify_session(id);
            lock(&inner.live).remove(&id);
            return;
        }
    };
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(Cmd::Send(input)) => {
                        if let Err(err) = harness.send(input).await {
                            set_live_error(&inner, &id, &err.to_string());
                        }
                    }
                    Some(Cmd::Interrupt) => {
                        if let Err(err) = harness.interrupt().await {
                            tracing::warn!(%err, "stopping the assistant turn failed");
                        }
                    }
                    None => break,
                }
            }
            event = events.recv() => {
                let Some(event) = event else { break };
                let boundary = matches!(
                    event,
                    AssistantEvent::Ready { .. }
                        | AssistantEvent::TurnDone { .. }
                        | AssistantEvent::Error { .. }
                );
                {
                    let mut sessions = lock(&inner.sessions);
                    if let Some(session) = sessions.get_mut(&id) {
                        apply_event(session, &event, chrono::Utc::now());
                    }
                }
                if matches!(event, AssistantEvent::TurnDone { .. }) {
                    inner.sync_waiting(&id);
                }
                if boundary {
                    inner.persist(&id);
                }
                inner.notify_session(id);
            }
        }
    }
    harness.close().await;
    lock(&inner.live).remove(&id);
}

fn set_live_error(inner: &Inner, id: &SessionId, stderr: &str) {
    let mut sessions = lock(&inner.sessions);
    if let Some(session) = sessions.get_mut(id) {
        mark_running_tools(session, "stopped");
        session.state = SessionState::Error {
            stderr: stderr.to_string(),
        };
    }
    drop(sessions);
    inner.persist(id);
    inner.notify_session(*id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::backing::Backing;
    use crate::services::tools::{ApprovalHub, ToolDeps};
    use berth_assistant::bridge::{BridgeRequest, BridgeResponse};
    use berth_core::{EngineId, HarnessId, HarnessTransport};
    use serde_json::json;
    use std::time::Duration;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_800_000_000, 0).expect("ts")
    }

    fn empty_session() -> AssistantSession {
        AssistantSession {
            id: SessionId(uuid::Uuid::now_v7()),
            harness: HarnessId("claude-code".to_string()),
            harness_session: None,
            project: Some(ProjectId("p1".to_string())),
            context: Vec::new(),
            turns: Vec::new(),
            state: SessionState::Idle,
            watch: false,
        }
    }

    #[test]
    fn assistant_tool_rows_say_what_the_tool_did() {
        // The three rows of the frame, word for word.
        assert_eq!(
            tool_summary(
                "containers.logs",
                &json!({"container": "shop-api-worker-1", "tail": 200})
            ),
            "Read 200 lines of shop-api-worker-1 logs"
        );
        assert_eq!(
            tool_summary("containers.inspect", &json!({"container": "shop-api-db-1"})),
            "Inspected shop-api-db-1"
        );
        assert_eq!(
            tool_summary("files.read", &json!({"path": "compose.yaml"})),
            "Read compose.yaml"
        );
        // The defaults and the wire spelling a harness uses.
        assert_eq!(
            tool_summary("containers.logs", &json!({"container": "api-1"})),
            "Read 200 lines of api-1 logs"
        );
        assert_eq!(
            tool_summary(
                "mcp__berth__containers_stats",
                &json!({"container": "api-1"})
            ),
            "Checked CPU and memory of api-1"
        );
        assert_eq!(
            tool_summary(
                "compose.up",
                &json!({"project": "shop-api", "services": ["db", "worker"]})
            ),
            "Started db, worker in shop-api"
        );
        assert_eq!(tool_summary("problems.list", &json!({})), "Listed problems");
    }

    #[test]
    fn assistant_command_line_is_the_frame_command() {
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool: "compose.up".to_string(),
            args: json!({"project": "shop-api", "services": ["db", "worker"]}),
            risk: Risk::Change,
        };
        assert_eq!(command_line(&call), "docker compose up -d db worker");
        let effect = effect_for(&call, "Start project shop-api", None, true);
        assert_eq!(effect.lines.len(), 2);
        assert!(
            effect.lines[1].text.contains("Write and recreate"),
            "{}",
            effect.lines[1].text
        );
        let plain = effect_for(&call, "Start project shop-api", None, false);
        assert_eq!(plain.lines.len(), 1);
    }

    #[test]
    fn assistant_apply_event_streams_text_tools_and_errors() {
        let mut session = empty_session();
        session.turns.push(Turn {
            role: TurnRole::User,
            parts: vec![TurnPart::Text("why?".to_string())],
            at: now(),
            origin: Some("From Logs \u{b7} Ask about 2 lines".to_string()),
        });
        apply_event(
            &mut session,
            &AssistantEvent::Ready {
                harness_session: "hs-1".to_string(),
            },
            now(),
        );
        assert_eq!(session.harness_session.as_deref(), Some("hs-1"));
        apply_event(
            &mut session,
            &AssistantEvent::TextDelta {
                text: "The worker ".to_string(),
            },
            now(),
        );
        apply_event(
            &mut session,
            &AssistantEvent::TextDelta {
                text: "restarts.".to_string(),
            },
            now(),
        );
        apply_event(
            &mut session,
            &AssistantEvent::ToolStarted {
                call_id: "t1".to_string(),
                tool: "mcp__berth__containers_logs".to_string(),
                args: json!({"container": "w-1", "tail": 50}),
            },
            now(),
        );
        apply_event(
            &mut session,
            &AssistantEvent::ToolFinished {
                call_id: "t1".to_string(),
                ok: true,
                ms: 140,
            },
            now(),
        );
        assert_eq!(session.turns.len(), 2);
        let parts = &session.turns[1].parts;
        assert_eq!(parts.len(), 2, "{parts:?}");
        match &parts[0] {
            TurnPart::Text(text) => assert_eq!(text, "The worker restarts."),
            other => panic!("expected streamed text, got {other:?}"),
        }
        match &parts[1] {
            TurnPart::Tool { call, status } => {
                assert_eq!(call.tool, "containers.logs");
                assert_eq!(call.risk, Risk::Read);
                assert_eq!(
                    *status,
                    ToolStatus::Done {
                        ms: 140,
                        summary: "Read 50 lines of w-1 logs".to_string()
                    }
                );
            }
            other => panic!("expected a tool row, got {other:?}"),
        }
        assert_eq!(session.state, SessionState::Thinking);
        apply_event(
            &mut session,
            &AssistantEvent::TurnDone { stop_reason: None },
            now(),
        );
        assert_eq!(session.state, SessionState::Idle);

        // An interrupt reported as an error rests the session (P7-T3).
        apply_event(
            &mut session,
            &AssistantEvent::Error {
                message: "turn interrupted".to_string(),
                stderr_tail: String::new(),
            },
            now(),
        );
        assert_eq!(session.state, SessionState::Idle);
        // A real error keeps its stderr for the Retry card.
        apply_event(
            &mut session,
            &AssistantEvent::Error {
                message: "claude stopped".to_string(),
                stderr_tail: "boom on stderr".to_string(),
            },
            now(),
        );
        assert_eq!(
            session.state,
            SessionState::Error {
                stderr: "boom on stderr".to_string()
            }
        );

        // A denied tool stays denied when its finish arrives.
        let mut session = empty_session();
        apply_event(
            &mut session,
            &AssistantEvent::ToolStarted {
                call_id: "t9".to_string(),
                tool: "containers.restart".to_string(),
                args: json!({"container": "w-1"}),
            },
            now(),
        );
        if let Some((_, status)) = find_tool(&mut session, "t9") {
            *status = ToolStatus::Denied;
        }
        apply_event(
            &mut session,
            &AssistantEvent::ToolFinished {
                call_id: "t9".to_string(),
                ok: false,
                ms: 3,
            },
            now(),
        );
        match &session.turns[0].parts[0] {
            TurnPart::Tool { status, .. } => assert_eq!(*status, ToolStatus::Denied),
            other => panic!("expected the tool row, got {other:?}"),
        }

        // Loaded back, a mid-turn session is at rest, tools unfinished.
        session.state = SessionState::Thinking;
        if let Some((_, status)) = find_tool(&mut session, "t9") {
            *status = ToolStatus::Running;
        }
        normalize_loaded(&mut session);
        assert_eq!(session.state, SessionState::Idle);
        match &session.turns[0].parts[0] {
            TurnPart::Tool { status, .. } => assert!(matches!(status, ToolStatus::Failed { .. })),
            other => panic!("expected the tool row, got {other:?}"),
        }
    }

    fn approval_after(plan: Option<uuid::Uuid>) -> Approval {
        Approval {
            id: uuid::Uuid::now_v7(),
            session: SessionId(uuid::Uuid::nil()),
            call: ToolCall {
                call_id: "c".to_string(),
                tool: "compose.up".to_string(),
                args: json!({}),
                risk: Risk::Change,
            },
            effect: effect_for(
                &ToolCall {
                    call_id: "c".to_string(),
                    tool: "compose.up".to_string(),
                    args: json!({}),
                    risk: Risk::Change,
                },
                "Start project shop-api",
                None,
                plan.is_some(),
            ),
            typed_name: None,
            after_plan: plan,
        }
    }

    #[test]
    fn assistant_approve_waits_until_the_plan_is_written() {
        let plan_id = uuid::Uuid::now_v7();
        let approval = approval_after(Some(plan_id));
        let mut session = empty_session();
        let notice_session = session.id;
        apply_proposal(
            &mut session,
            &ProposalNotice {
                session: notice_session,
                plan_id,
                title: "Move db".to_string(),
                files: vec![(PathBuf::from("compose.yaml"), 9, 2)],
            },
            now(),
        );
        assert!(approve_blocked(&approval, Some(&session)).is_some());
        // Still in the store: pending stays pending.
        reconcile_proposals(&mut session, &|id| *id == plan_id);
        assert_eq!(
            proposal_state(&session, &plan_id),
            Some(ProposalState::Pending)
        );
        // Dropped from the store unwritten: gone, and still blocked.
        reconcile_proposals(&mut session, &|_| false);
        assert_eq!(
            proposal_state(&session, &plan_id),
            Some(ProposalState::Gone)
        );
        assert!(approve_blocked(&approval, Some(&session)).is_some());
        // Written is the only way through.
        set_proposal_state(&mut session, &plan_id, ProposalState::Written);
        assert!(approve_blocked(&approval, Some(&session)).is_none());
        // No earlier step: never blocked.
        assert!(approve_blocked(&approval_after(None), Some(&session)).is_none());
        // The system prompt carries the redaction guarantee.
        let prompt = system_prompt("shop-api");
        assert!(prompt.contains("shop-api"), "{prompt}");
        assert!(prompt.contains("key names and ports only"), "{prompt}");
        assert!(prompt.contains("Review changes"), "{prompt}");
    }

    // -- The service over the real executor -------------------------------

    fn test_harness(binary: PathBuf) -> Harness {
        Harness {
            id: HarnessId("claude-code".to_string()),
            name: "Claude Code".to_string(),
            binary,
            version: Some("9.9.9".to_string()),
            transport: HarnessTransport::ClaudeStreamJson,
            available: true,
        }
    }

    struct Rig {
        service: AssistantService,
        core: Arc<ToolCore>,
        plans: PlanStore,
        session: SessionId,
        _dir: tempfile::TempDir,
        _ui: UnboundedReceiver<UiMsg>,
    }

    async fn rig() -> Rig {
        let dir = tempfile::tempdir().expect("dir");
        let folder = dir.path().join("shop-api");
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(
            folder.join("compose.yaml"),
            "services:\n  web:\n    image: nginx\n",
        )
        .expect("compose");
        std::fs::write(folder.join(".env"), "PLAIN=1\n").expect("env");
        let world = berth_engine::MockWorld::bundled();
        let engine = EngineId("colima:default".to_string());
        let mock = berth_engine::MockEngine::from_world(world, &engine).expect("mock");
        let clients = crate::services::engines::client_map();
        clients.lock().expect("clients").insert(
            engine,
            Arc::new(mock) as Arc<dyn berth_engine::EngineClient>,
        );
        let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
        let plans = PlanStore::default();
        let (hub, approvals) = ApprovalHub::channel();
        let core = Arc::new(ToolCore::new(
            ToolDeps {
                clients,
                store: Arc::clone(&store),
                compose: Arc::new(berth_engine::MockCompose::bundled()),
                plans: plans.clone(),
                backing: Backing::Live,
                session_folder: Some(folder.clone()),
            },
            hub,
        ));
        let (service, ui) = AssistantService::new(AssistantDeps {
            store: Some(store),
            core: Some(Arc::clone(&core)),
            mcp: None,
            approvals: Some(approvals),
            proposals: core.take_proposals(),
            cancel: CancellationToken::new(),
        });
        let session = service
            .open(OpenRequest {
                harness: test_harness(PathBuf::from("/bin/true")),
                project: ProjectId("shop-api".to_string()),
                project_name: "shop-api".to_string(),
                folder,
                model: None,
            })
            .await
            .expect("open");
        Rig {
            service,
            core,
            plans,
            session,
            _dir: dir,
            _ui: ui,
        }
    }

    fn bridge(session: SessionId, tool: &str, args: serde_json::Value) -> BridgeRequest {
        BridgeRequest {
            id: uuid::Uuid::now_v7(),
            token: "t".to_string(),
            session,
            tool: tool.to_string(),
            args,
            call_id: Some(format!("call-{tool}")),
        }
    }

    /// Poll with a hard deadline; a hung wait fails the test, never hangs it.
    async fn wait_for<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(value) = probe() {
                    return value;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
    }

    async fn next_approval(service: &AssistantService) -> Approval {
        wait_for("an approval card", || {
            service.approvals_snapshot().into_iter().next()
        })
        .await
    }

    #[tokio::test]
    async fn assistant_approvals_flow_through_the_hub() {
        let rig = rig().await;
        // Approve: the gated call waits, the card appears oldest-first,
        // Approve runs it.
        let core = Arc::clone(&rig.core);
        let session = rig.session;
        let call = tokio::spawn(async move {
            core.handle(bridge(
                session,
                "containers.restart",
                json!({"container": "shop-api-api-1"}),
            ))
            .await
        });
        let approval = next_approval(&rig.service).await;
        assert_eq!(approval.call.tool, "containers.restart");
        assert_eq!(approval.typed_name, None);
        assert_eq!(approval.after_plan, None);
        wait_for("the waiting state", || {
            let s = rig.service.session_snapshot(&session)?;
            matches!(s.state, SessionState::WaitingForApproval(_)).then_some(())
        })
        .await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!call.is_finished(), "the restart ran without approval");
        rig.service
            .answer(approval.id, Answer::Approve { typed: None })
            .expect("approve");
        let response = tokio::time::timeout(Duration::from_secs(10), call)
            .await
            .expect("the approved call finishes")
            .expect("call task");
        match response {
            BridgeResponse::Ok { text, .. } => {
                assert!(text.contains("Restarted shop-api-api-1"), "{text}");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
        assert!(rig.service.approvals_snapshot().is_empty());

        // Deny: the executor is told no, with Berth's reason.
        let core = Arc::clone(&rig.core);
        let call = tokio::spawn(async move {
            core.handle(bridge(
                session,
                "containers.restart",
                json!({"container": "shop-api-api-1"}),
            ))
            .await
        });
        let approval = next_approval(&rig.service).await;
        rig.service.answer(approval.id, Answer::Deny).expect("deny");
        let response = tokio::time::timeout(Duration::from_secs(10), call)
            .await
            .expect("the denied call finishes")
            .expect("call task");
        match response {
            BridgeResponse::Denied { reason, .. } => {
                assert!(reason.contains("denied"), "{reason}");
            }
            other => panic!("expected Denied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn assistant_destructive_needs_the_exact_typed_name() {
        let rig = rig().await;
        let core = Arc::clone(&rig.core);
        let session = rig.session;
        let call = tokio::spawn(async move {
            core.handle(bridge(
                session,
                "containers.remove",
                json!({"name": "shop-api-db-1"}),
            ))
            .await
        });
        let approval = next_approval(&rig.service).await;
        assert_eq!(approval.typed_name.as_deref(), Some("shop-api-db-1"));
        // A wrong name is refused by the service: the card stays and the
        // executor never hears an approval.
        let refused = rig.service.answer(
            approval.id,
            Answer::Approve {
                typed: Some("shop-api-db-2".to_string()),
            },
        );
        assert!(refused.is_err(), "a wrong typed name answered");
        assert_eq!(rig.service.approvals_snapshot().len(), 1);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!call.is_finished(), "the removal ran on a wrong name");
        rig.service
            .answer(
                approval.id,
                Answer::Approve {
                    typed: Some("shop-api-db-1".to_string()),
                },
            )
            .expect("approve with the name");
        let response = tokio::time::timeout(Duration::from_secs(10), call)
            .await
            .expect("the removal finishes")
            .expect("call task");
        match response {
            BridgeResponse::Ok { text, .. } => {
                assert!(text.contains("Removed shop-api-db-1"), "{text}");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn assistant_proposal_becomes_a_card_and_gates_the_next_step() {
        let rig = rig().await;
        let session = rig.session;
        let response = rig
            .core
            .handle(bridge(
                session,
                "files.propose_edit",
                json!({
                    "path": "compose.yaml",
                    "content": "services:\n  web:\n    image: nginx:1.27\n",
                    "summary": "Pin nginx",
                }),
            ))
            .await;
        assert!(
            matches!(response, BridgeResponse::Ok { .. }),
            "{response:?}"
        );
        // The turn gains the card's summary (never the plan body), and the
        // plan sits in the one shared PlanStore.
        let proposal = wait_for("the proposed-change card", || {
            let s = rig.service.session_snapshot(&session)?;
            crate::screens::assistant::proposals_of(&s)
                .into_iter()
                .next()
        })
        .await;
        assert_eq!(proposal.title, "Pin nginx");
        assert_eq!(proposal.state, ProposalState::Pending);
        assert_eq!(proposal.files.len(), 1);
        assert!(rig.plans.contains(&proposal.change_id), "plan not parked");
        assert!(
            std::fs::read_to_string(rig._dir.path().join("shop-api/compose.yaml"))
                .expect("compose on disk")
                .contains("image: nginx\n"),
            "propose_edit wrote the file"
        );

        // The next gated call is its "Then:" step: Approve waits.
        let core = Arc::clone(&rig.core);
        let call = tokio::spawn(async move {
            core.handle(bridge(
                session,
                "containers.restart",
                json!({"container": "shop-api-api-1"}),
            ))
            .await
        });
        let approval = next_approval(&rig.service).await;
        assert_eq!(approval.after_plan, Some(proposal.change_id));
        assert!(
            rig.service
                .answer(approval.id, Answer::Approve { typed: None })
                .is_err(),
            "a Then: step approved before its plan was written"
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!call.is_finished(), "the Then: step ran early");
        // Review changes writes it; the step may now run.
        rig.service.plan_written(proposal.change_id);
        rig.service
            .answer(approval.id, Answer::Approve { typed: None })
            .expect("approve after the write");
        let response = tokio::time::timeout(Duration::from_secs(10), call)
            .await
            .expect("the step finishes")
            .expect("call task");
        assert!(
            matches!(response, BridgeResponse::Ok { .. }),
            "{response:?}"
        );
        let s = rig.service.session_snapshot(&session).expect("session");
        assert_eq!(
            proposal_state(&s, &proposal.change_id),
            Some(ProposalState::Written)
        );
    }

    // -- A fake harness end to end (the P7-T3 replay pattern) ------------

    /// A shell script standing in for `claude`: prints its own init,
    /// then replays the recorded fixture per user line. Paths are baked
    /// in because the child gets no custom env.
    fn fake_script(dir: &std::path::Path, replay: &str) -> (PathBuf, PathBuf) {
        std::env::set_var("BERTH_DATA_DIR", dir.join("data"));
        let replay_file = dir.join("replay.jsonl");
        let replay = if replay.is_empty() || replay.ends_with('\n') {
            replay.to_string()
        } else {
            format!("{replay}\n")
        };
        std::fs::write(&replay_file, replay).expect("replay");
        let args_file = dir.join("args.txt");
        let script = dir.join("fake-claude.sh");
        let body = format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$@\" > '{args}'\n\
             printf '%s\\n' '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"fake-1\"}}'\n\
             while IFS= read -r line; do\n\
               case \"$line\" in\n\
                 *interrupt*) printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"stop_reason\":\"interrupted\"}}' ;;\n\
                 *) cat '{replay}' ;;\n\
               esac\n\
             done\n",
            args = args_file.display(),
            replay = replay_file.display(),
        );
        std::fs::write(&script, body).expect("script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        (script, args_file)
    }

    fn fixture_replay() -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/assistant/claude/list-containers.jsonl");
        std::fs::read_to_string(path)
            .expect("fixture")
            .lines()
            .filter(|line| !line.contains("\"subtype\":\"init\""))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn live_deps(
        store: Arc<berth_store::Store>,
        dir: &std::path::Path,
        cancel: CancellationToken,
    ) -> AssistantDeps {
        AssistantDeps {
            store: Some(store),
            core: None,
            mcp: Some(McpLaunch {
                berth_binary: dir.join("berth"),
                bridge: dir.join("bridge.sock"),
                token: "test-token".to_string(),
            }),
            approvals: None,
            proposals: None,
            cancel,
        }
    }

    fn answer_text(session: &AssistantSession) -> String {
        session
            .turns
            .iter()
            .filter(|t| t.role == TurnRole::Assistant)
            .flat_map(|t| &t.parts)
            .filter_map(|p| match p {
                TurnPart::Text(text) => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    #[tokio::test]
    async fn assistant_session_streams_saves_and_resumes() {
        let dir = tempfile::tempdir().expect("dir");
        let (script, args_file) = fake_script(dir.path(), &fixture_replay());
        let folder = dir.path().join("shop-api");
        std::fs::create_dir_all(&folder).expect("folder");
        let db = dir.path().join("berth.db");
        let project = ProjectId("shop-api".to_string());
        let request = OpenRequest {
            harness: test_harness(script),
            project: project.clone(),
            project_name: "shop-api".to_string(),
            folder: folder.clone(),
            model: None,
        };

        let cancel = CancellationToken::new();
        let store = Arc::new(berth_store::Store::open(&db).expect("store"));
        let (service, _ui) = AssistantService::new(live_deps(store, dir.path(), cancel.clone()));
        let id = service.open(request.clone()).await.expect("open");
        service
            .send(id, "List the containers".to_string(), None)
            .expect("send");
        let done = wait_for("the streamed answer", || {
            let s = service.session_snapshot(&id)?;
            (s.state == SessionState::Idle && answer_text(&s).contains("Three containers"))
                .then_some(s)
        })
        .await;
        assert_eq!(done.harness_session.as_deref(), Some("fake-1"));
        assert_eq!(done.turns.len(), 2, "{done:?}");
        assert_eq!(done.turns[0].role, TurnRole::User);
        let tools: Vec<&ToolStatus> = done.turns[1]
            .parts
            .iter()
            .filter_map(|p| match p {
                TurnPart::Tool { status, .. } => Some(status),
                _ => None,
            })
            .collect();
        assert_eq!(tools.len(), 1, "{done:?}");
        assert!(matches!(tools[0], ToolStatus::Done { .. }), "{tools:?}");
        drop(service);
        cancel.cancel();

        // Saved on the turn's end: a fresh store over the same file
        // reads the whole conversation back.
        let reopened = berth_store::Store::open(&db).expect("reopen");
        let records = reopened.sessions(Some(&project)).expect("sessions");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session.id, id);
        assert_eq!(records[0].session.turns, done.turns);
        assert_eq!(
            records[0].session.harness_session.as_deref(),
            Some("fake-1")
        );

        // Reopening resumes: same session, same turns, and the harness
        // is started with the stored resume id.
        let cancel2 = CancellationToken::new();
        let (service2, _ui2) =
            AssistantService::new(live_deps(Arc::new(reopened), dir.path(), cancel2.clone()));
        let id2 = service2.open(request).await.expect("reopen session");
        assert_eq!(id2, id);
        let resumed = service2.session_snapshot(&id2).expect("resumed");
        assert_eq!(resumed.turns, done.turns);
        let argv = wait_for("the resume argument", || {
            let text = std::fs::read_to_string(&args_file).ok()?;
            text.contains("--resume\nfake-1").then_some(text)
        })
        .await;
        assert!(argv.contains("fake-1"), "{argv}");
        drop(service2);
        cancel2.cancel();
    }

    #[tokio::test]
    async fn assistant_error_state_and_retry() {
        let dir = tempfile::tempdir().expect("dir");
        std::env::set_var("BERTH_DATA_DIR", dir.path().join("data"));
        let bad = dir.path().join("bad-claude.sh");
        std::fs::write(&bad, "#!/bin/sh\necho boom-went-the-harness >&2\nexit 3\n")
            .expect("bad script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let folder = dir.path().join("shop-api");
        std::fs::create_dir_all(&folder).expect("folder");
        let project = ProjectId("shop-api".to_string());
        let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
        let cancel = CancellationToken::new();
        let (service, _ui) = AssistantService::new(live_deps(store, dir.path(), cancel.clone()));
        let request = OpenRequest {
            harness: test_harness(bad),
            project: project.clone(),
            project_name: "shop-api".to_string(),
            folder: folder.clone(),
            model: None,
        };
        let id = service.open(request.clone()).await.expect("open");
        let _ = service.send(id, "hello".to_string(), None);
        let errored = wait_for("the error state", || {
            let s = service.session_snapshot(&id)?;
            match &s.state {
                SessionState::Error { stderr } => Some(stderr.clone()),
                _ => None,
            }
        })
        .await;
        assert!(!errored.trim().is_empty(), "no stderr tail kept");

        // Retry with a working harness: the same session answers.
        let (script, _) = fake_script(dir.path(), &fixture_replay());
        let mut retry_request = request;
        retry_request.harness = test_harness(script);
        service.retry(id, retry_request).await.expect("retry");
        service
            .send(id, "List the containers".to_string(), None)
            .expect("send after retry");
        let done = wait_for("the answer after retry", || {
            let s = service.session_snapshot(&id)?;
            (s.state == SessionState::Idle && answer_text(&s).contains("Three containers"))
                .then_some(s)
        })
        .await;
        assert_eq!(done.harness_session.as_deref(), Some("fake-1"));
        drop(service);
        cancel.cancel();
    }
}
