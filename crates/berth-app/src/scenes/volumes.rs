//! Scenes of the volumes screens.

use super::*;

// --- Volumes (P4-T4): no frame, the nearest is "Containers / selected / 1440" ---

/// The mock world on the Volumes screen, dark like the Containers frame.
fn volumes_world(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Dark;
    state.section.set(Section::Volumes);
}

fn volumes_list(state: &mut AppState) {
    volumes_world(state);
}

/// A volume in use selected: its Overview in the inspector.
fn volumes_selected(state: &mut AppState) {
    volumes_world(state);
    state
        .selection
        .set(Selection::Volume("shop-api_pgdata".to_string()));
}

/// The Files tab of that volume over an in-memory Postgres data folder,
/// with the helper image already on the engine.
fn volumes_browser(state: &mut AppState) {
    use crate::services::files::{FilesBackend, MemoryFiles};
    volumes_selected(state);
    let now = chrono::Utc::now().timestamp();
    let day = 86_400;
    let files = MemoryFiles::new()
        .with_dir("/base", now - 2 * day)
        .with_dir("/global", now - 2 * day)
        .with_dir("/pg_wal", now - 600)
        .with_dir("/pg_stat", now - 600)
        .with_file("/PG_VERSION", b"17\n", now - 9 * day)
        .with_file("/postgresql.conf", &[b'x'; 30_912], now - 9 * day)
        .with_file("/pg_hba.conf", &[b'x'; 5_711], now - 9 * day)
        .with_file("/postmaster.pid", &[b'x'; 98], now - 7_200)
        .with_file("/postmaster.opts", &[b'x'; 40], now - 7_200);
    provide_context(FilesBackend(std::sync::Arc::new(files)));
    provide_context(crate::screens::volume_browser::HelperPreset(
        crate::screens::volume_browser::Helper::Ready,
    ));
    if let Some(view) = try_consume_context::<crate::screens::volumes::VolumesView>() {
        let mut browse = view.browse;
        browse.set(Some("shop-api_pgdata".to_string()));
    }
}

/// The Files tab before the helper image is on the engine.
fn volumes_helper(state: &mut AppState) {
    volumes_browser(state);
    provide_context(crate::screens::volume_browser::HelperPreset(
        crate::screens::volume_browser::Helper::Missing,
    ));
}

fn volumes_create(state: &mut AppState) {
    volumes_world(state);
    state
        .overlays
        .write()
        .push(crate::state::Overlay::CreateVolume);
}

/// The typed-name delete dialog of an unused volume.
fn volumes_confirm_delete(state: &mut AppState) {
    volumes_world(state);
    state
        .selection
        .set(Selection::Volume("stale-cache".to_string()));
    crate::services::confirm::open_delete_volume(state, "stale-cache");
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Volumes / list / 1440", volumes_list),
    Scene::dev("Volumes / selected / 1440", volumes_selected),
    Scene::dev("Volumes / browser / 1440", volumes_browser),
    Scene::dev("Volumes / helper / 1440", volumes_helper),
    Scene::dev("Volumes / create / 1440", volumes_create),
    Scene::dev("Volumes / confirm delete / 1440", volumes_confirm_delete),
];
