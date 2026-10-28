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
//!
//! One file per kind of work: each defines its [`Work`] and the
//! `ActionsService` method that starts it, so a new kind adds a file and a
//! `mod` line here. `reasons/` holds the disabled reasons, one file per
//! resource.

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_core::Subject;

use crate::services::activity;
use crate::services::engines::ClientMap;
use crate::services::{colima, edit};
use crate::state::{AppState, PendingKey, PendingState, Selection};

mod container;
mod engine;
mod folder;
mod network;
mod project;
mod reasons;
mod remove;
mod volume;

pub use crate::services::compose::ProjectOp;
pub use crate::services::edit::EditOp;
pub use container::{container_action_id, run_container, settled, ContainerTarget, Settle};
pub use reasons::{disabled_reason, with_view, StateView};

// ---------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------

/// The label shown while an action runs (D34); each area lists its own in
/// `actions/<area>.rs`.
#[must_use]
pub fn pending_label(action_id: &str) -> &'static str {
    crate::actions::pending_label(action_id)
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
    if let Subject::Engine(id) = &done.key.subject {
        let after = match done.key.action {
            "engine.start" | "engine.restart" | "engine.edit_resources" => {
                Some(berth_core::EngineState::Running)
            }
            "engine.stop" => Some(berth_core::EngineState::Stopped),
            _ => None,
        };
        if let Some(after) = after {
            if let Some(found) = state.engines.write().iter_mut().find(|e| e.id == *id) {
                found.state = after;
            }
        }
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
// The service
// ---------------------------------------------------------------------

/// One kind of work an action starts. The service sets the pending state,
/// runs [`Work::execute`] in a task and records the outcome; the kinds
/// differ only in what they run. Each kind lives in its own file.
#[async_trait::async_trait]
trait Work: Send + Sync {
    /// Before the pending state is set (Colima moves the engine row to
    /// Starting or Stopping here).
    fn begin(&self, _inner: &Inner, _state: &mut AppState, _job: &Job) {}

    /// Run the work to its outcome. `cancel` fires when the app quits.
    async fn execute(
        &self,
        inner: &Inner,
        job: &Job,
        cancel: CancellationToken,
    ) -> Result<edit::Done, String>;

    /// After [`Work::execute`], before the outcome is recorded.
    async fn finished(&self, _inner: &Inner, _job: &Job, _executed: &Result<edit::Done, String>) {}
}

struct Job {
    key: PendingKey,
    engine: berth_core::EngineId,
    name: String,
    project: Option<berth_core::ProjectId>,
    work: Box<dyn Work>,
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
    /// The output of the Colima commands now running (or just failed).
    runs: colima::Runs,
    /// Reconnects an engine once Colima has started it (set by the App root).
    link: Arc<std::sync::OnceLock<crate::services::engines::EngineLink>>,
    /// The state an engine had before a Colima command set it to Starting
    /// or Stopping, for the case the command fails.
    previous: Arc<
        std::sync::Mutex<std::collections::BTreeMap<berth_core::EngineId, berth_core::EngineState>>,
    >,
    /// The output of each project's last failed Compose command (AD33): set when a command fails,
    /// dropped when the next one succeeds. Written by the action task, read by the UI.
    compose_failures: Arc<std::sync::Mutex<ComposeFailures>>,
}

type ComposeFailures = std::collections::BTreeMap<berth_core::ProjectId, Vec<String>>;

pub(crate) fn spawn_io(future: impl std::future::Future<Output = ()> + Send + 'static) {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(future);
    } else {
        spawn(future);
    }
}

/// Run `future` on the UI thread for as long as the app lives, in the App component's scope.
///
/// Use it for work the user started whose result must land even if the caller unmounts (a
/// click handler that swaps its own page out, a sheet that closes). A plain `spawn` attaches to
/// the current component scope and is dropped with it. Keep scoped `spawn` for subscriptions
/// that should die with their view.
///
/// The task runs in `ScopeId::APP` (the user's root component, which never unmounts), not
/// `ScopeId::ROOT` (the runtime wrapper above it, where `spawn_forever` runs). The app's signals
/// are created in the App scope, and Dioxus warns ("Copy Value ... hoisted") when one is used
/// from a scope that is not the App scope or below it.
pub(crate) fn spawn_job(
    future: impl std::future::Future<Output = ()> + 'static,
) -> dioxus::core::Task {
    dioxus::core::Runtime::current().in_scope(dioxus::core::ScopeId::APP, || spawn(future))
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
                runs: colima::Runs::default(),
                link: Arc::default(),
                previous: Arc::default(),
                compose_failures: Arc::default(),
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
        let settling = service.clone();
        spawn(async move {
            while let Some(done) = rx.recv().await {
                let key = done.key.clone();
                let ok = done.result.is_ok();
                apply_finished(&mut state, done);
                settling.settle_engine(&mut state, &key, ok);
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
        job.work.begin(&self.inner, state, &job);
        state.pending.write().insert(
            job.key.clone(),
            PendingState::Running {
                label: pending_label(job.key.action).to_string(),
            },
        );
        let inner = Arc::clone(&self.inner);
        spawn_io(async move {
            let cancel = inner.cancel.child_token();
            let executed = job.work.execute(&inner, &job, cancel).await;
            job.work.finished(&inner, &job, &executed).await;
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
}

#[cfg(test)]
mod tests;
