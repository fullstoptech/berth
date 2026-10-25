//! Scenes of the activity screens.

use super::*;

/// Activity has no frame; mock history on every engine (P2-T10).
fn activity_scene(state: &mut AppState, paused: bool) {
    let world = berth_engine::MockWorld::bundled();
    load_mock_world(state);
    state.engine_scope.set(EngineScope::All);
    state.section.set(Section::Activity);
    crate::screens::activity_scene::seed(&world, paused);
}

fn activity_live(state: &mut AppState) {
    activity_scene(state, false);
}

fn activity_paused(state: &mut AppState) {
    activity_scene(state, true);
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Activity / live / 1440", activity_live),
    Scene::dev("Activity / paused / 1440", activity_paused),
];
