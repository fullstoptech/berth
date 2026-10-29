//! Scenes of the networks screens.

use super::*;

/// The mock world on the Networks screen (P4-T5, no frame). `select` names
/// the network whose detail the inspector shows.
fn networks_world(state: &mut AppState, theme: berth_store::Theme, select: Option<&str>) {
    load_mock_world(state);
    state.settings.write().theme = theme;
    state.section.set(Section::Networks);
    if let Some(name) = select {
        let id = state
            .networks
            .peek()
            .iter()
            .find(|n| n.name == name)
            .map(|n| n.id.clone());
        if let Some(id) = id {
            state.selection.set(Selection::Network(id));
        }
    }
}

fn networks_default(state: &mut AppState) {
    networks_world(state, berth_store::Theme::Light, None);
}

fn networks_selected(state: &mut AppState) {
    networks_world(state, berth_store::Theme::Dark, Some("shop-api_default"));
}

/// The create sheet with a subnet that has host bits set, so the inline
/// error shows under the field and Create is disabled with its reason.
fn networks_create(state: &mut AppState) {
    networks_world(state, berth_store::Theme::Light, None);
    state
        .overlays
        .write()
        .push(crate::state::Overlay::CreateNetwork);
    provide_context(crate::sheets::create_network::DevNetworkDraft(|draft| {
        draft.name = "berthfx-net".to_string();
        draft.subnet = "172.31.250.5/24".to_string();
        draft.gateway = "172.31.250.1".to_string();
    }));
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Networks / default / 1440", networks_default),
    Scene::dev("Networks / selected / 1440", networks_selected),
    Scene::dev("Networks / selected / 1024", networks_selected),
    Scene::dev("Networks / create / 1440", networks_create),
];
