//! Why an action cannot run on a engine right now.

use super::StateView;
use crate::services::colima;

pub(super) fn reason(
    id: &berth_core::EngineId,
    action: &str,
    state: &StateView<'_>,
) -> Option<String> {
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
