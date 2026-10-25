//! Network confirms: delete a network.

use super::*;

/// What deleting a network does. Only an unused network can be deleted
/// (`disabled_reason`), so nothing running is touched; a Compose project's
/// network comes back with its next start.
#[must_use]
pub fn network_delete_effect(network: &Network, project: Option<&str>) -> EffectSummary {
    let what = match &network.subnet {
        Some(subnet) => format!("Network {} ({subnet}) is removed.", network.name),
        None => format!("Network {} is removed.", network.name),
    };
    let mut lines = vec![
        line(EffectKind::Goes, what),
        line(
            EffectKind::Stays,
            "No container is connected, so nothing running is affected.",
        ),
    ];
    if let Some(project) = project {
        lines.push(line(
            EffectKind::Changes,
            format!("Project {project} creates it again the next time it starts."),
        ));
    }
    ResourceRef::Network(network.id.clone()).delete_effect(lines)
}

/// The delete dialog for one network. `None` when the network is gone or
/// built in.
#[must_use]
pub fn network_delete_model(state: &AppState, id: &str) -> Option<ConfirmModel> {
    let networks = state.networks.peek();
    // A built-in network has no delete dialog: it has no Delete.
    let network = networks.iter().find(|n| n.id == id && !n.built_in)?;
    let containers = state.containers.peek();
    let engine = network.engine.clone();
    let project =
        crate::services::networks::project_of(network, &containers, &state.projects.peek());
    Some(ConfirmModel {
        title: format!("Delete network {}?", network.name),
        effect: network_delete_effect(network, project.as_deref()),
        action: EngineAction::Remove {
            engine,
            target: ResourceRef::Network(network.id.clone()),
            force: false,
        },
        checkbox: None,
        targets: vec![Subject::Network(network.id.clone())],
    })
}

/// Open the delete dialog for a network. Returns false when there is none
/// to delete.
pub fn open_delete_network(state: &mut AppState, id: &str) -> bool {
    let Some(model) = network_delete_model(state, id) else {
        return false;
    };
    state
        .overlays
        .write()
        .push(crate::state::Overlay::Confirm(model));
    true
}
