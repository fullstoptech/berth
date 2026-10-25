//! The container detail service (P3-T3, PLAN §6.11, D23): what the
//! inspector shows beyond the list row. One `client.container(id)` inspect
//! per container whose list row changed, plus the last 20 log lines of an
//! exited container. The UI reads the store; this file owns the I/O.
//!
//! Env values live here, in memory, only so the Env tab can reveal one on
//! click (D79). They never reach a log, a tracing field, the activity
//! store, a harness or a plugin: [`Loaded`] has a hand-written `Debug` that
//! prints keys' count only, and the inspect JSON that goes into the tree is
//! the masked one.

use std::collections::HashMap;

use dioxus::prelude::*;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use berth_core::{Container, ContainerDetail, ContainerId, ContainerState, Health};

use crate::services::Services;

/// The exited container's tail shown on the Overview tab.
pub const LAST_LINES: u32 = 20;

/// The text an env value shows as until a click reveals it (D79).
pub const MASK: &str = "••••";

/// What a detail was loaded for. A new list row with a different
/// signature (state, last start, restart count, exit code, health) makes
/// the stored detail stale, so the panel asks again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    state: ContainerState,
    started: Option<chrono::DateTime<chrono::Utc>>,
    restart_count: u32,
    exit_code: Option<i32>,
    health: Option<Health>,
}

impl Signature {
    #[must_use]
    pub fn of(c: &Container) -> Self {
        Self {
            state: c.state,
            started: c.started,
            restart_count: c.restart_count,
            exit_code: c.exit_code,
            health: c.health,
        }
    }
}

/// A detail and, for an exited container, the end of its log.
#[derive(Clone, PartialEq)]
pub struct Loaded {
    pub detail: ContainerDetail,
    /// The inspect JSON with every `Config.Env` value masked.
    pub inspect: serde_json::Value,
    /// The last [`LAST_LINES`] lines; empty unless the container is
    /// exited or dead.
    pub last_lines: Vec<String>,
}

// Not derived: `EnvVar`'s derived `Debug` would print values.
impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loaded")
            .field("container", &self.detail.container.name)
            .field("env_vars", &self.detail.env.len())
            .field("last_lines", &self.last_lines.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DetailEntry {
    Ready(Box<Loaded>),
    /// The engine said no (container gone, engine unreachable).
    Failed {
        message: String,
        gone: bool,
    },
}

/// Replace every `Config.Env` entry's value with [`MASK`]. The engine
/// mapper already does this; the inspector does it again on the way in so
/// the tree can never show a value whatever client produced the detail.
#[must_use]
pub fn mask_inspect(mut inspect: serde_json::Value) -> serde_json::Value {
    if let Some(env) = inspect
        .pointer_mut("/Config/Env")
        .and_then(serde_json::Value::as_array_mut)
    {
        for entry in env {
            let masked = entry.as_str().map(|s| {
                let key = s.split_once('=').map_or(s, |(key, _)| key);
                format!("{key}={MASK}")
            });
            if let Some(masked) = masked {
                *entry = serde_json::Value::String(masked);
            }
        }
    }
    inspect
}

fn wants_lines(state: ContainerState) -> bool {
    matches!(state, ContainerState::Exited | ContainerState::Dead)
}

/// Inspect one container, and read its last lines when it is exited.
///
/// # Errors
/// The engine's message, and whether the container no longer exists.
pub async fn load(
    client: &dyn berth_engine::EngineClient,
    container: &Container,
    cancel: CancellationToken,
) -> Result<Loaded, (String, bool)> {
    let detail = match client.container(&container.id).await {
        Ok(detail) => detail,
        Err(err) => {
            let gone = matches!(err, berth_engine::EngineError::NotFound(_));
            return Err((err.to_string(), gone));
        }
    };
    let last_lines = if wants_lines(detail.container.state) {
        tail(client, &container.id, cancel).await
    } else {
        Vec::new()
    };
    let inspect = mask_inspect(detail.inspect.clone());
    Ok(Loaded {
        detail,
        inspect,
        last_lines,
    })
}

async fn tail(
    client: &dyn berth_engine::EngineClient,
    id: &ContainerId,
    cancel: CancellationToken,
) -> Vec<String> {
    let query = berth_core::LogQuery {
        since: None,
        tail: Some(LAST_LINES),
        follow: false,
    };
    let mut stream = client.logs(id, query, cancel);
    let read = async {
        let mut lines = Vec::new();
        while let Some(Ok(line)) = stream.next().await {
            lines.push(line.text);
        }
        lines
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), read)
        .await
        .unwrap_or_default()
}

/// The details the inspector has, by container. Provided as context from
/// the App root (and by a scene root); `try_consume_context` it.
#[derive(Clone, Copy)]
pub struct DetailStore {
    pub entries: Signal<HashMap<ContainerId, DetailEntry>>,
    requested: Signal<HashMap<ContainerId, Signature>>,
}

impl DetailStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Signal::new(HashMap::new()),
            requested: Signal::new(HashMap::new()),
        }
    }

    /// Put a loaded detail in as if it had been requested for `container`
    /// (scenes and tests; no engine involved).
    pub fn seed(&self, container: &Container, loaded: Loaded) {
        let mut requested = self.requested;
        let mut entries = self.entries;
        requested
            .write()
            .insert(container.id.clone(), Signature::of(container));
        entries
            .write()
            .insert(container.id.clone(), DetailEntry::Ready(Box::new(loaded)));
    }

    /// Forget that `id`'s detail was asked for, so the next [`ensure`]
    /// reads it again (after a change the list row does not show, such as
    /// new limits). The stored detail stays until the new one replaces it.
    ///
    /// [`ensure`]: DetailStore::ensure
    pub fn invalidate(&self, id: &ContainerId) {
        let mut requested = self.requested;
        requested.write().remove(id);
    }

    /// Ask for `container`'s detail unless one was already asked for its
    /// current signature. Safe to call on every render.
    pub fn ensure(&self, services: &Services, container: &Container) {
        let signature = Signature::of(container);
        let mut requested = self.requested;
        if requested.peek().get(&container.id) == Some(&signature) {
            return;
        }
        requested
            .write()
            .insert(container.id.clone(), signature.clone());
        let mut entries = self.entries;
        let Some(client) = services.client(&container.engine) else {
            entries.write().insert(
                container.id.clone(),
                DetailEntry::Failed {
                    message: format!("Engine {} is not connected", container.engine.0),
                    gone: false,
                },
            );
            return;
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        let cancel = services.cancel.child_token();
        let target = container.clone();
        crate::services::actions::spawn_io(async move {
            let result = load(client.as_ref(), &target, cancel).await;
            let _ = tx.send(result);
        });
        let id = container.id.clone();
        spawn(async move {
            let Ok(result) = rx.await else { return };
            // A newer request for this container replaced this one.
            if requested.peek().get(&id) != Some(&signature) {
                return;
            }
            let entry = match result {
                Ok(loaded) => DetailEntry::Ready(Box::new(loaded)),
                Err((message, gone)) => DetailEntry::Failed { message, gone },
            };
            entries.write().insert(id, entry);
        });
    }
}

impl Default for DetailStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Provide one [`DetailStore`] from the App root (or a scene root).
pub fn provide_detail_store() -> DetailStore {
    use_hook(|| provide_context(DetailStore::new()))
}

#[cfg(test)]
#[path = "detail_tests.rs"]
mod tests;
