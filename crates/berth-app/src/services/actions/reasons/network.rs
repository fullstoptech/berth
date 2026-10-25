//! Why an action cannot run on a network right now.

use super::{plural, StateView};
use crate::services::networks;

pub(super) fn reason(id: &str, action: &str, state: &StateView<'_>) -> Option<String> {
    if action == "network.create" {
        return state
            .networks
            .iter()
            .any(|n| n.name == id)
            .then(|| format!("A network named {id} already exists"));
    }
    if action != "network.delete" {
        return None;
    }
    let Some(network) = state.networks.iter().find(|n| n.id == id || n.name == id) else {
        return Some("This network no longer exists".to_string());
    };
    if network.built_in {
        return Some(networks::BUILT_IN_REASON.to_string());
    }
    if network.containers.is_empty() {
        None
    } else {
        Some(format!(
            "{} connected",
            plural(network.containers.len(), "container is", "containers are")
        ))
    }
}
