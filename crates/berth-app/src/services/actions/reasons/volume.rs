//! Why an action cannot run on a volume right now.

use super::{engine_unavailable, plural, StateView};

pub(super) fn reason(name: &str, action: &str, state: &StateView<'_>) -> Option<String> {
    if action != "volume.delete" {
        return None;
    }
    let Some(volume) = state.volumes.iter().find(|v| v.name == name) else {
        return Some("This volume no longer exists".to_string());
    };
    if !volume.used_by.is_empty() {
        return Some(format!(
            "Used by {}",
            plural(volume.used_by.len(), "container", "containers")
        ));
    }
    engine_unavailable(&volume.engine, state)
}
