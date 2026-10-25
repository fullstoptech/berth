//! Actions service (D34): optimistic row actions. Pressing a button sets a
//! pending state for (subject, action) at once ("Stopping…"), the engine
//! call runs in a task, success clears the state (the row updates from the
//! engine's event), failure keeps the row as it was and sets
//! `PendingState::Failed` with the engine's text. Every action writes an
//! `ActivityEntry` (D65).
//!
//! Threading follows the engines service: I/O runs in tokio tasks that
//! never touch a signal; each task sends a [`Finished`] over a channel and
//! a Dioxus-spawned receiver applies it with [`apply_finished`]. Tests
//! drive the same channel by hand.
//!
//! Container actions go to the `EngineClient`, project actions to the
//! `ComposeRunner` ([`super::compose`]), engine actions to `ColimaControl`
//! ([`super::colima`]).

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_core::{ContainerAction, Subject};

use crate::services::engines::ClientMap;
use crate::services::{activity, colima, compose, edit, networks};
use crate::state::{AppState, PendingKey, PendingState, Selection};

pub use crate::services::compose::ProjectOp;
pub use crate::services::edit::EditOp;

// ---------------------------------------------------------------------
// Ids and labels
// ---------------------------------------------------------------------

/// The `ActionDef` id of a container action; also its activity action and
/// pending key.
#[must_use]
pub fn container_action_id(action: &ContainerAction) -> &'static str {
    match action {
        ContainerAction::Start => "container.start",
        ContainerAction::Stop { .. } => "container.stop",
        ContainerAction::Restart { .. } => "container.restart",
        ContainerAction::Pause => "container.pause",
        ContainerAction::Unpause => "container.resume",
        ContainerAction::Kill { .. } => "container.kill",
        ContainerAction::Remove { .. } => "container.delete",
        ContainerAction::Rename { .. } => "container.rename",
    }
}

/// The label shown while an action runs (D34).
#[must_use]
pub fn pending_label(action_id: &str) -> &'static str {
    match action_id {
        "container.start" | "project.start" | "engine.start" => "Starting…",
        "container.stop" | "project.stop" | "engine.stop" => "Stopping…",
        "container.restart" | "project.restart" | "engine.restart" => "Restarting…",
        "container.pause" => "Pausing…",
        "container.resume" => "Resuming…",
        "container.kill" => "Killing…",
        "container.delete" | "project.delete" => "Deleting…",
        "container.rename" => "Renaming…",
        "network.create" => "Creating…",
        "network.delete" | "image.delete" | "volume.delete" => "Deleting…",
        "network.connect_container" => "Connecting…",
        "network.disconnect" => "Disconnecting…",
        "container.apply_limits" => "Applying…",
        "container.recreate" => "Recreating…",
        "container.duplicate" => "Duplicating…",
        "project.rebuild" => "Rebuilding…",
        "project.pull_images" => "Pulling…",
        "engine.edit_resources" => "Applying…",
        _ => "Working…",
    }
}

// ---------------------------------------------------------------------
// Disabled reasons (pure)
// ---------------------------------------------------------------------

/// Read-only borrow of the state `disabled_reason` needs. Built from the
/// signals with [`with_view`], or by hand in tests.
pub struct StateView<'a> {
    pub engines: &'a [berth_core::Engine],
    pub containers: &'a [berth_core::Container],
    pub projects: &'a [berth_core::Project],
    pub images: &'a [berth_core::Image],
    pub volumes: &'a [berth_core::Volume],
    pub networks: &'a [berth_core::Network],
    pub pending: &'a std::collections::BTreeMap<PendingKey, PendingState>,
}

/// Run `f` over a [`StateView`] peeked from the signals (UI thread).
pub fn with_view<R>(state: &AppState, f: impl FnOnce(&StateView<'_>) -> R) -> R {
    let engines = state.engines.peek();
    let containers = state.containers.peek();
    let projects = state.projects.peek();
    let images = state.images.peek();
    let volumes = state.volumes.peek();
    let networks = state.networks.peek();
    let pending = state.pending.peek();
    f(&StateView {
        engines: &engines,
        containers: &containers,
        projects: &projects,
        images: &images,
        volumes: &volumes,
        networks: &networks,
        pending: &pending,
    })
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Why `action` (an `ActionDef` id) cannot run on `subject` right now, or
/// `None` when it can. Disabled buttons stay visible and show this text as
/// their tooltip (D34, brief §7.2). Pure: reads only `state`.
#[must_use]
pub fn disabled_reason(subject: &Subject, action: &str, state: &StateView<'_>) -> Option<String> {
    if let Some(label) = state.pending.iter().find_map(|(key, value)| match value {
        PendingState::Running { label } if key.subject == *subject => Some(label.as_str()),
        _ => None,
    }) {
        return Some(format!("{label} is still in progress"));
    }
    match subject {
        Subject::Container(id) => container_reason(id, action, state),
        Subject::Project(id) => project_reason(id, action, state),
        Subject::Engine(id) => engine_reason(id, action, state),
        Subject::Image(id) => image_reason(id, action, state),
        Subject::Volume(name) => volume_reason(name, action, state),
        Subject::Network(id) => network_reason(id, action, state),
        Subject::Plugin(_) => None,
    }
}

fn engine_unavailable(engine: &berth_core::EngineId, state: &StateView<'_>) -> Option<String> {
    let found = state.engines.iter().find(|e| e.id == *engine)?;
    match &found.state {
        berth_core::EngineState::Running => None,
        berth_core::EngineState::Stopped => Some(format!("Engine {} is stopped", found.name)),
        berth_core::EngineState::Starting => Some(format!("Engine {} is starting", found.name)),
        berth_core::EngineState::Stopping => Some(format!("Engine {} is stopping", found.name)),
        berth_core::EngineState::Unreachable { .. } => {
            Some(format!("Engine {} is unreachable", found.name))
        }
        berth_core::EngineState::Unsupported { reason } => Some(reason.clone()),
    }
}

fn container_reason(
    id: &berth_core::ContainerId,
    action: &str,
    state: &StateView<'_>,
) -> Option<String> {
    use berth_core::ContainerState as S;
    let Some(container) = state.containers.iter().find(|c| c.id == *id) else {
        return Some("This container no longer exists".to_string());
    };
    if let Some(reason) = engine_unavailable(&container.engine, state) {
        return Some(reason);
    }
    if container.state == S::Removing {
        return Some("This container is being removed".to_string());
    }
    let running = matches!(container.state, S::Running | S::Restarting);
    match action {
        "container.start" if running => Some("Already running".to_string()),
        "container.start" if container.state == S::Paused => {
            Some("Paused. Resume it instead".to_string())
        }
        "container.stop" if !running && container.state != S::Paused => {
            Some("Not running".to_string())
        }
        "container.pause" if container.state != S::Running => {
            Some("Only a running container can be paused".to_string())
        }
        "container.resume" if container.state != S::Paused => Some("Not paused".to_string()),
        "container.kill" if !running && container.state != S::Paused => {
            Some("Not running".to_string())
        }
        _ => None,
    }
}

fn project_reason(
    id: &berth_core::ProjectId,
    action: &str,
    state: &StateView<'_>,
) -> Option<String> {
    use berth_core::ProjectStatus as P;
    let Some(project) = state.projects.iter().find(|p| p.id == *id) else {
        return Some("This project no longer exists".to_string());
    };
    if !action.starts_with("project.") {
        return None;
    }
    if let Some(reason) = engine_unavailable(&project.engine, state) {
        return Some(reason);
    }
    let composable = matches!(
        action,
        "project.start"
            | "project.stop"
            | "project.restart"
            | "project.rebuild"
            | "project.pull_images"
            | "project.delete"
    );
    if composable {
        if project.folder.is_none() {
            return Some("This project has no folder on this Mac".to_string());
        }
        if project.compose_files.is_empty() {
            return Some("No Compose file is known for this project".to_string());
        }
    }
    match (action, project.status) {
        ("project.start", P::AllRunning) => Some("All services are already running".to_string()),
        ("project.stop" | "project.restart", P::Stopped | P::NotCreated) => {
            Some("Nothing is running".to_string())
        }
        ("project.delete", P::NotCreated) => {
            Some("No containers exist for this project".to_string())
        }
        _ => None,
    }
}

fn engine_reason(id: &berth_core::EngineId, action: &str, state: &StateView<'_>) -> Option<String> {
    use berth_core::EngineState as E;
    let controls = matches!(
        action,
        "engine.start" | "engine.stop" | "engine.restart" | "engine.edit_resources"
    );
    if !controls {
        return None;
    }
    let Some(engine) = state.engines.iter().find(|e| e.id == *id) else {
        return Some("This engine no longer exists".to_string());
    };
    if colima::profile_of(engine).is_none() {
        return Some("Only Colima engines can be started or stopped here".to_string());
    }
    match (action, &engine.state) {
        (_, E::Starting) => Some("The engine is starting".to_string()),
        (_, E::Stopping) => Some("The engine is stopping".to_string()),
        ("engine.start", E::Running) => Some("Already running".to_string()),
        ("engine.stop" | "engine.restart", E::Stopped) => Some("Already stopped".to_string()),
        _ => None,
    }
}

fn image_reason(id: &str, action: &str, state: &StateView<'_>) -> Option<String> {
    if action != "image.delete" {
        return None;
    }
    let Some(image) = state.images.iter().find(|i| i.id == id) else {
        return Some("This image no longer exists".to_string());
    };
    if image.used_by.is_empty() {
        None
    } else {
        Some(format!(
            "Used by {}",
            plural(image.used_by.len(), "container", "containers")
        ))
    }
}

fn volume_reason(name: &str, action: &str, state: &StateView<'_>) -> Option<String> {
    if action != "volume.delete" {
        return None;
    }
    let Some(volume) = state.volumes.iter().find(|v| v.name == name) else {
        return Some("This volume no longer exists".to_string());
    };
    if volume.used_by.is_empty() {
        None
    } else {
        Some(format!(
            "Used by {}",
            plural(volume.used_by.len(), "container", "containers")
        ))
    }
}

fn network_reason(id: &str, action: &str, state: &StateView<'_>) -> Option<String> {
    if action == "network.create" {
        return state
            .networks
            .iter()
            .any(|n| n.name == id)
            .then(|| format!("A network named {id} already exists"));
    }
    if action != "network.delete" {
        return None;
    }
    let Some(network) = state.networks.iter().find(|n| n.id == id || n.name == id) else {
        return Some("This network no longer exists".to_string());
    };
    if network.built_in {
        return Some(networks::BUILT_IN_REASON.to_string());
    }
    if network.containers.is_empty() {
        None
    } else {
        Some(format!(
            "{} connected",
            plural(network.containers.len(), "container is", "containers are")
        ))
    }
}

// ---------------------------------------------------------------------
// Settling after bollard's 2 s request timeout
// ---------------------------------------------------------------------

/// How long to wait for an engine that answered a container action with
/// `Timeout` (bollard's 2 s request timeout, PLAN §6.5): the engine keeps
/// working server-side, so Berth polls the container until the action's
/// outcome shows. The wait is the action's own grace period (`timeout_s`,
/// 10 s when unset) plus `grace`.
#[derive(Debug, Clone, Copy)]
pub struct Settle {
    pub interval: std::time::Duration,
    pub grace: std::time::Duration,
}

impl Default for Settle {
    fn default() -> Self {
        Self {
            interval: std::time::Duration::from_millis(250),
            grace: std::time::Duration::from_secs(20),
        }
    }
}

/// True when `observed` (the container after the action, `None` when the
/// engine no longer knows it) shows the outcome of `action`. Pure.
#[must_use]
pub fn settled(
    action: &ContainerAction,
    observed: Option<&berth_core::Container>,
    before_started: Option<chrono::DateTime<chrono::Utc>>,
) -> bool {
    use berth_core::ContainerState as S;
    match (action, observed) {
        (
            ContainerAction::Remove { .. }
            | ContainerAction::Stop { .. }
            | ContainerAction::Kill { .. },
            None,
        ) => true,
        (_, None) => false,
        (ContainerAction::Remove { .. }, Some(_)) => false,
        (ContainerAction::Stop { .. } | ContainerAction::Kill { .. }, Some(c)) => {
            !matches!(c.state, S::Running | S::Restarting | S::Paused)
        }
        (ContainerAction::Start | ContainerAction::Unpause, Some(c)) => c.state == S::Running,
        (ContainerAction::Restart { .. }, Some(c)) => {
            c.state == S::Running && c.started.is_some() && c.started != before_started
        }
        (ContainerAction::Pause, Some(c)) => c.state == S::Paused,
        (ContainerAction::Rename { name }, Some(c)) => c.name == *name,
    }
}

fn grace_secs(action: &ContainerAction) -> u64 {
    match action {
        ContainerAction::Stop { timeout_s } | ContainerAction::Restart { timeout_s } => {
            u64::from(timeout_s.unwrap_or(10))
        }
        _ => 0,
    }
}

/// Run one container action to its outcome. A `Timeout` from the engine is
/// not a failure: poll until [`settled`], or give up with a message.
pub async fn run_container(
    client: &dyn berth_engine::EngineClient,
    id: &berth_core::ContainerId,
    action: ContainerAction,
    before_started: Option<chrono::DateTime<chrono::Utc>>,
    settle: Settle,
    cancel: &CancellationToken,
) -> Result<(), String> {
    // A restart is settled by a new start time, judged by inspecting the
    // container (the list may carry none, as the live list does not). When
    // the caller has no baseline, read it before acting.
    let before_started = match (&action, before_started) {
        (ContainerAction::Restart { .. }, None) => client
            .container(id)
            .await
            .ok()
            .and_then(|detail| detail.container.started),
        (_, known) => known,
    };
    let first = tokio::select! {
        () = cancel.cancelled() => return Err("cancelled".to_string()),
        result = client.act(id, action.clone()) => result,
    };
    match first {
        Ok(()) => Ok(()),
        Err(berth_engine::EngineError::Timeout(_)) => {
            let wait = std::time::Duration::from_secs(grace_secs(&action)) + settle.grace;
            let deadline = tokio::time::Instant::now() + wait;
            loop {
                tokio::select! {
                    () = cancel.cancelled() => return Err("cancelled".to_string()),
                    () = tokio::time::sleep(settle.interval) => {}
                }
                match client.container(id).await {
                    Ok(detail) => {
                        if settled(&action, Some(&detail.container), before_started) {
                            return Ok(());
                        }
                    }
                    Err(berth_engine::EngineError::NotFound(_)) => {
                        if settled(&action, None, before_started) {
                            return Ok(());
                        }
                    }
                    Err(_) => {}
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!(
                        "The engine did not finish within {} s",
                        wait.as_secs().max(1)
                    ));
                }
            }
        }
        Err(err) => Err(err.to_string()),
    }
}

// ---------------------------------------------------------------------
// Pending state
// ---------------------------------------------------------------------

/// The result of one action task, applied to the signals on the UI thread.
#[derive(Debug)]
pub struct Finished {
    pub key: PendingKey,
    pub result: Result<(), String>,
    /// The container that took the place of the one the action ran on
    /// (a recreate gives a new id); a selection of the old one follows it.
    pub replaced_by: Option<berth_core::ContainerId>,
    /// The result toast, for an action that words its own.
    pub message: Option<String>,
}

/// Success clears the pending state; failure replaces it with
/// `PendingState::Failed` carrying the engine's message. The row itself is
/// never touched: it still shows what the engine last said.
pub fn apply_finished(state: &mut AppState, done: Finished) {
    let succeeded = done.result.is_ok();
    {
        let mut pending = state.pending.write();
        match done.result {
            Ok(()) => {
                pending.remove(&done.key);
            }
            Err(message) => {
                pending.insert(done.key.clone(), PendingState::Failed { message });
            }
        }
    }
    if !succeeded {
        return;
    }
    if let (Some(new), Subject::Container(old)) = (done.replaced_by, &done.key.subject) {
        let follows = matches!(
            &*state.selection.peek(),
            Selection::Containers(ids) if ids.len() == 1 && ids[0] == *old
        );
        if follows {
            state.selection.set(Selection::Containers(vec![new]));
        }
    }
    if let Some(text) = done.message {
        state.toasts.write().push(crate::state::Toast {
            id: uuid::Uuid::now_v7(),
            text,
            tone: crate::state::ToastTone::Success,
            action: None,
        });
    }
}

/// Clear a failure message (the user dismissed the `InlineMessage`). A
/// running action is left alone.
pub fn dismiss(state: &mut AppState, key: &PendingKey) {
    let mut pending = state.pending.write();
    if matches!(pending.get(key), Some(PendingState::Failed { .. })) {
        pending.remove(key);
    }
}

// ---------------------------------------------------------------------
// Targets and the service
// ---------------------------------------------------------------------

/// What a container action needs to know about its row.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerTarget {
    pub engine: berth_core::EngineId,
    pub id: berth_core::ContainerId,
    pub name: String,
    pub project: Option<berth_core::ProjectId>,
    pub started: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<&berth_core::Container> for ContainerTarget {
    fn from(c: &berth_core::Container) -> Self {
        Self {
            engine: c.engine.clone(),
            id: c.id.clone(),
            name: c.name.clone(),
            project: c.project.clone(),
            started: c.started,
        }
    }
}

enum Work {
    Container {
        client: Arc<dyn berth_engine::EngineClient>,
        id: berth_core::ContainerId,
        action: ContainerAction,
        before_started: Option<chrono::DateTime<chrono::Utc>>,
    },
    Compose {
        target: berth_core::ComposeTarget,
        command: berth_core::ComposeCommand,
    },
    Colima {
        profile: String,
        op: berth_core::ColimaOp,
    },
    Edit {
        client: Arc<dyn berth_engine::EngineClient>,
        id: berth_core::ContainerId,
        op: EditOp,
    },
    Network {
        client: Arc<dyn berth_engine::EngineClient>,
        op: networks::NetOp,
    },
    Remove {
        client: Arc<dyn berth_engine::EngineClient>,
        target: berth_core::ResourceRef,
        force: bool,
    },
}

struct Job {
    key: PendingKey,
    engine: berth_core::EngineId,
    name: String,
    project: Option<berth_core::ProjectId>,
    work: Work,
}

#[derive(Clone)]
struct Inner {
    clients: ClientMap,
    store: Arc<berth_store::Store>,
    compose: Arc<dyn berth_engine::ComposeRunner>,
    colima: Arc<dyn berth_engine::ColimaControl>,
    tx: tokio::sync::mpsc::UnboundedSender<Finished>,
    cancel: CancellationToken,
    settle: Settle,
}

impl Inner {
    async fn execute(&self, job: &Job) -> Result<edit::Done, String> {
        let cancel = self.cancel.child_token();
        let plain = |result: Result<(), String>| result.map(|()| edit::Done::default());
        match &job.work {
            Work::Container {
                client,
                id,
                action,
                before_started,
            } => plain(
                run_container(
                    client.as_ref(),
                    id,
                    action.clone(),
                    *before_started,
                    self.settle,
                    &cancel,
                )
                .await,
            ),
            Work::Compose { target, command } => {
                plain(compose::run(self.compose.as_ref(), target, command.clone(), cancel).await)
            }
            Work::Colima { profile, op } => {
                plain(colima::run(self.colima.as_ref(), profile, op.clone(), cancel).await)
            }
            Work::Edit { client, id, op } => {
                edit::run_edit(client.as_ref(), id, &job.name, op.clone(), &cancel).await
            }
            Work::Network { client, op } => {
                networks::run_net(client.as_ref(), &job.name, op.clone(), &cancel).await
            }
            // The confirm dialog says this result itself, from its summary.
            Work::Remove {
                client,
                target,
                force,
            } => edit::guard(
                &cancel,
                berth_engine::remove_resource(client.as_ref(), target, *force),
            )
            .await
            .map(|()| edit::Done::default()),
        }
    }
}

pub(crate) fn spawn_io(future: impl std::future::Future<Output = ()> + Send + 'static) {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(future);
    } else {
        spawn(future);
    }
}

/// The actions service. Cheap to clone; provide it as context from the App
/// root next to `Services`.
#[derive(Clone)]
pub struct ActionsService {
    inner: Arc<Inner>,
}

impl ActionsService {
    /// A service plus the receiver of its [`Finished`] messages. [`start`]
    /// wires the receiver to the signals; tests drain it by hand.
    ///
    /// [`start`]: ActionsService::start
    pub fn new(
        clients: ClientMap,
        store: Arc<berth_store::Store>,
        compose: Arc<dyn berth_engine::ComposeRunner>,
        colima: Arc<dyn berth_engine::ColimaControl>,
        cancel: CancellationToken,
    ) -> (Self, tokio::sync::mpsc::UnboundedReceiver<Finished>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let service = Self {
            inner: Arc::new(Inner {
                clients,
                store,
                compose,
                colima,
                tx,
                cancel,
                settle: Settle::default(),
            }),
        };
        (service, rx)
    }

    /// The same service with different [`Settle`] timing (tests).
    #[must_use]
    pub fn with_settle(&self, settle: Settle) -> Self {
        Self {
            inner: Arc::new(Inner {
                settle,
                ..(*self.inner).clone()
            }),
        }
    }

    /// Start from the App root (call inside `use_hook`): builds the service
    /// and spawns the Dioxus task that applies each [`Finished`].
    pub fn start(
        mut state: AppState,
        clients: ClientMap,
        store: Arc<berth_store::Store>,
        compose: Arc<dyn berth_engine::ComposeRunner>,
        colima: Arc<dyn berth_engine::ColimaControl>,
        cancel: CancellationToken,
    ) -> Self {
        let (service, mut rx) = Self::new(clients, store, compose, colima, cancel);
        spawn(async move {
            while let Some(done) = rx.recv().await {
                apply_finished(&mut state, done);
            }
        });
        service
    }

    fn client_for(
        &self,
        engine: &berth_core::EngineId,
    ) -> Result<Arc<dyn berth_engine::EngineClient>, String> {
        let map = match self.inner.clients.lock() {
            Ok(map) => map,
            Err(poisoned) => poisoned.into_inner(),
        };
        map.get(engine)
            .cloned()
            .ok_or_else(|| format!("Engine {} is not connected", engine.0))
    }

    /// Refuse an action `disabled_reason` rules out, before anything is set.
    fn check(state: &AppState, subject: &Subject, action: &'static str) -> Result<(), String> {
        match with_view(state, |view| disabled_reason(subject, action, view)) {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }

    /// Set the pending state now, then run the job in a task.
    fn launch(&self, state: &mut AppState, job: Job) {
        state.pending.write().insert(
            job.key.clone(),
            PendingState::Running {
                label: pending_label(job.key.action).to_string(),
            },
        );
        let inner = Arc::clone(&self.inner);
        spawn_io(async move {
            let executed = inner.execute(&job).await;
            let (result, replaced_by, message) = match executed {
                Ok(done) => (Ok(()), done.replaced_by, done.message),
                Err(reason) => (Err(reason), None, None),
            };
            let outcome = match &result {
                Ok(()) => berth_core::Outcome::Ok,
                Err(reason) => berth_core::Outcome::Failed {
                    reason: reason.clone(),
                },
            };
            activity::record(
                &inner.store,
                &activity::entry_for_action(
                    job.engine.clone(),
                    job.key.action,
                    job.key.subject.clone(),
                    job.name.clone(),
                    job.project.clone(),
                    outcome,
                ),
            );
            // The receiver is gone only at shutdown.
            let _ = inner.tx.send(Finished {
                key: job.key,
                result,
                replaced_by,
                message,
            });
        });
    }

    /// One container action. `Err` carries the disabled reason (or a
    /// missing connection); nothing is set pending in that case.
    pub fn container(
        &self,
        state: &mut AppState,
        target: &ContainerTarget,
        action: ContainerAction,
    ) -> Result<(), String> {
        let key = PendingKey {
            subject: Subject::Container(target.id.clone()),
            action: container_action_id(&action),
        };
        Self::check(state, &key.subject, key.action)?;
        let client = self.client_for(&target.engine)?;
        self.launch(
            state,
            Job {
                key,
                engine: target.engine.clone(),
                name: target.name.clone(),
                project: target.project.clone(),
                work: Work::Container {
                    client,
                    id: target.id.clone(),
                    action,
                    before_started: target.started,
                },
            },
        );
        Ok(())
    }

    /// One edit of a container: apply limits, recreate, recreate with the
    /// latest image, duplicate (P3-T7). Same contract as [`container`]:
    /// `Err` carries the reason and nothing is set pending.
    ///
    /// [`container`]: ActionsService::container
    pub fn edit(
        &self,
        state: &mut AppState,
        target: &ContainerTarget,
        op: EditOp,
    ) -> Result<(), String> {
        let key = PendingKey {
            subject: Subject::Container(target.id.clone()),
            action: op.action_id(),
        };
        Self::check(state, &key.subject, key.action)?;
        let client = self.client_for(&target.engine)?;
        self.launch(
            state,
            Job {
                key,
                engine: target.engine.clone(),
                name: target.name.clone(),
                project: target.project.clone(),
                work: Work::Edit {
                    client,
                    id: target.id.clone(),
                    op,
                },
            },
        );
        Ok(())
    }

    /// One network action (P4-T5): create, delete, connect, disconnect.
    /// `id` is the network's id (its name for a create, which has none yet)
    /// and keys the pending state; `name` is what toasts and the activity
    /// entry call it. Same contract as [`container`]: `Err` carries the
    /// reason and nothing is set pending.
    ///
    /// [`container`]: ActionsService::container
    pub fn network(
        &self,
        state: &mut AppState,
        engine: &berth_core::EngineId,
        id: &str,
        name: &str,
        op: networks::NetOp,
    ) -> Result<(), String> {
        let key = PendingKey {
            subject: Subject::Network(id.to_string()),
            action: op.action_id(),
        };
        Self::check(state, &key.subject, key.action)?;
        let client = self.client_for(engine)?;
        self.launch(
            state,
            Job {
                key,
                engine: engine.clone(),
                name: name.to_string(),
                project: None,
                work: Work::Network { client, op },
            },
        );
        Ok(())
    }

    /// Delete one image, volume or network (`EngineAction::Remove`, D25a).
    /// The pending key's subject and action come from `target`; `name` is
    /// what the activity entry calls it. Same contract as [`container`]:
    /// `Err` carries the reason (an image in use, a network with containers
    /// on it, a built-in network) and nothing is set pending.
    ///
    /// [`container`]: ActionsService::container
    pub fn remove(
        &self,
        state: &mut AppState,
        engine: &berth_core::EngineId,
        target: &berth_core::ResourceRef,
        name: &str,
        force: bool,
    ) -> Result<(), String> {
        let key = PendingKey {
            subject: target.subject(),
            action: target.action_id(),
        };
        Self::check(state, &key.subject, key.action)?;
        let client = self.client_for(engine)?;
        self.launch(
            state,
            Job {
                key,
                engine: engine.clone(),
                name: name.to_string(),
                project: None,
                work: Work::Remove {
                    client,
                    target: target.clone(),
                    force,
                },
            },
        );
        Ok(())
    }

    /// The same action on several containers in one call: every pending
    /// state is set before this returns and the engine calls run
    /// concurrently. One result per target, in order.
    pub fn containers(
        &self,
        state: &mut AppState,
        targets: &[ContainerTarget],
        action: &ContainerAction,
    ) -> Vec<Result<(), String>> {
        targets
            .iter()
            .map(|target| self.container(state, target, action.clone()))
            .collect()
    }

    /// One project action through Compose.
    pub fn project(
        &self,
        state: &mut AppState,
        project: &berth_core::Project,
        op: ProjectOp,
    ) -> Result<(), String> {
        let key = PendingKey {
            subject: Subject::Project(project.id.clone()),
            action: op.action_id(),
        };
        Self::check(state, &key.subject, key.action)?;
        let client = self.client_for(&project.engine)?;
        let target = compose::target_for(project, client.docker_host())?;
        self.launch(
            state,
            Job {
                key,
                engine: project.engine.clone(),
                name: project.name.clone(),
                project: Some(project.id.clone()),
                work: Work::Compose {
                    target,
                    command: op.command(),
                },
            },
        );
        Ok(())
    }

    /// One Colima operation on an engine (start, stop, restart, apply).
    pub fn engine(
        &self,
        state: &mut AppState,
        engine: &berth_core::Engine,
        op: berth_core::ColimaOp,
    ) -> Result<(), String> {
        let key = PendingKey {
            subject: Subject::Engine(engine.id.clone()),
            action: colima::action_id(&op),
        };
        Self::check(state, &key.subject, key.action)?;
        let Some(profile) = colima::profile_of(engine) else {
            return Err("Only Colima engines can be started or stopped here".to_string());
        };
        self.launch(
            state,
            Job {
                key,
                engine: engine.id.clone(),
                name: engine.name.clone(),
                project: None,
                work: Work::Colima {
                    profile: profile.to_string(),
                    op,
                },
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use berth_engine::client::Bs;
    use berth_engine::ColimaControl as _;
    use berth_engine::EngineClient as _;
    use berth_engine::EngineError;
    use futures::StreamExt;

    // --- VirtualDom harness: real signals, captured from the root. ---

    thread_local! {
        static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
    }

    #[component]
    fn Harness() -> Element {
        let state = crate::state::use_app_state();
        use_hook(move || {
            SLOT.with(|slot| *slot.borrow_mut() = Some(state));
        });
        rsx! { div {} }
    }

    fn harness() -> (VirtualDom, AppState) {
        let mut dom = VirtualDom::new(Harness);
        let _ = dom.rebuild_to_vec();
        let state = SLOT
            .with(|slot| slot.borrow_mut().take())
            .expect("harness captured AppState");
        (dom, state)
    }

    // --- Test double: MockEngine with a scripted `act`. ---

    enum Mode {
        /// Delegate straight to the mock.
        Normal,
        /// Wait for the gate before acting.
        Hold(Arc<tokio::sync::Semaphore>),
        /// Answer with a Docker error.
        Fail(String),
        /// Answer `Timeout` at once; the mock applies the action `after` later.
        SlowAct(std::time::Duration),
        /// Answer `Timeout` and never apply it.
        TimeoutOnly,
    }

    struct TestEngine {
        inner: Arc<berth_engine::MockEngine>,
        mode: Mode,
        acts: std::sync::Mutex<Vec<(berth_core::ContainerId, ContainerAction)>>,
    }

    impl TestEngine {
        fn new(mode: Mode) -> Arc<Self> {
            Arc::new(Self {
                inner: Arc::new(berth_engine::MockEngine::bundled(&engine_id())),
                mode,
                acts: std::sync::Mutex::new(Vec::new()),
            })
        }
        fn acts(&self) -> Vec<(berth_core::ContainerId, ContainerAction)> {
            self.acts.lock().expect("acts lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl berth_engine::EngineClient for TestEngine {
        fn id(&self) -> &berth_core::EngineId {
            self.inner.id()
        }
        fn docker_host(&self) -> &str {
            self.inner.docker_host()
        }
        async fn info(&self) -> Result<berth_core::EngineInfo, EngineError> {
            self.inner.info().await
        }
        async fn containers(&self) -> Result<Vec<berth_core::Container>, EngineError> {
            self.inner.containers().await
        }
        async fn container(
            &self,
            id: &berth_core::ContainerId,
        ) -> Result<berth_core::ContainerDetail, EngineError> {
            self.inner.container(id).await
        }
        async fn images(&self) -> Result<Vec<berth_core::Image>, EngineError> {
            self.inner.images().await
        }
        async fn image(&self, id: &str) -> Result<berth_core::ImageDetail, EngineError> {
            self.inner.image(id).await
        }
        async fn volumes(&self) -> Result<Vec<berth_core::Volume>, EngineError> {
            self.inner.volumes().await
        }
        async fn networks(&self) -> Result<Vec<berth_core::Network>, EngineError> {
            self.inner.networks().await
        }
        async fn disk_usage(&self) -> Result<berth_core::DiskUsage, EngineError> {
            self.inner.disk_usage().await
        }
        async fn volume_sizes(
            &self,
        ) -> Result<std::collections::BTreeMap<String, bytesize::ByteSize>, EngineError> {
            self.inner.volume_sizes().await
        }
        fn events(&self, cancel: CancellationToken) -> Bs<berth_core::EngineEvent> {
            self.inner.events(cancel)
        }
        fn logs(
            &self,
            id: &berth_core::ContainerId,
            q: berth_core::LogQuery,
            cancel: CancellationToken,
        ) -> Bs<berth_core::LogLine> {
            self.inner.logs(id, q, cancel)
        }
        fn stats(
            &self,
            id: &berth_core::ContainerId,
            cancel: CancellationToken,
        ) -> Bs<berth_core::LiveStats> {
            self.inner.stats(id, cancel)
        }
        async fn act(
            &self,
            id: &berth_core::ContainerId,
            action: ContainerAction,
        ) -> Result<(), EngineError> {
            self.acts
                .lock()
                .expect("acts lock")
                .push((id.clone(), action.clone()));
            match &self.mode {
                Mode::Normal => self.inner.act(id, action).await,
                Mode::Hold(gate) => {
                    if let Ok(permit) = gate.acquire().await {
                        permit.forget();
                    }
                    self.inner.act(id, action).await
                }
                Mode::Fail(message) => Err(EngineError::Docker(message.clone())),
                Mode::SlowAct(after) => {
                    let inner = Arc::clone(&self.inner);
                    let id = id.clone();
                    let after = *after;
                    tokio::spawn(async move {
                        tokio::time::sleep(after).await;
                        let _ = inner.act(&id, action).await;
                    });
                    Err(EngineError::Timeout(std::time::Duration::from_secs(2)))
                }
                Mode::TimeoutOnly => Err(EngineError::Timeout(std::time::Duration::from_secs(2))),
            }
        }
        async fn update_limits(
            &self,
            id: &berth_core::ContainerId,
            limits: &berth_core::Limits,
            restart: &berth_core::RestartPolicy,
        ) -> Result<(), EngineError> {
            self.inner.update_limits(id, limits, restart).await
        }
        async fn create(
            &self,
            spec: &berth_core::ContainerSpec,
            start: bool,
        ) -> Result<berth_core::ContainerId, EngineError> {
            self.inner.create(spec, start).await
        }
        async fn recreate(
            &self,
            id: &berth_core::ContainerId,
            spec: &berth_core::ContainerSpec,
        ) -> Result<berth_core::ContainerId, EngineError> {
            self.inner.recreate(id, spec).await
        }
        async fn exec(
            &self,
            id: &berth_core::ContainerId,
            req: berth_core::ExecRequest,
            cancel: CancellationToken,
        ) -> Result<berth_engine::ExecHandle, EngineError> {
            self.inner.exec(id, req, cancel).await
        }
        async fn list_files(
            &self,
            target: &berth_core::FileTarget,
            path: &str,
        ) -> Result<Vec<berth_core::FileEntry>, EngineError> {
            self.inner.list_files(target, path).await
        }
        fn download(
            &self,
            target: &berth_core::FileTarget,
            path: &str,
            cancel: CancellationToken,
        ) -> Bs<bytes::Bytes> {
            self.inner.download(target, path, cancel)
        }
        async fn upload(
            &self,
            target: &berth_core::FileTarget,
            dir: &str,
            tar: bytes::Bytes,
        ) -> Result<(), EngineError> {
            self.inner.upload(target, dir, tar).await
        }
        fn pull(
            &self,
            reference: &str,
            platform: Option<&str>,
            cancel: CancellationToken,
        ) -> Bs<berth_core::PullProgress> {
            self.inner.pull(reference, platform, cancel)
        }
        async fn remove_image(&self, id: &str, force: bool) -> Result<(), EngineError> {
            self.inner.remove_image(id, force).await
        }
        async fn search_images(
            &self,
            term: &str,
            limit: u32,
        ) -> Result<Vec<berth_core::ImageSearchHit>, EngineError> {
            self.inner.search_images(term, limit).await
        }
        async fn create_volume(
            &self,
            spec: &berth_core::VolumeSpec,
        ) -> Result<berth_core::Volume, EngineError> {
            self.inner.create_volume(spec).await
        }
        async fn remove_volume(&self, name: &str) -> Result<(), EngineError> {
            self.inner.remove_volume(name).await
        }
        async fn create_network(
            &self,
            spec: &berth_core::NetworkSpec,
        ) -> Result<berth_core::Network, EngineError> {
            self.inner.create_network(spec).await
        }
        async fn remove_network(&self, id: &str) -> Result<(), EngineError> {
            self.inner.remove_network(id).await
        }
        async fn connect_network(
            &self,
            network: &str,
            id: &berth_core::ContainerId,
        ) -> Result<(), EngineError> {
            self.inner.connect_network(network, id).await
        }
        async fn disconnect_network(
            &self,
            network: &str,
            id: &berth_core::ContainerId,
        ) -> Result<(), EngineError> {
            self.inner.disconnect_network(network, id).await
        }
        async fn prune(
            &self,
            plan: &berth_core::PrunePlan,
            cancel: CancellationToken,
        ) -> Result<berth_core::PruneReport, EngineError> {
            self.inner.prune(plan, cancel).await
        }
    }

    /// A compose runner that fails with a message on every command.
    struct FailingCompose(&'static str);

    #[async_trait::async_trait]
    impl berth_engine::ComposeRunner for FailingCompose {
        fn run(
            &self,
            _target: &berth_core::ComposeTarget,
            _cmd: berth_core::ComposeCommand,
            _cancel: CancellationToken,
        ) -> Bs<berth_core::ComposeEvent> {
            futures::stream::iter([
                Ok(berth_core::ComposeEvent::Line {
                    stream: berth_core::Stream::Stderr,
                    text: self.0.to_string(),
                }),
                Ok(berth_core::ComposeEvent::Done { code: 1 }),
            ])
            .boxed()
        }
        async fn config(
            &self,
            _target: &berth_core::ComposeTarget,
        ) -> Result<serde_json::Value, EngineError> {
            Ok(serde_json::json!({}))
        }
        async fn validate(
            &self,
            _folder: &std::path::Path,
            _files: &[std::path::PathBuf],
        ) -> Result<(), EngineError> {
            Ok(())
        }
    }

    fn engine_id() -> berth_core::EngineId {
        berth_core::EngineId("colima:default".to_string())
    }

    struct Rig {
        dom: VirtualDom,
        state: AppState,
        service: ActionsService,
        rx: tokio::sync::mpsc::UnboundedReceiver<Finished>,
        store: Arc<berth_store::Store>,
        engine: Arc<TestEngine>,
        compose: Arc<berth_engine::MockCompose>,
        colima: Arc<berth_engine::MockColima>,
    }

    impl Rig {
        fn new(mode: Mode) -> Self {
            let (dom, mut state) = harness();
            let world = berth_engine::MockWorld::bundled();
            let engine = TestEngine::new(mode);
            let clients = crate::services::engines::client_map();
            clients.lock().expect("clients").insert(
                engine_id(),
                Arc::clone(&engine) as Arc<dyn berth_engine::EngineClient>,
            );
            let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
            let compose = Arc::new(berth_engine::MockCompose::bundled());
            let colima = Arc::new(berth_engine::MockColima::bundled());
            let (service, rx) = ActionsService::new(
                clients,
                Arc::clone(&store),
                Arc::clone(&compose) as Arc<dyn berth_engine::ComposeRunner>,
                Arc::clone(&colima) as Arc<dyn berth_engine::ColimaControl>,
                CancellationToken::new(),
            );
            let service = service.with_settle(Settle {
                interval: std::time::Duration::from_millis(10),
                grace: std::time::Duration::from_millis(300),
            });
            dom.in_runtime(|| {
                state.engines.set(world.engines.clone());
                state.containers.set(world.containers_for(&engine_id()));
                state.projects.set(world.projects.clone());
                state
                    .images
                    .set(world.images_by_engine[&engine_id().0].clone());
                state
                    .volumes
                    .set(world.volumes_by_engine[&engine_id().0].clone());
                state
                    .networks
                    .set(world.networks_by_engine[&engine_id().0].clone());
            });
            Self {
                dom,
                state,
                service,
                rx,
                store,
                engine,
                compose,
                colima,
            }
        }

        fn container(&self, name: &str) -> berth_core::Container {
            self.dom.in_runtime(|| {
                self.state
                    .containers
                    .peek()
                    .iter()
                    .find(|c| c.name == name)
                    .cloned()
                    .unwrap_or_else(|| panic!("mock container {name}"))
            })
        }

        fn target(&self, name: &str) -> ContainerTarget {
            ContainerTarget::from(&self.container(name))
        }

        fn project(&self, name: &str) -> berth_core::Project {
            self.dom.in_runtime(|| {
                self.state
                    .projects
                    .peek()
                    .iter()
                    .find(|p| p.name == name)
                    .cloned()
                    .unwrap_or_else(|| panic!("mock project {name}"))
            })
        }

        fn pending(&self) -> Vec<(PendingKey, PendingState)> {
            self.dom.in_runtime(|| {
                self.state
                    .pending
                    .peek()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
        }

        fn do_container(&mut self, name: &str, action: ContainerAction) -> Result<(), String> {
            let target = self.target(name);
            let service = self.service.clone();
            let mut state = self.state;
            self.dom
                .in_runtime(|| service.container(&mut state, &target, action))
        }

        /// Wait for the next finished action and apply it.
        async fn settle_one(&mut self) -> PendingKey {
            let done = tokio::time::timeout(std::time::Duration::from_secs(5), self.rx.recv())
                .await
                .expect("a Finished within 5 s")
                .expect("channel open");
            let key = done.key.clone();
            let mut state = self.state;
            self.dom.in_runtime(|| apply_finished(&mut state, done));
            key
        }

        fn activity(&self) -> Vec<berth_core::ActivityEntry> {
            self.store
                .activity_query(&berth_store::ActivityFilter::default(), None, 50)
                .expect("activity")
        }
    }

    fn key(name: &str, rig: &Rig, action: &'static str) -> PendingKey {
        PendingKey {
            subject: Subject::Container(rig.container(name).id),
            action,
        }
    }

    const STOP: ContainerAction = ContainerAction::Stop { timeout_s: None };
    const RESTART: ContainerAction = ContainerAction::Restart { timeout_s: None };

    // --- Pending state ---

    #[tokio::test]
    async fn actions_pending_is_set_before_the_engine_answers() {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let mut rig = Rig::new(Mode::Hold(Arc::clone(&gate)));
        rig.do_container("shop-api-api-1", STOP).expect("accepted");
        // Nothing has awaited yet: the engine has not been asked, the row is
        // unchanged, and the pending state is already visible.
        assert_eq!(
            rig.pending(),
            vec![(
                key("shop-api-api-1", &rig, "container.stop"),
                PendingState::Running {
                    label: "Stopping…".to_string()
                }
            )]
        );
        assert_eq!(
            rig.container("shop-api-api-1").state,
            berth_core::ContainerState::Running
        );
        gate.add_permits(1);
        rig.settle_one().await;
        assert!(rig.pending().is_empty(), "success clears the pending state");
    }

    #[tokio::test]
    async fn actions_labels_follow_the_action() {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let mut rig = Rig::new(Mode::Hold(gate));
        rig.do_container("shop-api-api-1", RESTART)
            .expect("restart");
        rig.do_container("shop-api-redis-1", ContainerAction::Pause)
            .expect("pause");
        rig.do_container("shop-api-db-1", ContainerAction::Start)
            .expect("start");
        let labels: Vec<(&str, String)> = rig
            .pending()
            .into_iter()
            .map(|(k, v)| match v {
                PendingState::Running { label } => (k.action, label),
                PendingState::Failed { message } => (k.action, message),
            })
            .collect();
        assert!(labels.contains(&("container.restart", "Restarting…".to_string())));
        assert!(labels.contains(&("container.pause", "Pausing…".to_string())));
        assert!(labels.contains(&("container.start", "Starting…".to_string())));
        assert_eq!(pending_label("project.start"), "Starting…");
        assert_eq!(pending_label("project.stop"), "Stopping…");
        assert_eq!(pending_label("engine.restart"), "Restarting…");
        assert_eq!(pending_label("project.rebuild"), "Rebuilding…");
    }

    #[tokio::test]
    async fn actions_success_updates_the_engine_and_clears_pending() {
        let mut rig = Rig::new(Mode::Normal);
        rig.do_container("shop-api-api-1", STOP).expect("accepted");
        rig.settle_one().await;
        assert!(rig.pending().is_empty());
        let detail = rig
            .engine
            .container(&rig.container("shop-api-api-1").id)
            .await
            .expect("detail");
        assert_eq!(detail.container.state, berth_core::ContainerState::Exited);
        assert_eq!(
            rig.engine.acts(),
            vec![(rig.container("shop-api-api-1").id, STOP)]
        );
    }

    #[tokio::test]
    async fn actions_failure_restores_the_row_and_sets_failed_with_the_engine_message() {
        let mut rig = Rig::new(Mode::Fail("port is already allocated".to_string()));
        let before = rig.container("shop-api-api-1");
        rig.do_container("shop-api-api-1", STOP).expect("accepted");
        assert!(matches!(rig.pending()[0].1, PendingState::Running { .. }));
        rig.settle_one().await;
        assert_eq!(
            rig.pending(),
            vec![(
                key("shop-api-api-1", &rig, "container.stop"),
                PendingState::Failed {
                    message: "port is already allocated".to_string()
                }
            )]
        );
        assert_eq!(rig.container("shop-api-api-1"), before, "row unchanged");
    }

    #[tokio::test]
    async fn actions_a_new_attempt_replaces_the_failure_and_dismiss_clears_it() {
        let mut rig = Rig::new(Mode::Fail("boom".to_string()));
        rig.do_container("shop-api-api-1", STOP).expect("first");
        rig.settle_one().await;
        let failed_key = key("shop-api-api-1", &rig, "container.stop");
        assert!(matches!(rig.pending()[0].1, PendingState::Failed { .. }));
        // A failure does not block retrying.
        rig.do_container("shop-api-api-1", STOP).expect("retry");
        assert!(matches!(rig.pending()[0].1, PendingState::Running { .. }));
        // Dismiss leaves a running action alone...
        let mut state = rig.state;
        rig.dom.in_runtime(|| dismiss(&mut state, &failed_key));
        assert_eq!(rig.pending().len(), 1);
        rig.settle_one().await;
        // ...and removes a failure.
        rig.dom.in_runtime(|| dismiss(&mut state, &failed_key));
        assert!(rig.pending().is_empty());
    }

    #[tokio::test]
    async fn actions_second_press_is_refused_while_the_first_runs() {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let mut rig = Rig::new(Mode::Hold(Arc::clone(&gate)));
        rig.do_container("shop-api-api-1", STOP).expect("first");
        let second = rig.do_container("shop-api-api-1", RESTART);
        assert_eq!(second, Err("Stopping… is still in progress".to_string()));
        assert_eq!(rig.pending().len(), 1);
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert_eq!(rig.engine.acts().len(), 1, "the engine was asked once");
        gate.add_permits(1);
        rig.settle_one().await;
    }

    #[tokio::test]
    async fn actions_disabled_action_is_refused_and_sets_nothing() {
        let mut rig = Rig::new(Mode::Normal);
        let result = rig.do_container("shop-api-api-1", ContainerAction::Start);
        assert_eq!(result, Err("Already running".to_string()));
        assert!(rig.pending().is_empty());
        assert!(rig.engine.acts().is_empty());
    }

    #[tokio::test]
    async fn actions_unconnected_engine_is_refused_before_pending() {
        let rig = Rig::new(Mode::Normal);
        let mut target = rig.target("shop-api-api-1");
        target.engine = berth_core::EngineId("socket:/nowhere".to_string());
        let service = rig.service.clone();
        let mut state = rig.state;
        let result = rig
            .dom
            .in_runtime(|| service.container(&mut state, &target, STOP));
        assert_eq!(
            result,
            Err("Engine socket:/nowhere is not connected".to_string())
        );
        assert!(rig.pending().is_empty());
    }

    // --- Activity ---

    #[tokio::test]
    async fn actions_every_action_writes_an_activity_entry_with_its_outcome() {
        let mut rig = Rig::new(Mode::Normal);
        let target = rig.target("shop-api-api-1");
        rig.do_container("shop-api-api-1", RESTART).expect("ok");
        rig.settle_one().await;
        let entries = rig.activity();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "container.restart");
        assert_eq!(entries[0].origin, berth_core::ActivityOrigin::Berth);
        assert_eq!(entries[0].subject, Subject::Container(target.id.clone()));
        assert_eq!(entries[0].subject_name, "shop-api-api-1");
        assert_eq!(entries[0].engine, engine_id());
        assert_eq!(entries[0].project, target.project);
        assert_eq!(entries[0].outcome, berth_core::Outcome::Ok);
    }

    #[tokio::test]
    async fn actions_failed_action_writes_a_failed_activity_entry() {
        let mut rig = Rig::new(Mode::Fail("no such container".to_string()));
        rig.do_container("shop-api-api-1", STOP).expect("accepted");
        rig.settle_one().await;
        let entries = rig.activity();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].outcome,
            berth_core::Outcome::Failed {
                reason: "no such container".to_string()
            }
        );
    }

    // --- Several containers in one call ---

    #[tokio::test]
    async fn actions_restart_of_three_containers_in_one_call() {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let mut rig = Rig::new(Mode::Hold(Arc::clone(&gate)));
        let names = ["shop-api-api-1", "shop-api-redis-1", "shop-api-mailpit-1"];
        let targets: Vec<ContainerTarget> = names.iter().map(|n| rig.target(n)).collect();
        let service = rig.service.clone();
        let mut state = rig.state;
        let results = rig
            .dom
            .in_runtime(|| service.containers(&mut state, &targets, &RESTART));
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        assert_eq!(
            rig.pending().len(),
            3,
            "all three pending before any answer"
        );
        for _ in 0..3 {
            gate.add_permits(1);
        }
        for _ in 0..3 {
            rig.settle_one().await;
        }
        assert!(rig.pending().is_empty());
        assert_eq!(rig.engine.acts().len(), 3);
        assert_eq!(rig.activity().len(), 3);
        for name in names {
            assert!(rig.container(name).restart_count < 1000);
        }
    }

    #[tokio::test]
    async fn actions_bulk_reports_each_refusal_and_runs_the_rest() {
        let mut rig = Rig::new(Mode::Normal);
        let targets = vec![rig.target("shop-api-api-1"), rig.target("shop-api-db-1")];
        let service = rig.service.clone();
        let mut state = rig.state;
        let results = rig
            .dom
            .in_runtime(|| service.containers(&mut state, &targets, &STOP));
        assert_eq!(results[0], Ok(()));
        assert_eq!(results[1], Err("Not running".to_string()));
        rig.settle_one().await;
        assert!(rig.pending().is_empty());
    }

    // --- bollard's 2 s timeout ---

    #[tokio::test]
    async fn actions_stop_that_times_out_is_polled_until_the_container_exits() {
        let mut rig = Rig::new(Mode::SlowAct(std::time::Duration::from_millis(80)));
        let started = std::time::Instant::now();
        rig.do_container(
            "shop-api-api-1",
            ContainerAction::Stop { timeout_s: Some(0) },
        )
        .expect("accepted");
        rig.settle_one().await;
        assert!(rig.pending().is_empty(), "polled to success, not Timeout");
        assert!(started.elapsed() >= std::time::Duration::from_millis(80));
        assert_eq!(rig.activity()[0].outcome, berth_core::Outcome::Ok);
    }

    /// The live container list carries no start times, so a target built
    /// from it has `started: None`. A restart that times out must still be
    /// judged by inspecting the container, against the start time read
    /// before the action: the old run is not "restarted".
    #[tokio::test]
    async fn actions_restart_that_times_out_waits_for_a_new_run_without_list_start_times() {
        let mut rig = Rig::new(Mode::SlowAct(std::time::Duration::from_millis(80)));
        let mut target = rig.target("shop-api-api-1");
        let old_started = rig.container("shop-api-api-1").started;
        assert!(old_started.is_some(), "the mock container has run before");
        target.started = None;
        let service = rig.service.clone();
        let mut state = rig.state;
        let began = std::time::Instant::now();
        rig.dom
            .in_runtime(|| {
                service.container(
                    &mut state,
                    &target,
                    ContainerAction::Restart { timeout_s: Some(0) },
                )
            })
            .expect("accepted");
        rig.settle_one().await;
        assert!(
            began.elapsed() >= std::time::Duration::from_millis(80),
            "settled on the old run after {:?}",
            began.elapsed()
        );
        assert!(rig.pending().is_empty(), "{:?}", rig.pending());
        assert_eq!(rig.activity()[0].outcome, berth_core::Outcome::Ok);
    }

    #[tokio::test]
    async fn actions_stop_that_never_settles_fails_with_a_message_not_a_timeout_error() {
        let mut rig = Rig::new(Mode::TimeoutOnly);
        rig.do_container(
            "shop-api-api-1",
            ContainerAction::Stop { timeout_s: Some(0) },
        )
        .expect("accepted");
        rig.settle_one().await;
        match &rig.pending()[0].1 {
            PendingState::Failed { message } => {
                assert!(
                    message.starts_with("The engine did not finish within"),
                    "{message}"
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn actions_settled_reads_the_outcome_of_each_action() {
        use berth_core::ContainerState as S;
        let mut running = berth_engine::MockWorld::bundled()
            .containers
            .into_iter()
            .find(|c| c.state == S::Running)
            .expect("running container");
        let t0 = chrono::DateTime::from_timestamp(1_000, 0);
        let t1 = chrono::DateTime::from_timestamp(2_000, 0);
        running.started = t0;
        let mut exited = running.clone();
        exited.state = S::Exited;
        assert!(!settled(&STOP, Some(&running), t0));
        assert!(settled(&STOP, Some(&exited), t0));
        assert!(settled(&STOP, None, t0), "gone counts as stopped");
        assert!(!settled(&ContainerAction::Start, Some(&exited), t0));
        assert!(settled(&ContainerAction::Start, Some(&running), t0));
        // Restart: still the old run, then a new one.
        assert!(!settled(&RESTART, Some(&running), t0));
        running.started = t1;
        assert!(settled(&RESTART, Some(&running), t0));
        assert!(!settled(&RESTART, Some(&exited), t0));
        assert!(!settled(
            &ContainerAction::Remove {
                force: false,
                volumes: false
            },
            Some(&running),
            t0
        ));
        assert!(settled(
            &ContainerAction::Remove {
                force: false,
                volumes: false
            },
            None,
            t0
        ));
    }

    // --- Project and engine actions ---

    #[tokio::test]
    async fn actions_stop_project_then_start_project_run_compose_stop_then_up() {
        let mut rig = Rig::new(Mode::Normal);
        let project = rig.project("shop-api");
        let service = rig.service.clone();
        let mut state = rig.state;
        rig.dom
            .in_runtime(|| service.project(&mut state, &project, ProjectOp::Stop))
            .expect("stop accepted");
        assert_eq!(
            rig.pending(),
            vec![(
                PendingKey {
                    subject: Subject::Project(project.id.clone()),
                    action: "project.stop"
                },
                PendingState::Running {
                    label: "Stopping…".to_string()
                }
            )]
        );
        rig.settle_one().await;
        assert!(rig.pending().is_empty());
        // shop-api is Failing (not stopped), so Start is allowed too.
        rig.dom
            .in_runtime(|| service.project(&mut state, &project, ProjectOp::Start))
            .expect("start accepted");
        rig.settle_one().await;
        let recorded = rig.compose.recorded();
        assert_eq!(recorded.len(), 2);
        assert_eq!(
            recorded[0].1,
            berth_core::ComposeCommand::Stop { services: vec![] }
        );
        assert_eq!(
            recorded[1].1,
            berth_core::ComposeCommand::Up {
                services: vec![],
                build: false
            }
        );
        assert_eq!(recorded[0].0.project_name, "shop-api");
        assert_eq!(
            recorded[0].0.folder,
            project.folder.clone().expect("folder")
        );
        assert_eq!(recorded[0].0.docker_host, rig.engine.docker_host());
        let actions: Vec<String> = rig.activity().into_iter().map(|e| e.action).collect();
        assert!(actions.contains(&"project.stop".to_string()));
        assert!(actions.contains(&"project.start".to_string()));
        assert!(rig
            .activity()
            .iter()
            .all(|e| e.project == Some(project.id.clone())));
    }

    #[tokio::test]
    async fn actions_delete_project_runs_down_with_the_sheets_options() {
        let mut rig = Rig::new(Mode::Normal);
        let project = rig.project("shop-api");
        let service = rig.service.clone();
        let mut state = rig.state;
        rig.dom
            .in_runtime(|| {
                service.project(
                    &mut state,
                    &project,
                    ProjectOp::Delete {
                        volumes: true,
                        remove_images: false,
                    },
                )
            })
            .expect("accepted");
        rig.settle_one().await;
        assert_eq!(
            rig.compose.recorded()[0].1,
            berth_core::ComposeCommand::Down {
                volumes: true,
                remove_images: false
            }
        );
    }

    #[tokio::test]
    async fn actions_project_failure_shows_the_compose_error() {
        let mut rig = Rig::new(Mode::Normal);
        let (service, rx) = ActionsService::new(
            Arc::clone(&rig.service.inner.clients),
            Arc::clone(&rig.store),
            Arc::new(FailingCompose("port 5432 is already allocated")),
            Arc::clone(&rig.colima) as Arc<dyn berth_engine::ColimaControl>,
            CancellationToken::new(),
        );
        rig.rx = rx;
        let project = rig.project("shop-api");
        let mut state = rig.state;
        rig.dom
            .in_runtime(|| service.project(&mut state, &project, ProjectOp::Restart))
            .expect("accepted");
        rig.service = service;
        rig.settle_one().await;
        assert_eq!(
            rig.pending()[0].1,
            PendingState::Failed {
                message: "port 5432 is already allocated".to_string()
            }
        );
        assert_eq!(
            rig.activity()[0].outcome,
            berth_core::Outcome::Failed {
                reason: "port 5432 is already allocated".to_string()
            }
        );
    }

    #[tokio::test]
    async fn actions_engine_stop_runs_colima_and_clears_pending() {
        let mut rig = Rig::new(Mode::Normal);
        let engine = rig
            .dom
            .in_runtime(|| {
                rig.state
                    .engines
                    .peek()
                    .iter()
                    .find(|e| e.id == engine_id())
                    .cloned()
            })
            .expect("engine");
        let service = rig.service.clone();
        let mut state = rig.state;
        rig.dom
            .in_runtime(|| service.engine(&mut state, &engine, berth_core::ColimaOp::Stop))
            .expect("accepted");
        assert_eq!(
            rig.pending()[0].1,
            PendingState::Running {
                label: "Stopping…".to_string()
            }
        );
        rig.settle_one().await;
        assert!(rig.pending().is_empty());
        assert!(!rig.colima.status("default").await.expect("profile").running);
        assert_eq!(rig.activity()[0].action, "engine.stop");
        assert_eq!(rig.activity()[0].subject, Subject::Engine(engine_id()));
    }

    // --- Disabled reasons ---

    fn reason(rig: &Rig, subject: &Subject, action: &str) -> Option<String> {
        rig.dom
            .in_runtime(|| with_view(&rig.state, |view| disabled_reason(subject, action, view)))
    }

    fn container_subject(rig: &Rig, name: &str) -> Subject {
        Subject::Container(rig.container(name).id)
    }

    #[tokio::test]
    async fn actions_disabled_reasons_for_containers_follow_their_state() {
        let rig = Rig::new(Mode::Normal);
        let running = container_subject(&rig, "shop-api-api-1");
        let exited = container_subject(&rig, "shop-api-db-1");
        let paused = container_subject(&rig, "api-worker-7f9c4d8b6-xk2lp");
        assert_eq!(
            reason(&rig, &running, "container.start").as_deref(),
            Some("Already running")
        );
        assert_eq!(reason(&rig, &running, "container.stop"), None);
        assert_eq!(reason(&rig, &running, "container.pause"), None);
        assert_eq!(
            reason(&rig, &running, "container.resume").as_deref(),
            Some("Not paused")
        );
        assert_eq!(reason(&rig, &exited, "container.start"), None);
        assert_eq!(
            reason(&rig, &exited, "container.stop").as_deref(),
            Some("Not running")
        );
        assert_eq!(
            reason(&rig, &exited, "container.pause").as_deref(),
            Some("Only a running container can be paused")
        );
        assert_eq!(
            reason(&rig, &exited, "container.kill").as_deref(),
            Some("Not running")
        );
        assert_eq!(reason(&rig, &exited, "container.delete"), None);
        assert_eq!(reason(&rig, &exited, "container.restart"), None);
        assert_eq!(reason(&rig, &paused, "container.resume"), None);
        assert_eq!(reason(&rig, &paused, "container.stop"), None);
        assert_eq!(
            reason(&rig, &paused, "container.start").as_deref(),
            Some("Paused. Resume it instead")
        );
        assert_eq!(
            reason(
                &rig,
                &Subject::Container(berth_core::ContainerId("gone".to_string())),
                "container.stop"
            )
            .as_deref(),
            Some("This container no longer exists")
        );
    }

    #[tokio::test]
    async fn actions_disabled_reason_names_the_stopped_engine() {
        let rig = Rig::new(Mode::Normal);
        let mut container = rig.container("shop-api-api-1");
        container.engine = berth_core::EngineId("colima:work".to_string());
        let mut state = rig.state;
        rig.dom
            .in_runtime(|| state.containers.set(vec![container.clone()]));
        assert_eq!(
            reason(&rig, &Subject::Container(container.id), "container.stop").as_deref(),
            Some("Engine work is stopped")
        );
    }

    #[tokio::test]
    async fn actions_delete_image_in_use_is_disabled_with_the_count() {
        let rig = Rig::new(Mode::Normal);
        let (used, unused) = rig.dom.in_runtime(|| {
            let images = rig.state.images.peek();
            (
                images
                    .iter()
                    .find(|i| i.used_by.len() == 2)
                    .cloned()
                    .expect("used by two"),
                images
                    .iter()
                    .find(|i| i.used_by.is_empty())
                    .cloned()
                    .expect("unused"),
            )
        });
        assert_eq!(
            reason(&rig, &Subject::Image(used.id.clone()), "image.delete").as_deref(),
            Some("Used by 2 containers")
        );
        assert_eq!(
            reason(&rig, &Subject::Image(unused.id), "image.delete"),
            None
        );
        assert_eq!(reason(&rig, &Subject::Image(used.id), "image.pull"), None);
        let one = rig.dom.in_runtime(|| {
            rig.state
                .images
                .peek()
                .iter()
                .find(|i| i.used_by.len() == 1)
                .cloned()
                .expect("one")
        });
        assert_eq!(
            reason(&rig, &Subject::Image(one.id), "image.delete").as_deref(),
            Some("Used by 1 container")
        );
    }

    #[tokio::test]
    async fn actions_delete_volume_and_network_in_use_are_disabled() {
        let rig = Rig::new(Mode::Normal);
        assert_eq!(
            reason(
                &rig,
                &Subject::Volume("shop-api_pgdata".to_string()),
                "volume.delete"
            )
            .as_deref(),
            Some("Used by 1 container")
        );
        assert_eq!(
            reason(
                &rig,
                &Subject::Volume("stale-cache".to_string()),
                "volume.delete"
            ),
            None
        );
        assert_eq!(
            reason(
                &rig,
                &Subject::Network("bridge".to_string()),
                "network.delete"
            )
            .as_deref(),
            Some("Built-in networks cannot be removed")
        );
        assert_eq!(
            reason(
                &rig,
                &Subject::Network("shop-api_default".to_string()),
                "network.delete"
            )
            .as_deref(),
            Some("5 containers are connected")
        );
    }

    #[tokio::test]
    async fn actions_disabled_reasons_for_projects_follow_status_and_files() {
        let rig = Rig::new(Mode::Normal);
        let id = |name: &str| Subject::Project(rig.project(name).id);
        // berth-site is AllRunning, ml-notebooks Stopped, docker-manger NotCreated
        // with no Compose file, landing is remote-only with no folder.
        assert_eq!(
            reason(&rig, &id("berth-site"), "project.start").as_deref(),
            Some("All services are already running")
        );
        assert_eq!(reason(&rig, &id("berth-site"), "project.stop"), None);
        assert_eq!(reason(&rig, &id("ml-notebooks"), "project.start"), None);
        assert_eq!(
            reason(&rig, &id("ml-notebooks"), "project.stop").as_deref(),
            Some("Nothing is running")
        );
        assert_eq!(
            reason(&rig, &id("ml-notebooks"), "project.restart").as_deref(),
            Some("Nothing is running")
        );
        assert_eq!(
            reason(&rig, &id("docker-manger"), "project.start").as_deref(),
            Some("No Compose file is known for this project")
        );
        assert_eq!(reason(&rig, &id("ml-notebooks"), "project.delete"), None);
        assert_eq!(
            reason(&rig, &id("landing"), "project.stop").as_deref(),
            Some("Engine hetzner-1 is unreachable")
        );
    }

    #[tokio::test]
    async fn actions_disabled_reasons_for_engines() {
        let rig = Rig::new(Mode::Normal);
        let default = Subject::Engine(engine_id());
        let work = Subject::Engine(berth_core::EngineId("colima:work".to_string()));
        let ssh = Subject::Engine(berth_core::EngineId("ssh:hetzner-1".to_string()));
        assert_eq!(
            reason(&rig, &default, "engine.start").as_deref(),
            Some("Already running")
        );
        assert_eq!(reason(&rig, &default, "engine.stop"), None);
        assert_eq!(reason(&rig, &work, "engine.start"), None);
        assert_eq!(
            reason(&rig, &work, "engine.stop").as_deref(),
            Some("Already stopped")
        );
        assert_eq!(
            reason(&rig, &ssh, "engine.start").as_deref(),
            Some("Only Colima engines can be started or stopped here")
        );
    }
}
