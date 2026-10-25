//! Disabled reasons (pure): why an action cannot run on a subject right
//! now. One file per resource; [`disabled_reason`] picks by subject.

use dioxus::prelude::ReadableExt;

use berth_core::Subject;

use crate::state::{AppState, PendingKey, PendingState};

mod container;
mod engine;
mod image;
mod network;
mod project;
mod volume;

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

pub(super) fn plural(n: usize, one: &str, many: &str) -> String {
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
        Subject::Container(id) => container::reason(id, action, state),
        Subject::Project(id) => project::reason(id, action, state),
        Subject::Engine(id) => engine::reason(id, action, state),
        Subject::Image(id) => image::reason(id, action, state),
        Subject::Volume(name) => volume::reason(name, action, state),
        Subject::Network(id) => network::reason(id, action, state),
        Subject::Plugin(_) => None,
    }
}

pub(super) fn engine_unavailable(
    engine: &berth_core::EngineId,
    state: &StateView<'_>,
) -> Option<String> {
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
