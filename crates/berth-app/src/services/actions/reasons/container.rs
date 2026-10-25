//! Why an action cannot run on a container right now.

use super::{engine_unavailable, StateView};

pub(super) fn reason(
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
