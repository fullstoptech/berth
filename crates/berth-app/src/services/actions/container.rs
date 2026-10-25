//! Container work: one action on a container (and the same action on
//! several), the edits of P3-T7, and the settling poll after the engine's
//! 2 s request timeout.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use berth_core::{ContainerAction, Subject};

use super::{edit, ActionsService, EditOp, Inner, Job, Work};
use crate::state::{AppState, PendingKey};

// ---------------------------------------------------------------------
// Ids
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
// Targets and work
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

/// A container action sent to the engine.
struct ContainerWork {
    client: Arc<dyn berth_engine::EngineClient>,
    id: berth_core::ContainerId,
    action: ContainerAction,
    before_started: Option<chrono::DateTime<chrono::Utc>>,
}

#[async_trait::async_trait]
impl Work for ContainerWork {
    async fn execute(
        &self,
        inner: &Inner,
        _job: &Job,
        cancel: CancellationToken,
    ) -> Result<edit::Done, String> {
        run_container(
            self.client.as_ref(),
            &self.id,
            self.action.clone(),
            self.before_started,
            inner.settle,
            &cancel,
        )
        .await
        .map(|()| edit::Done::default())
    }
}

/// An edit of a container: limits, recreate, duplicate.
struct EditWork {
    client: Arc<dyn berth_engine::EngineClient>,
    id: berth_core::ContainerId,
    op: EditOp,
}

#[async_trait::async_trait]
impl Work for EditWork {
    async fn execute(
        &self,
        _inner: &Inner,
        job: &Job,
        cancel: CancellationToken,
    ) -> Result<edit::Done, String> {
        edit::run_edit(
            self.client.as_ref(),
            &self.id,
            &job.name,
            self.op.clone(),
            &cancel,
        )
        .await
    }
}

impl ActionsService {
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
                work: Box::new(ContainerWork {
                    client,
                    id: target.id.clone(),
                    action,
                    before_started: target.started,
                }),
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
                work: Box::new(EditWork {
                    client,
                    id: target.id.clone(),
                    op,
                }),
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
}
