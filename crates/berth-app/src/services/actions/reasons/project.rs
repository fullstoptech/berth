//! Why an action cannot run on a project right now.

use super::{engine_unavailable, StateView};

pub(super) fn reason(
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
