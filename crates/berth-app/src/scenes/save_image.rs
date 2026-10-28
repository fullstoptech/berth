//! Scenes of the Save as image bar (P8-T4). The design has no frame for it; these are dev scenes
//! over the mock world: the inspector of the loose `pg-scratch` container with the bar open under
//! the header, in the four states a click can reach.

use super::*;

use crate::sheets::save_image::DevSaveImage;
use crate::state::{PendingKey, PendingState};

const REFERENCE: &str = "berthfx/pg-snap:1";

fn open(state: &mut AppState, dev: DevSaveImage) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Dark;
    state.section.set(Section::Containers);
    let Some(target) = state
        .containers
        .peek()
        .iter()
        .find(|c| c.name == "pg-scratch")
        .cloned()
    else {
        return;
    };
    state
        .selection
        .set(Selection::Containers(vec![target.id.clone()]));
    provide_context(dev);
}

/// The field with the name the bar suggests.
fn form(state: &mut AppState) {
    open(state, DevSaveImage::default());
}

/// A name Docker would refuse: the field says why and the button waits.
fn invalid(state: &mut AppState) {
    open(
        state,
        DevSaveImage {
            typed: Some("My Snapshot:1".to_string()),
            ..DevSaveImage::default()
        },
    );
}

/// The commit is running: the button spins and Cancel stops waiting.
fn running(state: &mut AppState) {
    open(
        state,
        DevSaveImage {
            typed: Some(REFERENCE.to_string()),
            running: Some(REFERENCE.to_string()),
            ..DevSaveImage::default()
        },
    );
    state.pending.write().insert(
        PendingKey {
            subject: berth_core::Subject::Image(REFERENCE.to_string()),
            action: crate::services::images::COMMIT_ACTION,
        },
        PendingState::Running {
            label: "Saving pg-scratch as berthfx/pg-snap:1".to_string(),
        },
    );
}

/// The engine made the image: the bar says so and offers Images.
fn saved(state: &mut AppState) {
    open(
        state,
        DevSaveImage {
            saved: Some(REFERENCE.to_string()),
            ..DevSaveImage::default()
        },
    );
}

/// The engine refused: its reason sits under the field.
fn failed(state: &mut AppState) {
    open(
        state,
        DevSaveImage {
            typed: Some(REFERENCE.to_string()),
            failed: Some("No space left on device".to_string()),
            ..DevSaveImage::default()
        },
    );
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Containers / save as image / 1440", form),
    Scene::dev("Containers / save as image / failed / 1440", failed),
    Scene::dev("Containers / save as image / invalid / 1440", invalid),
    Scene::dev("Containers / save as image / running / 1440", running),
    Scene::dev("Containers / save as image / saved / 1440", saved),
];
