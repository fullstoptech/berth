//! The bridge executor (PLAN §6.9, ADR 0006): runs the 25 assistant tools
//! against the engines, the rules and the store; gates them by risk; redacts
//! every result; records `assistant.<tool>` in Activity.
//!
//! Risk decides the gate (D44). Read runs at once. Change waits for an
//! approval that arrives over the [`ApprovalHub`] channel — deliberately
//! AppState-agnostic, so the P7-T5 UI drives it from `state.approvals` and
//! tests drive it directly. Destructive additionally needs the item's exact
//! name typed back. `files.propose_edit` is the one Change tool that does
//! not wait: it parks a pending [`GenerationPlan`] and Review changes is
//! the user's gate (ADR 0012). It never writes.
//!
//! Secrets: `.env` values and container env values are gathered per call
//! only to scrub results through `berth_assistant::redact`; they are never
//! logged, never stored, and dropped when the call ends. Tool arguments are
//! never logged either (`files.propose_edit` carries proposed file text).
//!
//! Everything above the "App wiring" line is Dioxus-free.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use berth_assistant::bridge::{BridgeRequest, BridgeResponse, Incoming};
use berth_core::{
    ActivityEntry, ActivityOrigin, Actor, Container, ContainerAction, EngineId, Outcome, Project,
    ProjectId, Risk, SessionId, Subject, ToolCall,
};
use berth_engine::{ComposeRunner, EngineClient};
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::services::activity;
use crate::services::backing::{Backing, Capability};
use crate::services::engines::ClientMap;
use crate::services::plans::{PendingPlan, PlanRun, PlanStore};

// ---------------------------------------------------------------------------
// Approvals
// ---------------------------------------------------------------------------

/// The user's answer to one [`ApprovalRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Approved {
        /// What the user typed on a Destructive card; `None` for Change.
        typed_name: Option<String>,
    },
    Denied {
        reason: String,
    },
}

/// One gated call waiting for the user. The UI (P7-T5) turns it into an
/// `Approval` in AppState and answers through [`ApprovalRequest::answer`].
pub struct ApprovalRequest {
    pub id: uuid::Uuid,
    pub session: SessionId,
    pub call: ToolCall,
    /// One plain sentence for the card ("Restart container shop-api-api-1").
    /// Built from names only: never an argument value that could be secret.
    pub summary: String,
    /// For Destructive tools: the exact name the user must type back.
    pub typed_name: Option<String>,
    reply: oneshot::Sender<Decision>,
}

impl ApprovalRequest {
    /// Resolves the waiting call. Dropping the request unanswered denies it.
    pub fn answer(self, decision: Decision) {
        let _ = self.reply.send(decision);
    }
}

impl std::fmt::Debug for ApprovalRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately without the call's arguments.
        f.debug_struct("ApprovalRequest")
            .field("id", &self.id)
            .field("tool", &self.call.tool)
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}

/// The executor's end of the approval channel: cheap to clone into the core.
#[derive(Clone)]
pub struct ApprovalHub {
    tx: tokio::sync::mpsc::Sender<ApprovalRequest>,
}

impl ApprovalHub {
    /// The hub and the receiver whoever answers approvals (the UI, a test)
    /// holds.
    #[must_use]
    pub fn channel() -> (Self, tokio::sync::mpsc::Receiver<ApprovalRequest>) {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        (Self { tx }, rx)
    }

    async fn ask(
        &self,
        session: SessionId,
        call: ToolCall,
        summary: String,
        typed_name: Option<String>,
    ) -> Decision {
        let (reply, wait) = oneshot::channel();
        let request = ApprovalRequest {
            id: uuid::Uuid::now_v7(),
            session,
            call,
            summary,
            typed_name,
            reply,
        };
        if self.tx.send(request).await.is_err() {
            return Decision::Denied {
                reason: "no one can approve in Berth right now".to_string(),
            };
        }
        wait.await.unwrap_or_else(|_| Decision::Denied {
            reason: "the approval was dropped".to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// The core
// ---------------------------------------------------------------------------

/// Everything the executor needs, none of it Dioxus.
pub struct ToolDeps {
    pub clients: ClientMap,
    pub store: Arc<berth_store::Store>,
    pub compose: Arc<dyn ComposeRunner>,
    pub plans: PlanStore,
    pub backing: Backing,
    /// The folder files tools use when the call names no project (the
    /// session's project). The app fills this per session in P7-T5; tests
    /// point it at a temp folder.
    pub session_folder: Option<PathBuf>,
}

/// What one executed tool produced, before the final scrub.
struct ToolOutput {
    text: String,
    data: Option<Value>,
    /// Values that must not reach the harness, gathered during the call.
    secrets: Vec<String>,
    engine: Option<EngineId>,
    subject: Subject,
    subject_name: String,
    project: Option<ProjectId>,
}

impl ToolOutput {
    fn plain(text: String) -> Self {
        Self {
            text,
            data: None,
            secrets: Vec::new(),
            engine: None,
            subject: Subject::Engine(EngineId(String::new())),
            subject_name: String::new(),
            project: None,
        }
    }
}

type ToolResult = Result<ToolOutput, String>;

/// One parked proposed change, for whoever shows the session (P7-T5):
/// enough to draw the card, never the plan itself (AD37).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalNotice {
    pub session: SessionId,
    pub plan_id: uuid::Uuid,
    /// The harness's one-line summary, or "Edit <path>".
    pub title: String,
    /// Per file: path, lines added, lines removed.
    pub files: Vec<(PathBuf, u32, u32)>,
}

pub struct ToolCore {
    deps: ToolDeps,
    approvals: ApprovalHub,
    /// Where `files.propose_edit` announces its parked plan.
    proposal_tx: tokio::sync::mpsc::UnboundedSender<ProposalNotice>,
    proposal_rx: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<ProposalNotice>>>,
    /// The folder each live session's files tools act in, registered by
    /// the assistant service when it opens the session.
    session_folders: std::sync::Mutex<std::collections::BTreeMap<SessionId, PathBuf>>,
}

type Client = Arc<dyn EngineClient>;

impl ToolCore {
    #[must_use]
    pub fn new(deps: ToolDeps, approvals: ApprovalHub) -> Self {
        let (proposal_tx, proposal_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            deps,
            approvals,
            proposal_tx,
            proposal_rx: std::sync::Mutex::new(Some(proposal_rx)),
            session_folders: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// The receiver of [`ProposalNotice`]s. Only the first caller gets it.
    pub fn take_proposals(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<ProposalNotice>> {
        self.proposal_rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Where a session's files tools act when a call names no project.
    pub fn register_session_folder(&self, session: SessionId, folder: PathBuf) {
        self.session_folders
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session, folder);
    }

    fn session_folder(&self, session: Option<&SessionId>) -> Option<PathBuf> {
        session
            .and_then(|id| {
                self.session_folders
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(id)
                    .cloned()
            })
            .or_else(|| self.deps.session_folder.clone())
    }

    /// The pending proposed changes (`files.propose_edit` parks plans here).
    #[must_use]
    pub fn plans(&self) -> &PlanStore {
        &self.deps.plans
    }

    /// Runs one bridge request through the risk gate to its response.
    pub async fn handle(&self, req: BridgeRequest) -> BridgeResponse {
        let id = req.id;
        let Some(def) = berth_assistant::tool(&req.tool) else {
            return BridgeResponse::Error {
                id,
                message: format!("unknown tool {}", req.tool),
            };
        };
        let call = ToolCall {
            call_id: req.call_id.clone().unwrap_or_else(|| req.id.to_string()),
            tool: def.name.to_string(),
            args: req.args.clone(),
            risk: def.risk,
        };

        // The gate. `files.propose_edit` parks a plan instead of waiting:
        // Review changes is its approval (ADR 0006, ADR 0012).
        let gated = def.risk != Risk::Read && def.name != "files.propose_edit";
        if gated {
            let verdict = self.gate(def, &req, call).await;
            if let Err(response) = verdict {
                return response;
            }
        }

        match self.execute(def, &req.args, &req.session).await {
            Ok(mut out) => {
                let text = berth_assistant::redact::scrub(&out.text, &out.secrets);
                let data = out
                    .data
                    .take()
                    .map(|value| scrub_value(value, &out.secrets));
                self.record(def, &out, Outcome::Ok);
                BridgeResponse::Ok { id, text, data }
            }
            Err(message) => {
                let mut out = ToolOutput::plain(String::new());
                out.subject_name = target_name(&req.args);
                self.record(
                    def,
                    &out,
                    Outcome::Failed {
                        reason: message.clone(),
                    },
                );
                BridgeResponse::Error { id, message }
            }
        }
    }

    /// The approval wait for a Change or Destructive call. `Err` is the
    /// finished response (a refusal or a denial); `Ok` runs the tool.
    async fn gate(
        &self,
        def: &berth_assistant::ToolDef,
        req: &BridgeRequest,
        call: ToolCall,
    ) -> Result<(), BridgeResponse> {
        let id = req.id;
        let typed_name = if def.risk == Risk::Destructive {
            match self.expected_typed_name(def, &req.args).await {
                Ok(name) => Some(name),
                Err(message) => {
                    return Err(BridgeResponse::Error { id, message });
                }
            }
        } else {
            None
        };
        let summary = approval_summary(def, &req.args);
        let decision = self
            .approvals
            .ask(req.session, call, summary, typed_name.clone())
            .await;
        match decision {
            Decision::Denied { reason } => {
                let mut out = ToolOutput::plain(String::new());
                out.subject_name = target_name(&req.args);
                self.record(
                    def,
                    &out,
                    Outcome::Failed {
                        reason: "denied".to_string(),
                    },
                );
                Err(BridgeResponse::Denied { id, reason })
            }
            Decision::Approved { typed_name: typed } => {
                if let Some(expected) = typed_name {
                    if typed.as_deref() != Some(expected.as_str()) {
                        return Err(BridgeResponse::Denied {
                            id,
                            reason: format!(
                                "the typed name did not match {expected}; nothing was removed"
                            ),
                        });
                    }
                }
                Ok(())
            }
        }
    }

    /// The exact name a Destructive call must be confirmed with, resolved
    /// against what the engine holds. A `name` that matches nothing is
    /// refused here, before any approval is asked.
    async fn expected_typed_name(
        &self,
        def: &berth_assistant::ToolDef,
        args: &Value,
    ) -> Result<String, String> {
        let name = req_str(args, "name")?;
        let engine = get_str(args, "engine");
        match def.name {
            "containers.remove" => {
                let (_, _, container) = self.find_container(engine.as_deref(), &name).await?;
                Ok(container.name)
            }
            "volumes.remove" => {
                let (_, client) = self.pick_engine(engine.as_deref())?;
                let volumes = client.volumes().await.map_err(err)?;
                volumes
                    .into_iter()
                    .find(|v| v.name == name)
                    .map(|v| v.name)
                    .ok_or_else(|| format!("no volume named {name}"))
            }
            "images.remove" => {
                let (_, client) = self.pick_engine(engine.as_deref())?;
                let images = client.images().await.map_err(err)?;
                images
                    .iter()
                    .find(|i| i.id == name || i.repo_tags.iter().any(|t| t == &name))
                    .map(|_| name.clone())
                    .ok_or_else(|| format!("no image named {name}"))
            }
            "system.prune" => {
                let (engine_id, _) = self.pick_engine(Some(&name))?;
                Ok(engine_id.0)
            }
            other => Err(format!("{other} needs no typed name")),
        }
    }

    /// One Activity entry per call: `assistant.<tool>`, never with arguments.
    fn record(&self, def: &berth_assistant::ToolDef, out: &ToolOutput, outcome: Outcome) {
        let engine = out
            .engine
            .clone()
            .or_else(|| self.snapshot().first().map(|(id, _)| id.clone()))
            .unwrap_or_else(|| EngineId(String::new()));
        let entry = ActivityEntry {
            id: uuid::Uuid::now_v7(),
            ts: chrono::Utc::now(),
            engine,
            actor: Actor::Local,
            origin: ActivityOrigin::Berth,
            action: format!("assistant.{}", def.name),
            subject: out.subject.clone(),
            subject_name: out.subject_name.clone(),
            project: out.project.clone(),
            outcome,
        };
        activity::record(&self.deps.store, &entry);
    }

    // -- resolution -------------------------------------------------------

    fn snapshot(&self) -> Vec<(EngineId, Client)> {
        self.deps
            .clients
            .lock()
            .map(|map| {
                map.iter()
                    .map(|(k, v)| (k.clone(), Arc::clone(v)))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn pick_engine(&self, arg: Option<&str>) -> Result<(EngineId, Client), String> {
        let all = self.snapshot();
        if let Some(wanted) = arg {
            return all
                .into_iter()
                .find(|(id, _)| id.0 == wanted)
                .ok_or_else(|| format!("engine {wanted} is not connected"));
        }
        let default = all.iter().find(|(id, _)| id.0 == "colima:default").cloned();
        default
            .or_else(|| all.into_iter().next())
            .ok_or_else(|| "no Docker engine is connected".to_string())
    }

    async fn find_container(
        &self,
        engine_arg: Option<&str>,
        name_or_id: &str,
    ) -> Result<(EngineId, Client, Container), String> {
        let candidates: Vec<(EngineId, Client)> = match engine_arg {
            Some(_) => vec![self.pick_engine(engine_arg)?],
            None => self.snapshot(),
        };
        let bare = name_or_id.trim_start_matches('/');
        for (engine, client) in candidates {
            let containers = client.containers().await.map_err(err)?;
            if let Some(found) = containers
                .into_iter()
                .find(|c| c.name == bare || c.id.0 == name_or_id || c.id.0.starts_with(name_or_id))
            {
                return Ok((engine, client, found));
            }
        }
        Err(format!("no container named {name_or_id}"))
    }

    /// Projects as the engines' containers group, plus pinned folders.
    async fn all_projects(&self) -> Vec<Project> {
        let pins = self.deps.store.pins().unwrap_or_default();
        let mut out = Vec::new();
        for (engine, client) in self.snapshot() {
            if let Ok(containers) = client.containers().await {
                let engine_pins: Vec<_> = pins
                    .iter()
                    .filter(|p| p.engine == engine)
                    .cloned()
                    .collect();
                out.extend(berth_engine::projects::group(
                    &engine,
                    &containers,
                    &engine_pins,
                    false,
                ));
            }
        }
        out
    }

    async fn find_project(&self, name_or_id: &str) -> Result<Project, String> {
        self.all_projects()
            .await
            .into_iter()
            .find(|p| p.name == name_or_id || p.id.0 == name_or_id)
            .ok_or_else(|| format!("no project named {name_or_id}"))
    }

    /// The folder a files tool acts in: the named project, else the
    /// folder registered for `session` (or the deps' session folder).
    /// Returns the folder and the project, when known.
    async fn resolve_folder(
        &self,
        project_arg: Option<&str>,
        session: Option<&SessionId>,
    ) -> Result<(PathBuf, Option<Project>), String> {
        if let Some(name) = project_arg {
            if let Ok(project) = self.find_project(name).await {
                let folder = project
                    .folder
                    .clone()
                    .ok_or_else(|| format!("project {name} has no folder on this Mac"))?;
                return Ok((folder, Some(project)));
            }
            if let Some(folder) = self.session_folder(session) {
                let base = folder.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if base == name {
                    return Ok((folder, None));
                }
            }
            return Err(format!("no project named {name}"));
        }
        self.session_folder(session)
            .map(|folder| (folder, None))
            .ok_or_else(|| "name the project this file belongs to".to_string())
    }

    /// Env values of a project's containers plus its `.env` files: the scrub
    /// list. Memory only, never logged.
    async fn gather_secrets(
        &self,
        project: Option<&Project>,
        folder: Option<&Path>,
    ) -> Vec<String> {
        let mut secrets = Vec::new();
        if let Some(folder) = folder {
            secrets.extend(env_file_values(folder).await);
        }
        if let Some(project) = project {
            if let Ok((_, client)) = self.pick_engine(Some(&project.engine.0)) {
                if let Ok(containers) = client.containers().await {
                    for c in containers
                        .iter()
                        .filter(|c| c.project.as_ref() == Some(&project.id))
                    {
                        if let Ok(detail) = client.container(&c.id).await {
                            secrets.extend(detail.env.into_iter().map(|v| v.value));
                        }
                    }
                }
            }
        }
        secrets.retain(|s| !s.is_empty());
        secrets
    }

    // -- dispatch ----------------------------------------------------------

    async fn execute(
        &self,
        def: &berth_assistant::ToolDef,
        args: &Value,
        session: &SessionId,
    ) -> ToolResult {
        match def.name {
            "engines.list" => self.engines_list().await,
            "projects.list" => self.projects_list().await,
            "containers.list" => self.containers_list(args).await,
            "containers.inspect" => self.containers_inspect(args).await,
            "containers.logs" => self.containers_logs(args).await,
            "containers.stats" => self.containers_stats(args).await,
            "images.list" => self.images_list(args).await,
            "volumes.list" => self.volumes_list(args).await,
            "networks.list" => self.networks_list(args).await,
            "compose.config" => self.compose_config(args, session).await,
            "problems.list" => self.problems_list(args).await,
            "files.read" => self.files_read(args, session).await,
            "activity.list" => self.activity_list(args).await,
            "files.propose_edit" => self.files_propose_edit(args, session).await,
            "compose.up" => self.compose_run(args, true).await,
            "compose.restart" => self.compose_run(args, false).await,
            "containers.restart" => {
                self.container_act(
                    args,
                    ContainerAction::Restart {
                        timeout_s: get_u32(args, "timeout_s"),
                    },
                    "Restarted",
                )
                .await
            }
            "containers.start" => {
                self.container_act(args, ContainerAction::Start, "Started")
                    .await
            }
            "containers.stop" => {
                self.container_act(
                    args,
                    ContainerAction::Stop {
                        timeout_s: get_u32(args, "timeout_s"),
                    },
                    "Stopped",
                )
                .await
            }
            "images.pull" => self.images_pull(args).await,
            "images.build" => Err(
                "building images arrives with the engine's build support in a later phase (P8); \
                 compose.up builds what a Compose service needs"
                    .to_string(),
            ),
            "containers.remove" => {
                let mut remove_args = args.clone();
                if let Value::Object(map) = &mut remove_args {
                    if let Some(name) = map.get("name").cloned() {
                        map.insert("container".to_string(), name);
                    }
                }
                self.container_act(
                    &remove_args,
                    ContainerAction::Remove {
                        force: get_bool(args, "force"),
                        volumes: get_bool(args, "volumes"),
                    },
                    "Removed",
                )
                .await
            }
            "volumes.remove" => self.remove_resource(args, true).await,
            "images.remove" => self.remove_resource(args, false).await,
            "system.prune" => self.system_prune(args).await,
            other => Err(format!("tool {other} is not implemented")),
        }
    }

    // -- Read tools --------------------------------------------------------

    async fn engines_list(&self) -> ToolResult {
        let mut lines = Vec::new();
        let mut data = Vec::new();
        for (id, client) in self.snapshot() {
            let info = client.info().await.map_err(err)?;
            lines.push(format!(
                "{} — Docker {} (API {}), {} CPUs, {} memory, host {}",
                id.0,
                info.docker_version,
                info.api_version,
                info.ncpu,
                berth_core::fmt::memory(info.mem_total),
                client.docker_host()
            ));
            data.push(json!({
                "id": id.0,
                "docker_version": info.docker_version,
                "api_version": info.api_version,
                "os": info.os,
                "arch": info.arch.docker_name(),
            }));
        }
        if lines.is_empty() {
            lines.push("No Docker engine is connected.".to_string());
        }
        let mut out = ToolOutput::plain(lines.join("\n"));
        out.data = Some(Value::Array(data));
        Ok(out)
    }

    async fn projects_list(&self) -> ToolResult {
        let projects = self.all_projects().await;
        let mut lines = Vec::new();
        for p in &projects {
            let services: Vec<String> = p
                .services
                .iter()
                .map(|s| format!("{} ({:?})", s.name, s.state))
                .collect();
            lines.push(format!(
                "{} — {:?}, {} service(s){}{}",
                p.name,
                p.status,
                p.services.len(),
                p.folder
                    .as_ref()
                    .map(|f| format!(", folder {}", f.display()))
                    .unwrap_or_default(),
                if services.is_empty() {
                    String::new()
                } else {
                    format!(": {}", services.join(", "))
                }
            ));
        }
        if lines.is_empty() {
            lines.push("No projects.".to_string());
        }
        let mut out = ToolOutput::plain(lines.join("\n"));
        out.data = serde_json::to_value(&projects).ok();
        Ok(out)
    }

    async fn containers_list(&self, args: &Value) -> ToolResult {
        let engine_arg = get_str(args, "engine");
        let project_arg = get_str(args, "project");
        let all = args.get("all").and_then(Value::as_bool).unwrap_or(true);
        let engines: Vec<(EngineId, Client)> = match &engine_arg {
            Some(_) => vec![self.pick_engine(engine_arg.as_deref())?],
            None => self.snapshot(),
        };
        let mut rows: Vec<Container> = Vec::new();
        for (_, client) in engines {
            rows.extend(client.containers().await.map_err(err)?);
        }
        if let Some(project) = &project_arg {
            rows.retain(|c| c.compose_project.as_deref() == Some(project.as_str()));
        }
        if !all {
            rows.retain(|c| c.state == berth_core::ContainerState::Running);
        }
        let lines: Vec<String> = rows.iter().map(container_line).collect();
        let text = if lines.is_empty() {
            "No containers.".to_string()
        } else {
            lines.join("\n")
        };
        let mut out = ToolOutput::plain(text);
        out.data = serde_json::to_value(&rows).ok();
        Ok(out)
    }

    async fn containers_inspect(&self, args: &Value) -> ToolResult {
        let wanted = req_str(args, "container")?;
        let (engine, client, found) = self
            .find_container(get_str(args, "engine").as_deref(), &wanted)
            .await?;
        let detail = client.container(&found.id).await.map_err(err)?;
        let secrets: Vec<String> = detail.env.iter().map(|v| v.value.clone()).collect();
        let raw = if detail.inspect.is_null() {
            serde_json::to_value(&detail).unwrap_or(Value::Null)
        } else {
            detail.inspect.clone()
        };
        let redacted = berth_assistant::redact::inspect(&raw);
        let text = serde_json::to_string_pretty(&redacted).unwrap_or_default();
        let mut out = ToolOutput::plain(text);
        out.data = Some(redacted);
        out.secrets = secrets;
        out.engine = Some(engine);
        out.subject = Subject::Container(found.id.clone());
        out.subject_name = found.name.clone();
        out.project = found.project.clone();
        Ok(out)
    }

    async fn containers_logs(&self, args: &Value) -> ToolResult {
        let wanted = req_str(args, "container")?;
        let (engine, client, found) = self
            .find_container(get_str(args, "engine").as_deref(), &wanted)
            .await?;
        let tail = get_u32(args, "tail").unwrap_or(200).min(1000);
        let since = match get_str(args, "since") {
            Some(text) => Some(parse_since(&text)?),
            None => None,
        };
        let cancel = CancellationToken::new();
        let query = berth_core::LogQuery {
            since,
            tail: Some(tail),
            follow: false,
        };
        let mut stream = client.logs(&found.id, query, cancel.clone());
        let mut texts = Vec::new();
        let collect = async {
            while let Some(item) = stream.next().await {
                match item {
                    Ok(line) => texts.push(line.text),
                    Err(e) => return Err(err(e)),
                }
            }
            Ok::<(), String>(())
        };
        let done = tokio::time::timeout(std::time::Duration::from_secs(15), collect).await;
        cancel.cancel();
        match done {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err("reading the logs timed out".to_string()),
        }
        if let Some(grep) = get_str(args, "grep") {
            texts.retain(|line| line.contains(&grep));
        }
        let mut secrets = Vec::new();
        if let Ok(detail) = client.container(&found.id).await {
            secrets.extend(detail.env.into_iter().map(|v| v.value));
        }
        if let Some(project_id) = &found.project {
            if let Some(project) = self
                .all_projects()
                .await
                .into_iter()
                .find(|p| &p.id == project_id)
            {
                secrets.extend(
                    self.gather_secrets(Some(&project), project.folder.as_deref())
                        .await,
                );
            }
        }
        let mut text = texts.join("\n");
        if text.len() > 24_000 {
            text.truncate(
                text.char_indices()
                    .take(24_000)
                    .last()
                    .map_or(0, |(i, _)| i),
            );
            text.push_str("\n… (truncated)");
        }
        let mut out = ToolOutput::plain(if text.is_empty() {
            "No log lines.".to_string()
        } else {
            text
        });
        out.secrets = secrets;
        out.engine = Some(engine);
        out.subject = Subject::Container(found.id.clone());
        out.subject_name = found.name.clone();
        out.project = found.project.clone();
        Ok(out)
    }

    async fn containers_stats(&self, args: &Value) -> ToolResult {
        let wanted = req_str(args, "container")?;
        let (engine, client, found) = self
            .find_container(get_str(args, "engine").as_deref(), &wanted)
            .await?;
        if let Some(stats) = &found.stats {
            let mut out = ToolOutput::plain(format!(
                "{}: CPU {:.1}%, memory {} of {}, network {} in / {} out",
                found.name,
                stats.cpu_pct,
                berth_core::fmt::memory(stats.mem_used),
                berth_core::fmt::memory(stats.mem_limit),
                berth_core::fmt::memory(stats.net_rx),
                berth_core::fmt::memory(stats.net_tx)
            ));
            out.data = serde_json::to_value(stats).ok();
            out.engine = Some(engine);
            out.subject = Subject::Container(found.id.clone());
            out.subject_name = found.name.clone();
            return Ok(out);
        }
        let cancel = CancellationToken::new();
        let mut stream = client.stats(&found.id, cancel.clone());
        let first = tokio::time::timeout(std::time::Duration::from_secs(10), stream.next()).await;
        cancel.cancel();
        let stats = match first {
            Ok(Some(Ok(stats))) => stats,
            Ok(Some(Err(e))) => return Err(err(e)),
            _ => return Err(format!("no stats for {}", found.name)),
        };
        let mut out = ToolOutput::plain(format!(
            "{}: CPU {:.1}%, memory {} of {}",
            found.name,
            stats.cpu_pct,
            berth_core::fmt::memory(stats.mem_used),
            berth_core::fmt::memory(stats.mem_limit)
        ));
        out.data = serde_json::to_value(&stats).ok();
        out.engine = Some(engine);
        out.subject = Subject::Container(found.id.clone());
        out.subject_name = found.name.clone();
        Ok(out)
    }

    async fn images_list(&self, args: &Value) -> ToolResult {
        let (engine, client) = self.pick_engine(get_str(args, "engine").as_deref())?;
        let images = client.images().await.map_err(err)?;
        let lines: Vec<String> = images
            .iter()
            .map(|i| {
                format!(
                    "{} — {}, used by {} container(s){}",
                    if i.repo_tags.is_empty() {
                        i.id.clone()
                    } else {
                        i.repo_tags.join(", ")
                    },
                    berth_core::fmt::bytes(i.size),
                    i.used_by.len(),
                    if i.dangling { ", dangling" } else { "" }
                )
            })
            .collect();
        let mut out = ToolOutput::plain(if lines.is_empty() {
            "No images.".to_string()
        } else {
            lines.join("\n")
        });
        out.data = serde_json::to_value(&images).ok();
        out.engine = Some(engine);
        Ok(out)
    }

    async fn volumes_list(&self, args: &Value) -> ToolResult {
        let (engine, client) = self.pick_engine(get_str(args, "engine").as_deref())?;
        let volumes = client.volumes().await.map_err(err)?;
        let sizes = client.volume_sizes().await.unwrap_or_default();
        let lines: Vec<String> = volumes
            .iter()
            .map(|v| {
                let size = v
                    .size
                    .or_else(|| sizes.get(&v.name).copied())
                    .map(berth_core::fmt::bytes)
                    .unwrap_or_else(|| "size unknown".to_string());
                format!(
                    "{} — {}, used by {} container(s)",
                    v.name,
                    size,
                    v.used_by.len()
                )
            })
            .collect();
        let mut out = ToolOutput::plain(if lines.is_empty() {
            "No volumes.".to_string()
        } else {
            lines.join("\n")
        });
        out.data = serde_json::to_value(&volumes).ok();
        out.engine = Some(engine);
        Ok(out)
    }

    async fn networks_list(&self, args: &Value) -> ToolResult {
        let (engine, client) = self.pick_engine(get_str(args, "engine").as_deref())?;
        let networks = client.networks().await.map_err(err)?;
        let lines: Vec<String> = networks
            .iter()
            .map(|n| {
                format!(
                    "{} — {}{}, {} container(s)",
                    n.name,
                    n.driver,
                    n.subnet
                        .as_ref()
                        .map(|s| format!(", {s}"))
                        .unwrap_or_default(),
                    n.containers.len()
                )
            })
            .collect();
        let mut out = ToolOutput::plain(if lines.is_empty() {
            "No networks.".to_string()
        } else {
            lines.join("\n")
        });
        out.data = serde_json::to_value(&networks).ok();
        out.engine = Some(engine);
        Ok(out)
    }

    async fn compose_config(&self, args: &Value, session: &SessionId) -> ToolResult {
        let wanted = req_str(args, "project")?;
        let (target, project) = match self.find_project(&wanted).await {
            Ok(project) => {
                let (_, client) = self.pick_engine(Some(&project.engine.0))?;
                let target = crate::services::compose::target_for(&project, client.docker_host())?;
                (target, Some(project))
            }
            Err(_) => {
                let (folder, _) = self.resolve_folder(Some(&wanted), Some(session)).await?;
                let (engine, client) = self.pick_engine(None)?;
                let files = compose_files_in(&folder);
                if files.is_empty() {
                    return Err(format!("no Compose file in {}", folder.display()));
                }
                let target = crate::services::compose::target_for_folder(
                    &folder,
                    &engine,
                    client.docker_host(),
                    &files,
                    None,
                );
                (target, None)
            }
        };
        // `CliCompose::config` runs with --no-interpolate (AD37); the result
        // is still masked and scrubbed before it can leave.
        let raw = self.deps.compose.config(&target).await.map_err(err)?;
        let redacted = berth_assistant::redact::compose_config(&raw);
        let secrets = self
            .gather_secrets(project.as_ref(), Some(&target.folder))
            .await;
        let mut out =
            ToolOutput::plain(serde_json::to_string_pretty(&redacted).unwrap_or_default());
        out.data = Some(redacted);
        out.secrets = secrets;
        out.engine = Some(target.engine.clone());
        if let Some(project) = &project {
            out.subject = Subject::Project(project.id.clone());
            out.subject_name = project.name.clone();
            out.project = Some(project.id.clone());
        }
        Ok(out)
    }

    async fn problems_list(&self, args: &Value) -> ToolResult {
        use berth_rules::problems::{ContainerFacts, ProblemInput, ProjectFacts};
        let project_filter = get_str(args, "project");
        let mut problems = Vec::new();
        let mut secrets = Vec::new();
        for (engine_id, client) in self.snapshot() {
            let containers = client.containers().await.map_err(err)?;
            let pins: Vec<_> = self
                .deps
                .store
                .pins()
                .unwrap_or_default()
                .into_iter()
                .filter(|p| p.engine == engine_id)
                .collect();
            let projects = berth_engine::projects::group(&engine_id, &containers, &pins, false);
            let mut facts = Vec::new();
            for c in &containers {
                let detail = if worth_reading(c) {
                    client.container(&c.id).await.ok()
                } else {
                    None
                };
                if let Some(detail) = &detail {
                    secrets.extend(detail.env.iter().map(|v| v.value.clone()));
                }
                facts.push(ContainerFacts {
                    container: c.clone(),
                    detail,
                    error_lines: Vec::new(),
                    last_lines: Vec::new(),
                });
            }
            let mut project_facts = Vec::new();
            for p in &projects {
                let (compose_text, env_text) = match &p.folder {
                    Some(folder) => {
                        secrets.extend(env_file_values(folder).await);
                        let compose_text = p
                            .compose_files
                            .first()
                            .and_then(|f| std::fs::read_to_string(folder.join(f)).ok());
                        let env_text = std::fs::read_to_string(folder.join(".env")).ok();
                        (compose_text, env_text)
                    }
                    None => (None, None),
                };
                project_facts.push(ProjectFacts {
                    project: p.clone(),
                    compose_text,
                    env_text,
                    compose_error: None,
                    failure_lines: Vec::new(),
                    missing_binds: Vec::new(),
                });
            }
            let info = client.info().await.ok();
            let engine = berth_core::Engine {
                id: engine_id.clone(),
                name: engine_id.0.clone(),
                kind: engine_kind_of(&engine_id),
                state: berth_core::EngineState::Running,
                docker_host: Some(client.docker_host().to_string()),
                docker_version: info.as_ref().map(|i| i.docker_version.clone()),
                resources: None,
                counts: berth_core::ResourceCounts::default(),
                owner: berth_core::OwnerId::Local,
            };
            let input = ProblemInput {
                now: chrono::Utc::now(),
                engine,
                host_arch: berth_core::Arch::parse(std::env::consts::ARCH),
                containers: facts,
                projects: project_facts,
                disk: client.disk_usage().await.ok(),
                taken_ports: crate::services::ports::taken_ports(&containers),
            };
            let mut found = berth_rules::problems::evaluate(&input);
            if let Some(name) = &project_filter {
                found.retain(|problem| problem_belongs_to(problem, &projects, name));
            }
            problems.extend(found);
        }
        // Only the words leave: a fix can carry a GenerationPlan whose file
        // text holds real values, so fixes render as labels, never payloads.
        let lines: Vec<String> = problems
            .iter()
            .map(|p| {
                let fixes: Vec<&str> = p.fixes.iter().map(|f| f.label.as_str()).collect();
                format!(
                    "[{:?}] {} — {}{}",
                    p.severity,
                    p.title,
                    p.explanation,
                    if fixes.is_empty() {
                        String::new()
                    } else {
                        format!(" Fixes: {}", fixes.join("; "))
                    }
                )
            })
            .collect();
        let data: Vec<Value> = problems
            .iter()
            .map(|p| {
                json!({
                    "id": p.id.0,
                    "kind": format!("{:?}", p.kind),
                    "severity": format!("{:?}", p.severity),
                    "title": p.title,
                    "explanation": p.explanation,
                    "evidence": p.evidence,
                    "fixes": p.fixes.iter().map(|f| f.label.clone()).collect::<Vec<_>>(),
                })
            })
            .collect();
        let mut out = ToolOutput::plain(if lines.is_empty() {
            "No problems found.".to_string()
        } else {
            lines.join("\n")
        });
        out.data = Some(Value::Array(data));
        out.secrets = secrets;
        Ok(out)
    }

    async fn files_read(&self, args: &Value, session: &SessionId) -> ToolResult {
        let rel = req_str(args, "path")?;
        let (folder, project) = self
            .resolve_folder(get_str(args, "project").as_deref(), Some(session))
            .await?;
        let path = safe_join(&folder, &rel)?;
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| format!("cannot read {rel}: {e}"))?;
        let text = String::from_utf8(bytes).map_err(|_| format!("{rel} is not a text file"))?;
        let truncated = text.len() > 64_000;
        let text = if truncated {
            format!(
                "{}\n… (truncated)",
                &text[..text[..64_000].rfind('\n').unwrap_or(64_000)]
            )
        } else {
            text
        };
        let rendered = if berth_assistant::redact::is_env_file(&path) {
            berth_assistant::redact::env_file(&text)
        } else if is_compose_name(&path) {
            berth_assistant::redact::compose_text(&text)
        } else {
            text
        };
        let secrets = self.gather_secrets(project.as_ref(), Some(&folder)).await;
        let mut out = ToolOutput::plain(rendered);
        out.secrets = secrets;
        if let Some(project) = &project {
            out.subject = Subject::Project(project.id.clone());
            out.subject_name = project.name.clone();
            out.project = Some(project.id.clone());
            out.engine = Some(project.engine.clone());
        }
        Ok(out)
    }

    async fn activity_list(&self, args: &Value) -> ToolResult {
        let limit = get_u32(args, "limit").unwrap_or(50).min(200) as usize;
        let mut entries = activity::load_recent(&self.deps.store, limit);
        if let Some(name) = get_str(args, "project") {
            let id = self.find_project(&name).await.ok().map(|p| p.id);
            entries.retain(|e| e.project == id && id.is_some());
        }
        let lines: Vec<String> = entries
            .iter()
            .map(|e| {
                format!(
                    "{} {} {} — {}",
                    e.ts.format("%H:%M:%S"),
                    e.action,
                    e.subject_name,
                    match &e.outcome {
                        Outcome::Ok => "ok".to_string(),
                        Outcome::Failed { reason } => format!("failed: {reason}"),
                    }
                )
            })
            .collect();
        let mut out = ToolOutput::plain(if lines.is_empty() {
            "No activity yet.".to_string()
        } else {
            lines.join("\n")
        });
        out.data = serde_json::to_value(&entries).ok();
        Ok(out)
    }

    // -- Change and Destructive tools --------------------------------------
    //
    // These run only after `gate` approved. The same engine calls the
    // Dioxus services make (ActionsService's `act`, `remove_resource` for
    // EngineAction::Remove, compose::run, prune::run): the services
    // themselves write signals, which this Dioxus-free core cannot do.

    async fn container_act(&self, args: &Value, action: ContainerAction, verb: &str) -> ToolResult {
        let wanted = req_str(args, "container")?;
        let (engine, client, found) = self
            .find_container(get_str(args, "engine").as_deref(), &wanted)
            .await?;
        client.act(&found.id, action).await.map_err(err)?;
        let mut out = ToolOutput::plain(format!("{verb} {}", found.name));
        out.engine = Some(engine);
        out.subject = Subject::Container(found.id.clone());
        out.subject_name = found.name.clone();
        out.project = found.project.clone();
        Ok(out)
    }

    async fn compose_run(&self, args: &Value, up: bool) -> ToolResult {
        let wanted = req_str(args, "project")?;
        let project = self.find_project(&wanted).await?;
        let (_, client) = self.pick_engine(Some(&project.engine.0))?;
        let target = crate::services::compose::target_for(&project, client.docker_host())?;
        let services = get_str_list(args, "services");
        let command = if up {
            berth_core::ComposeCommand::Up {
                services,
                build: false,
            }
        } else {
            berth_core::ComposeCommand::Restart { services }
        };
        crate::services::compose::run(
            self.deps.compose.as_ref(),
            &target,
            command,
            CancellationToken::new(),
        )
        .await
        .map_err(|failure| failure.message)?;
        let mut out = ToolOutput::plain(format!(
            "{} {}",
            if up { "Started" } else { "Restarted" },
            project.name
        ));
        out.engine = Some(project.engine.clone());
        out.subject = Subject::Project(project.id.clone());
        out.subject_name = project.name.clone();
        out.project = Some(project.id.clone());
        out.secrets = self
            .gather_secrets(Some(&project), project.folder.as_deref())
            .await;
        Ok(out)
    }

    async fn images_pull(&self, args: &Value) -> ToolResult {
        let reference = req_str(args, "reference")?;
        let platform = get_str(args, "platform");
        let (engine, client) = self.pick_engine(get_str(args, "engine").as_deref())?;
        let cancel = CancellationToken::new();
        let mut stream = client.pull(&reference, platform.as_deref(), None, cancel.clone());
        let mut events = 0_u32;
        while let Some(item) = stream.next().await {
            item.map_err(err)?;
            events += 1;
        }
        let mut out = ToolOutput::plain(format!("Pulled {reference} ({events} progress events)"));
        out.engine = Some(engine);
        out.subject = Subject::Image(reference.clone());
        out.subject_name = reference;
        Ok(out)
    }

    /// `volume` selects `ResourceRef::Volume`, else `ResourceRef::Image`:
    /// the same removal `EngineAction::Remove` runs (AD25).
    async fn remove_resource(&self, args: &Value, volume: bool) -> ToolResult {
        let name = req_str(args, "name")?;
        let (engine, client) = self.pick_engine(get_str(args, "engine").as_deref())?;
        let target = if volume {
            berth_core::ResourceRef::Volume(name.clone())
        } else {
            berth_core::ResourceRef::Image(name.clone())
        };
        berth_engine::remove_resource(client.as_ref(), &target, get_bool(args, "force"))
            .await
            .map_err(err)?;
        let mut out = ToolOutput::plain(format!("Removed {name}"));
        out.engine = Some(engine);
        out.subject = if volume {
            Subject::Volume(name.clone())
        } else {
            Subject::Image(name.clone())
        };
        out.subject_name = name;
        Ok(out)
    }

    async fn system_prune(&self, args: &Value) -> ToolResult {
        use crate::services::prune::{self, Group};
        let engine_name = req_str(args, "name")?;
        let (engine, client) = self.pick_engine(Some(&engine_name))?;
        let containers = client.containers().await.map_err(err)?;
        let images = client.images().await.map_err(err)?;
        let volumes = client.volumes().await.map_err(err)?;
        let networks = client.networks().await.map_err(err)?;
        let usage = client.disk_usage().await.ok();
        let lists = prune::Lists {
            containers: &containers,
            images: &images,
            volumes: &volumes,
            networks: &networks,
            usage: usage.as_ref(),
        };
        let entries = prune::candidates(&engine, &lists);
        let checked: std::collections::BTreeSet<Group> = match args.get("kinds") {
            Some(Value::Array(kinds)) => {
                let mut set = std::collections::BTreeSet::new();
                for kind in kinds.iter().filter_map(Value::as_str) {
                    match kind {
                        "container" => {
                            set.insert(Group::StoppedContainers);
                        }
                        "image" => {
                            set.insert(Group::DanglingImages);
                            set.insert(Group::UnusedImages);
                        }
                        "volume" => {
                            set.insert(Group::UnusedVolumes);
                        }
                        "network" => {
                            set.insert(Group::UnusedNetworks);
                        }
                        "build_cache" => {
                            set.insert(Group::BuildCache);
                        }
                        other => return Err(format!("unknown prune kind {other}")),
                    }
                }
                set
            }
            _ => prune::default_checked(),
        };
        let plan = prune::plan(&engine, &entries, &checked);
        let planned = plan.items.iter().filter(|i| i.selected).count();
        let outcome = prune::run(client.as_ref(), &plan, CancellationToken::new(), |_| {}).await;
        let mut text = format!(
            "Pruned {} of {} item(s) on {}, freed {}",
            outcome.removed.len(),
            planned,
            engine.0,
            berth_core::fmt::bytes(outcome.freed)
        );
        for (item, reason) in &outcome.failed {
            text.push_str(&format!("\nCould not remove {}: {reason}", item.name));
        }
        let mut out = ToolOutput::plain(text);
        out.data = Some(json!({
            "removed": outcome.removed.iter().map(|i| i.name.clone()).collect::<Vec<_>>(),
            "failed": outcome.failed.iter().map(|(i, r)| json!({"name": i.name, "reason": r})).collect::<Vec<_>>(),
            "freed_bytes": outcome.freed.as_u64(),
        }));
        out.engine = Some(engine.clone());
        out.subject = Subject::Engine(engine);
        out.subject_name = engine_name;
        Ok(out)
    }

    // -- files.propose_edit --------------------------------------------------

    /// Parks a pending plan and writes nothing (ADR 0006, ADR 0012). The
    /// proposed text is never logged; a `KEY=••••` line in a proposed
    /// `.env` keeps the value on disk (`redact::unmask_env`).
    async fn files_propose_edit(&self, args: &Value, session: &SessionId) -> ToolResult {
        if let Some(reason) = self.deps.backing.unsupported(Capability::WriteFiles) {
            return Err(reason);
        }
        let rel = req_str(args, "path")?;
        let content = req_str(args, "content")?;
        let (folder, project) = self
            .resolve_folder(get_str(args, "project").as_deref(), Some(session))
            .await?;
        // The file may not exist yet, so containment is checked on the
        // relative shape and the canonical parent, not the file itself.
        let rel_path = safe_rel(&rel)?;
        let target = folder.join(&rel_path);
        if let Some(parent) = target.parent() {
            let canon_folder = std::fs::canonicalize(&folder)
                .map_err(|e| format!("the project folder is unreadable: {e}"))?;
            let canon_parent =
                std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
            if !canon_parent.starts_with(&canon_folder) {
                return Err(format!("{rel} is outside the project folder"));
            }
        }
        let old = tokio::fs::read_to_string(&target).await.ok();
        let new_text = if berth_assistant::redact::is_env_file(&rel_path) {
            berth_assistant::redact::unmask_env(&content, old.as_deref().unwrap_or(""))
        } else {
            content
        };
        let diff = berth_rules::diff::file_diff(&rel_path, old.as_deref(), Some(&new_text));
        let notice_file = (diff.path.clone(), diff.added, diff.removed);
        let summary = get_str(args, "summary");
        let plan = berth_core::GenerationPlan {
            root: folder.clone(),
            files: vec![diff],
            source: berth_core::PlanSource::Harness(berth_core::HarnessId("assistant".to_string())),
            notes: summary.clone().into_iter().collect(),
            after_write: None,
        };
        let title = project.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| {
            folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        let mut pending = PendingPlan::new(plan, title);
        pending.run = match &project {
            Some(project) => PlanRun::for_project(project),
            None => self.pick_engine(None).ok().map(|(engine, _)| {
                PlanRun::for_folder(&engine, &folder, compose_files_in(&folder))
            }),
        };
        let plan_id = self.deps.plans.insert(pending);
        // Tell whoever shows the session (P7-T5) so the turn gains its
        // proposed-change card; the notice carries no plan body (AD37).
        let _ = self.proposal_tx.send(ProposalNotice {
            session: *session,
            plan_id,
            title: summary.clone().unwrap_or_else(|| format!("Edit {rel}")),
            files: vec![notice_file],
        });
        let mut out = ToolOutput::plain(format!(
            "Proposed change {plan_id} for {rel} is waiting for the user's review in Berth \
             (Review changes). Nothing was written."
        ));
        out.data = Some(json!({
            "plan_id": plan_id.to_string(),
            "path": rel,
            "summary": summary,
        }));
        if let Some(project) = &project {
            out.subject = Subject::Project(project.id.clone());
            out.subject_name = project.name.clone();
            out.project = Some(project.id.clone());
            out.engine = Some(project.engine.clone());
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn err(e: berth_engine::EngineError) -> String {
    e.to_string()
}

fn get_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn req_str(args: &Value, key: &str) -> Result<String, String> {
    get_str(args, key).ok_or_else(|| format!("missing argument {key}"))
}

fn get_bool(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn get_u32(args: &Value, key: &str) -> Option<u32> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

fn get_str_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The name an activity line may carry: a container, project, image or file
/// name from the arguments. Never a value, never file content.
fn target_name(args: &Value) -> String {
    for key in ["container", "name", "project", "reference", "path"] {
        if let Some(value) = get_str(args, key) {
            return value;
        }
    }
    String::new()
}

/// One sentence for the approval card, from names only.
fn approval_summary(def: &berth_assistant::ToolDef, args: &Value) -> String {
    let target = target_name(args);
    let verb = match def.name {
        "containers.restart" => "Restart container",
        "containers.start" => "Start container",
        "containers.stop" => "Stop container",
        "containers.remove" => "Remove container",
        "compose.up" => "Start project",
        "compose.restart" => "Restart project",
        "images.pull" => "Pull image",
        "images.build" => "Build image",
        "images.remove" => "Remove image",
        "volumes.remove" => "Remove volume",
        "system.prune" => "Prune engine",
        _ => def.name,
    };
    if target.is_empty() {
        verb.to_string()
    } else {
        format!("{verb} {target}")
    }
}

fn container_line(c: &Container) -> String {
    let ports: Vec<String> = c.ports.iter().map(berth_core::fmt::port).collect();
    format!(
        "{} — {:?}, image {}, {}{}{}",
        c.name,
        c.state,
        c.image.reference,
        c.compose_project
            .as_ref()
            .map(|p| format!("project {p}, "))
            .unwrap_or_default(),
        if ports.is_empty() {
            "no ports".to_string()
        } else {
            format!("ports {}", ports.join(", "))
        },
        c.health
            .map(|h| format!(", health {h:?}"))
            .unwrap_or_default()
    )
}

/// Mirrors the Problems service: a running, healthy, never-restarted
/// container cannot have a problem in its inspect.
fn worth_reading(c: &Container) -> bool {
    !(c.state == berth_core::ContainerState::Running
        && matches!(
            c.health,
            None | Some(berth_core::Health::Healthy | berth_core::Health::Starting)
        )
        && c.restart_count == 0)
}

fn engine_kind_of(id: &EngineId) -> berth_core::EngineKind {
    match id.0.strip_prefix("colima:") {
        Some(profile) => berth_core::EngineKind::Colima {
            profile: profile.to_string(),
        },
        None => berth_core::EngineKind::Socket {
            path: PathBuf::new(),
            context: None,
        },
    }
}

fn problem_belongs_to(problem: &berth_core::Problem, projects: &[Project], name: &str) -> bool {
    match &problem.subject {
        Subject::Project(id) => projects
            .iter()
            .any(|p| &p.id == id && (p.name == name || p.id.0 == name)),
        Subject::Container(cid) => projects
            .iter()
            .filter(|p| p.name == name || p.id.0 == name)
            .any(|p| p.services.iter().any(|s| s.containers.contains(cid))),
        _ => false,
    }
}

/// `10m`, `2h`, `30s`, `1d`, or an RFC 3339 timestamp.
fn parse_since(text: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (digits, unit) = text.split_at(split);
    if !digits.is_empty() {
        if let Ok(amount) = digits.parse::<i64>() {
            let span = match unit {
                "s" => Some(chrono::TimeDelta::seconds(amount)),
                "m" => Some(chrono::TimeDelta::minutes(amount)),
                "h" => Some(chrono::TimeDelta::hours(amount)),
                "d" => Some(chrono::TimeDelta::days(amount)),
                _ => None,
            };
            if let Some(span) = span {
                return Ok(chrono::Utc::now() - span);
            }
        }
    }
    chrono::DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&chrono::Utc))
        .map_err(|_| {
            format!("since must be a duration such as 10m or an RFC 3339 time, not {text}")
        })
}

/// A relative path that cannot leave its folder: no absolute, no `..`.
fn safe_rel(rel: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(rel);
    if path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(format!("{rel} is outside the project folder"));
    }
    Ok(path)
}

/// `folder/rel` for a file that exists, symlink escapes included in the ban.
fn safe_join(folder: &Path, rel: &str) -> Result<PathBuf, String> {
    let joined = folder.join(safe_rel(rel)?);
    let canon_folder = std::fs::canonicalize(folder)
        .map_err(|e| format!("the project folder is unreadable: {e}"))?;
    let canon = std::fs::canonicalize(&joined).map_err(|e| format!("cannot read {rel}: {e}"))?;
    if !canon.starts_with(&canon_folder) {
        return Err(format!("{rel} is outside the project folder"));
    }
    Ok(canon)
}

fn is_compose_name(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("compose.yaml" | "compose.yml" | "docker-compose.yaml" | "docker-compose.yml")
    )
}

fn compose_files_in(folder: &Path) -> Vec<PathBuf> {
    [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yml",
        "docker-compose.yaml",
    ]
    .iter()
    .map(PathBuf::from)
    .filter(|f| folder.join(f).is_file())
    .collect()
}

/// Values of the `.env` files at a folder's root. Memory only.
async fn env_file_values(folder: &Path) -> Vec<String> {
    let mut values = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(folder).await else {
        return values;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !berth_assistant::redact::is_env_file(&path) {
            continue;
        }
        if let Ok(text) = tokio::fs::read_to_string(&path).await {
            values.extend(parse_env_values(&text));
        }
    }
    values
}

/// `KEY=value` values of a `.env` text, quotes dropped. Best effort for a
/// scrub list; exact masking of the file itself is `redact::env_file`'s job.
fn parse_env_values(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (_, value) = line.split_once('=')?;
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            (!value.is_empty()).then(|| value.to_string())
        })
        .collect()
}

fn scrub_value(value: Value, secrets: &[String]) -> Value {
    if secrets.iter().all(|s| s.is_empty()) {
        return value;
    }
    match serde_json::to_string(&value) {
        Ok(text) => serde_json::from_str(&berth_assistant::redact::scrub(&text, secrets))
            .unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
}

// ---------------------------------------------------------------------------
// Serving and app wiring
// ---------------------------------------------------------------------------

/// Answers bridge requests with `core` until the channel closes or `cancel`
/// fires. Each request runs in its own task: an approval can wait minutes
/// and must not hold up reads on other connections.
pub async fn serve(
    core: Arc<ToolCore>,
    mut incoming: tokio::sync::mpsc::Receiver<Incoming>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            next = incoming.recv() => {
                let Some(item) = next else { return };
                let core = Arc::clone(&core);
                crate::services::actions::spawn_io(async move {
                    let response = core.handle(item.request).await;
                    let _ = item.reply.send(response);
                });
            }
        }
    }
}

/// What the app keeps from starting the tools service: where `berth mcp`
/// connects, the launch token (memory only), and the core. P7-T3 turns it
/// into a [`berth_assistant::McpLaunch`] when it writes an `--mcp-config`.
#[derive(Clone)]
pub struct ToolsHandle {
    pub socket: PathBuf,
    pub token: String,
    pub core: Arc<ToolCore>,
    /// The approval requests, until the assistant service takes them.
    approvals: Arc<std::sync::Mutex<Option<tokio::sync::mpsc::Receiver<ApprovalRequest>>>>,
}

impl ToolsHandle {
    /// The launch description a harness session needs (PLAN §6.9).
    #[must_use]
    pub fn mcp_launch(&self) -> berth_assistant::McpLaunch {
        berth_assistant::McpLaunch {
            berth_binary: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("berth")),
            bridge: self.socket.clone(),
            token: self.token.clone(),
        }
    }

    /// The receiver of gated calls waiting for the user. Only the first
    /// caller (the assistant service, P7-T5) gets it; until then requests
    /// wait in the channel rather than being answered for the user.
    pub fn take_approvals(&self) -> Option<tokio::sync::mpsc::Receiver<ApprovalRequest>> {
        self.approvals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

/// Binds the one `bridge.sock` in the runtime dir with a fresh token per
/// launch and serves it until `cancel`. Gated calls wait on the approval
/// channel; the assistant service (P7-T5) takes it through
/// [`ToolsHandle::take_approvals`] and drives `state.approvals` from it —
/// nothing is approved or denied on the user's behalf here.
#[must_use]
pub fn start_in_app(deps: ToolDeps, cancel: CancellationToken) -> ToolsHandle {
    let socket = berth_sys::paths::runtime_dir().join(berth_assistant::bridge::SOCKET_NAME);
    let token = uuid::Uuid::now_v7().to_string();
    let (hub, approvals) = ApprovalHub::channel();
    let core = Arc::new(ToolCore::new(deps, hub));
    let handle = ToolsHandle {
        socket: socket.clone(),
        token: token.clone(),
        core: Arc::clone(&core),
        approvals: Arc::new(std::sync::Mutex::new(Some(approvals))),
    };
    crate::services::actions::spawn_io(async move {
        match berth_assistant::bridge::BridgeServer::bind_at(socket, token, cancel.clone()).await {
            Ok((_server, incoming)) => serve(core, incoming, cancel).await,
            Err(err) => tracing::warn!(%err, "the MCP bridge did not start"),
        }
    });
    handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::Receiver;

    const SECRET: &str = "s3cr3t-tool-test-value";

    struct Harness {
        core: Arc<ToolCore>,
        approvals: Receiver<ApprovalRequest>,
        store: Arc<berth_store::Store>,
        _dir: tempfile::TempDir,
    }

    impl Harness {
        async fn next_approval(&mut self) -> ApprovalRequest {
            tokio::time::timeout(std::time::Duration::from_secs(5), self.approvals.recv())
                .await
                .expect("an approval is asked")
                .expect("approval")
        }
    }

    fn harness(backing: Backing) -> Harness {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(
            dir.path().join(".env"),
            format!("# comment\nSECRET={SECRET}\nPLAIN=1\n"),
        )
        .expect("env");
        std::fs::write(
            dir.path().join("notes.txt"),
            format!("the pw is {SECRET}\n"),
        )
        .expect("notes");
        std::fs::write(
            dir.path().join("compose.yaml"),
            "services:\n  web:\n    image: nginx\n    environment:\n      TOKEN: abc123\n",
        )
        .expect("compose");
        let world = berth_engine::MockWorld::bundled();
        let engine = EngineId("colima:default".to_string());
        let mock = berth_engine::MockEngine::from_world(world.clone(), &engine).expect("mock");
        let clients = crate::services::engines::client_map();
        clients
            .lock()
            .expect("lock")
            .insert(engine, Arc::new(mock) as Client);
        let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
        let (hub, approvals) = ApprovalHub::channel();
        let core = Arc::new(ToolCore::new(
            ToolDeps {
                clients,
                store: Arc::clone(&store),
                compose: Arc::new(berth_engine::MockCompose::from_world(&world)),
                plans: PlanStore::default(),
                backing,
                session_folder: Some(dir.path().to_path_buf()),
            },
            hub,
        ));
        Harness {
            core,
            approvals,
            store,
            _dir: dir,
        }
    }

    fn request(tool: &str, args: Value) -> BridgeRequest {
        BridgeRequest {
            id: uuid::Uuid::now_v7(),
            token: "t".to_string(),
            session: SessionId(uuid::Uuid::nil()),
            tool: tool.to_string(),
            args,
            call_id: None,
        }
    }

    fn ok_text(response: BridgeResponse) -> String {
        match response {
            BridgeResponse::Ok { text, .. } => text,
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tools_read_runs_at_once_and_records_activity_without_args() {
        let mut h = harness(Backing::Live);
        let text = ok_text(h.core.handle(request("containers.list", json!({}))).await);
        assert!(text.contains("shop-api-api-1"), "{text}");
        assert!(
            h.approvals.try_recv().is_err(),
            "a Read tool asked for approval"
        );
        let entries = activity::load_recent(&h.store, 10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "assistant.containers.list");
        assert_eq!(entries[0].origin, ActivityOrigin::Berth);
    }

    #[tokio::test]
    async fn tools_change_blocks_until_approved_and_a_deny_denies() {
        let mut h = harness(Backing::Live);
        let core = Arc::clone(&h.core);
        let call = {
            let core = Arc::clone(&core);
            tokio::spawn(async move {
                core.handle(request(
                    "containers.restart",
                    json!({"container": "shop-api-api-1"}),
                ))
                .await
            })
        };
        let approval = h.next_approval().await;
        assert_eq!(approval.call.tool, "containers.restart");
        assert!(
            approval.summary.contains("shop-api-api-1"),
            "{}",
            approval.summary
        );
        assert_eq!(approval.typed_name, None);
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        assert!(!call.is_finished(), "the restart ran without approval");
        approval.answer(Decision::Approved { typed_name: None });
        let text = ok_text(call.await.expect("call task"));
        assert!(text.contains("Restarted shop-api-api-1"), "{text}");

        let denied = {
            let core = Arc::clone(&core);
            tokio::spawn(async move {
                core.handle(request(
                    "containers.restart",
                    json!({"container": "shop-api-api-1"}),
                ))
                .await
            })
        };
        let approval = h.next_approval().await;
        approval.answer(Decision::Denied {
            reason: "not now".to_string(),
        });
        match denied.await.expect("denied task") {
            BridgeResponse::Denied { reason, .. } => assert_eq!(reason, "not now"),
            other => panic!("expected Denied, got {other:?}"),
        }
        let actions: Vec<String> = activity::load_recent(&h.store, 10)
            .into_iter()
            .map(|e| e.action)
            .collect();
        assert_eq!(
            actions
                .iter()
                .filter(|a| *a == "assistant.containers.restart")
                .count(),
            2,
            "{actions:?}"
        );
    }

    #[tokio::test]
    async fn tools_destructive_refuses_a_wrong_name_and_a_wrong_typed_name() {
        let mut h = harness(Backing::Live);
        // A name nothing carries: refused before any approval is asked.
        let refused = h
            .core
            .handle(request(
                "containers.remove",
                json!({"name": "no-such-container"}),
            ))
            .await;
        match refused {
            BridgeResponse::Error { message, .. } => {
                assert!(message.contains("no container named"), "{message}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(h.approvals.try_recv().is_err(), "a refused call asked");

        let core = Arc::clone(&h.core);
        let call = tokio::spawn(async move {
            core.handle(request(
                "containers.remove",
                json!({"name": "shop-api-db-1"}),
            ))
            .await
        });
        let approval = h.next_approval().await;
        assert_eq!(approval.typed_name.as_deref(), Some("shop-api-db-1"));
        approval.answer(Decision::Approved {
            typed_name: Some("shop-api-db-2".to_string()),
        });
        match call.await.expect("call") {
            BridgeResponse::Denied { reason, .. } => {
                assert!(reason.contains("did not match"), "{reason}");
            }
            other => panic!("expected a typed-name denial, got {other:?}"),
        }

        let core = Arc::clone(&h.core);
        let call = tokio::spawn(async move {
            core.handle(request(
                "containers.remove",
                json!({"name": "shop-api-db-1"}),
            ))
            .await
        });
        let approval = h.next_approval().await;
        approval.answer(Decision::Approved {
            typed_name: Some("shop-api-db-1".to_string()),
        });
        let text = ok_text(call.await.expect("call"));
        assert!(text.contains("Removed shop-api-db-1"), "{text}");
    }

    #[tokio::test]
    async fn tools_files_read_masks_env_scrubs_leaks_and_refuses_escapes() {
        let h = harness(Backing::Live);
        let env = ok_text(
            h.core
                .handle(request("files.read", json!({"path": ".env"})))
                .await,
        );
        assert!(env.contains("SECRET=••••"), "{env}");
        assert!(!env.contains(SECRET), "the env value leaked: {env}");

        let notes = ok_text(
            h.core
                .handle(request("files.read", json!({"path": "notes.txt"})))
                .await,
        );
        assert!(
            !notes.contains(SECRET),
            "the secret echoed through: {notes}"
        );
        assert!(notes.contains("••••"), "{notes}");

        let compose = ok_text(
            h.core
                .handle(request("files.read", json!({"path": "compose.yaml"})))
                .await,
        );
        assert!(!compose.contains("abc123"), "compose env leaked: {compose}");

        let escape = h
            .core
            .handle(request("files.read", json!({"path": "../outside.txt"})))
            .await;
        assert!(matches!(escape, BridgeResponse::Error { .. }), "{escape:?}");

        let inspect = ok_text(
            h.core
                .handle(request(
                    "containers.inspect",
                    json!({"container": "shop-api-worker-1"}),
                ))
                .await,
        );
        assert!(
            !inspect.contains("postgres://shop@db:5432/shop"),
            "inspect leaked an env value"
        );
    }

    #[tokio::test]
    async fn tools_propose_edit_parks_a_plan_writes_nothing_and_keeps_masked_values() {
        let h = harness(Backing::Live);
        let folder = h._dir.path().to_path_buf();
        let before = std::fs::read_to_string(folder.join(".env")).expect("env before");
        let response = h
            .core
            .handle(request(
                "files.propose_edit",
                json!({
                    "path": ".env",
                    "content": "SECRET=••••\nPLAIN=2\n",
                    "summary": "bump PLAIN",
                }),
            ))
            .await;
        let (text, data) = match response {
            BridgeResponse::Ok { text, data, .. } => (text, data),
            other => panic!("expected Ok, got {other:?}"),
        };
        assert!(text.contains("Nothing was written"), "{text}");
        assert_eq!(
            std::fs::read_to_string(folder.join(".env")).expect("env after"),
            before,
            "propose_edit wrote the file"
        );
        assert_eq!(h.core.plans().len(), 1);
        let plan_id: uuid::Uuid = data
            .and_then(|d| d.get("plan_id").and_then(Value::as_str).map(str::to_string))
            .and_then(|s| s.parse().ok())
            .expect("plan id");
        let pending = h.core.plans().get(&plan_id).expect("pending plan");
        assert!(matches!(
            pending.plan.source,
            berth_core::PlanSource::Harness(_)
        ));
        let new_text = pending.plan.files[0].new_text.clone().expect("new text");
        assert!(
            new_text.contains(&format!("SECRET={SECRET}")),
            "the masked line lost its on-disk value"
        );
        assert!(new_text.contains("PLAIN=2"), "{new_text}");
    }

    #[tokio::test]
    async fn tools_propose_edit_under_the_mock_is_the_backing_reason() {
        let h = harness(Backing::Mock);
        let response = h
            .core
            .handle(request(
                "files.propose_edit",
                json!({"path": "compose.yaml", "content": "services: {}\n"}),
            ))
            .await;
        match response {
            BridgeResponse::Error { message, .. } => {
                assert_eq!(
                    message,
                    Backing::Mock
                        .unsupported(Capability::WriteFiles)
                        .expect("a reason")
                );
            }
            other => panic!("expected the mock reason, got {other:?}"),
        }
        assert_eq!(h.core.plans().len(), 0);
    }

    #[test]
    fn tools_parse_since_and_safe_paths() {
        assert!(parse_since("10m").is_ok());
        assert!(parse_since("2026-10-26T10:00:00Z").is_ok());
        assert!(parse_since("yesterday-ish").is_err());
        assert!(safe_rel("a/b.txt").is_ok());
        assert!(safe_rel("../b.txt").is_err());
        assert!(safe_rel("/etc/hosts").is_err());
    }
}
