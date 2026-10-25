//! Scenes of the palette screens.

use super::*;

/// Dark; nothing selected (so no inspector, and the table keeps its Uptime
/// column), the palette open on the query "rest". The bare verb still picks
/// the container that needs the most attention, the restarting worker.
///
/// The palette searches every engine (D4), and "rest" would also reach
/// `backup-1` and its restic image on hetzner-1. The frame was drawn with
/// one engine's data, so this scene keeps the default engine's records
/// only (the other engines stay listed).
fn palette_open(state: &mut AppState) {
    super::containers::containers_selected(state);
    state.selection.set(Selection::None);
    keep_default_engine_records(state);
    crate::screens::palette::set_initial_query("rest");
    state.overlays.write().push(crate::state::Overlay::Palette);
}

/// Drop the records of every engine but `colima:default` from the lists:
/// its containers, and the images, volumes and networks that
/// are unused or used by one of those containers.
fn keep_default_engine_records(state: &mut AppState) {
    let default = berth_core::EngineId("colima:default".to_string());
    state.containers.write().retain(|c| c.engine == default);
    let mine: Vec<berth_core::ContainerId> = state
        .containers
        .read()
        .iter()
        .map(|c| c.id.clone())
        .collect();
    let used_here = |users: &[berth_core::ContainerId]| {
        users.is_empty() || users.iter().any(|id| mine.contains(id))
    };
    state.images.write().retain(|i| used_here(&i.used_by));
    state.volumes.write().retain(|v| used_here(&v.used_by));
    state.networks.write().retain(|n| used_here(&n.containers));
}

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Palette / open / 1440", palette_open),
];
