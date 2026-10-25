//! Group actions on containers (P2-T7, P2-T6): which members of a group a
//! bulk Start, Stop or Restart applies to, and running one container
//! action over several of them through the `ActionsService`. Shared by the
//! Containers screen's group header and the Projects screen's Unassigned
//! row, so both agree on who qualifies.

use dioxus::prelude::ReadableExt;

use berth_core::{Container, ContainerAction, ContainerId, ContainerState};

use super::actions::{ActionsService, ContainerTarget};
use crate::state::AppState;

/// Running, restarting or paused: what "Stop all" and "Restart" act on.
#[must_use]
pub fn is_active(state: ContainerState) -> bool {
    matches!(
        state,
        ContainerState::Running | ContainerState::Restarting | ContainerState::Paused
    )
}

/// Exited, created or dead: what "Start" acts on.
#[must_use]
pub fn is_stopped(state: ContainerState) -> bool {
    matches!(
        state,
        ContainerState::Exited | ContainerState::Created | ContainerState::Dead
    )
}

/// The members of a group each bulk action applies to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupIds {
    /// Stopped members ("Start N stopped" starts these).
    pub start_ids: Vec<ContainerId>,
    /// Running, restarting or paused members ("Stop all", restart).
    pub stop_ids: Vec<ContainerId>,
}

impl GroupIds {
    pub fn of<'a>(members: impl IntoIterator<Item = &'a Container>) -> Self {
        let mut ids = Self::default();
        for c in members {
            if is_stopped(c.state) {
                ids.start_ids.push(c.id.clone());
            }
            if is_active(c.state) {
                ids.stop_ids.push(c.id.clone());
            }
        }
        ids
    }
}

/// Run `action` on each container in `ids` that the state still holds, one
/// pending key per container. A refusal is logged, never fatal: the
/// containers that qualify still run.
pub fn run(
    state: &mut AppState,
    actions: Option<&ActionsService>,
    ids: &[ContainerId],
    action: &ContainerAction,
) {
    let Some(service) = actions else { return };
    let targets: Vec<ContainerTarget> = {
        let containers = state.containers.peek();
        ids.iter()
            .filter_map(|id| containers.iter().find(|c| c.id == *id))
            .map(ContainerTarget::from)
            .collect()
    };
    for result in service.containers(state, &targets, action) {
        if let Err(reason) = result {
            tracing::debug!(%reason, "group action refused");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_state(name: &str, state: ContainerState) -> Container {
        let mut c = berth_engine::MockWorld::bundled()
            .containers
            .into_iter()
            .next()
            .expect("a mock container");
        c.id = ContainerId(name.to_string());
        c.state = state;
        c
    }

    #[test]
    fn group_ids_split_members_by_what_each_bulk_action_applies_to() {
        let members = [
            with_state("running", ContainerState::Running),
            with_state("restarting", ContainerState::Restarting),
            with_state("paused", ContainerState::Paused),
            with_state("exited", ContainerState::Exited),
            with_state("created", ContainerState::Created),
            with_state("dead", ContainerState::Dead),
            with_state("removing", ContainerState::Removing),
        ];
        let ids = GroupIds::of(&members);
        let names = |ids: &[ContainerId]| ids.iter().map(|i| i.0.clone()).collect::<Vec<_>>();
        assert_eq!(names(&ids.stop_ids), ["running", "restarting", "paused"]);
        assert_eq!(names(&ids.start_ids), ["exited", "created", "dead"]);
    }
}
