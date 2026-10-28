//! Edit, duplicate and recreate (P3-T7, PLAN §6.5): the engine work behind
//! the Limits tab, the edit and recreate sheet and the ⋯ menu. The actions
//! service runs [`run_edit`] as one pending action per container.
//!
//! A failed recreate leaves the old container as it was: the engine
//! renames it back and starts it again (`EngineClient::recreate`), and a
//! failed pull happens before anything is touched.

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use berth_core::{ContainerId, ContainerSpec, Limits, RestartPolicy};

use crate::sheets::edit_recreate::model::duplicate_spec;

/// What an edit does to one container.
#[derive(Debug, Clone, PartialEq)]
pub enum EditOp {
    /// Limits and restart policy, on the running container.
    ApplyLimits {
        limits: Limits,
        restart: RestartPolicy,
    },
    /// Replace the container by one made from this spec, after pulling its
    /// image again when `pull` is set ("Recreate with latest image", AD18).
    Recreate {
        spec: Box<ContainerSpec>,
        pull: bool,
    },
    /// A new container from the same settings, named `<name>-copy`.
    /// Created, not started: it may want the ports the original holds.
    Duplicate,
}

impl EditOp {
    /// The live update from `old` to `new`. Docker refuses a new memory
    /// limit on a container with no swap setting unless the swap is set in
    /// the same update, so a memory limit that changed without a swap
    /// figure gets Docker's own default for `docker run -m`: twice the
    /// memory. A limit that did not change is passed as it is.
    #[must_use]
    pub fn apply_limits(old: &ContainerSpec, new: &ContainerSpec) -> Self {
        let mut limits = new.limits.clone();
        if let (Some(memory), None) = (limits.memory, limits.memory_swap) {
            if old.limits.memory != Some(memory) {
                limits.memory_swap = Some(bytesize::ByteSize(memory.0.saturating_mul(2)));
            }
        }
        Self::ApplyLimits {
            limits,
            restart: new.restart,
        }
    }

    /// The pending key and activity action of this op.
    #[must_use]
    pub fn action_id(&self) -> &'static str {
        match self {
            Self::ApplyLimits { .. } => "container.apply_limits",
            Self::Recreate { .. } => "container.recreate",
            Self::Duplicate => "container.duplicate",
        }
    }
}

/// What an edit left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Done {
    /// The container that took the place of the one edited.
    pub replaced_by: Option<ContainerId>,
    /// The toast to show, when the action says its own result.
    pub message: Option<String>,
}

/// Run one edit to its outcome.
///
/// # Errors
/// The engine's message. After a failed recreate the old container is
/// back (or the message says it could not be restored).
pub async fn run_edit(
    client: &dyn berth_engine::EngineClient,
    id: &ContainerId,
    name: &str,
    op: EditOp,
    cancel: &CancellationToken,
) -> Result<Done, String> {
    match op {
        EditOp::ApplyLimits { limits, restart } => {
            guard(cancel, client.update_limits(id, &limits, &restart)).await?;
            Ok(Done {
                replaced_by: None,
                message: Some(format!("Limits applied to {name}")),
            })
        }
        EditOp::Recreate {
            spec,
            pull: pull_first,
        } => {
            if pull_first {
                pull(client, &spec, cancel).await?;
            }
            let new = guard(cancel, client.recreate(id, &spec)).await?;
            // The sheet says the result itself; the latest-image dialog's
            // watcher says it from the same summary.
            Ok(Done {
                replaced_by: Some(new),
                message: (!pull_first).then(crate::services::confirm::recreate_result),
            })
        }
        EditOp::Duplicate => {
            let detail = guard(cancel, client.container(id)).await?;
            let taken: Vec<String> = guard(cancel, client.containers())
                .await?
                .into_iter()
                .map(|c| c.name)
                .collect();
            let taken: Vec<&str> = taken.iter().map(String::as_str).collect();
            let copy = duplicate_spec(&ContainerSpec::from_detail(&detail), &taken);
            let copy_name = copy.name.clone();
            guard(cancel, client.create(&copy, false)).await?;
            Ok(Done {
                replaced_by: None,
                message: Some(format!("Duplicated as {copy_name}")),
            })
        }
    }
}

pub(crate) async fn guard<T>(
    cancel: &CancellationToken,
    call: impl std::future::Future<Output = Result<T, berth_engine::EngineError>>,
) -> Result<T, String> {
    tokio::select! {
        // Cancelled wins a tie, so a cancelled edit always reports "cancelled".
        biased;
        () = cancel.cancelled() => Err("cancelled".to_string()),
        result = call => result.map_err(|e| e.to_string()),
    }
}

/// Pull the spec's image to the end. Nothing is touched when it fails.
async fn pull(
    client: &dyn berth_engine::EngineClient,
    spec: &ContainerSpec,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let reference = &spec.image.reference;
    let mut stream = client.pull(
        reference,
        spec.platform.as_deref(),
        None,
        cancel.child_token(),
    );
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => return Err("cancelled".to_string()),
            next = stream.next() => match next {
                None => return Ok(()),
                Some(Ok(_)) => {}
                Some(Err(err)) => {
                    return Err(format!("Could not pull {reference}: {err}. The container was not touched."));
                }
            },
        }
    }
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
