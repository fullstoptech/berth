//! Scenes of the drop overlay, the Detected sheet and the New project sheet (P5-T5). Two draw
//! frames of the design; the others are the states the frames leave out.

use super::*;

use berth_core::{Confidence, FoundKind};

use crate::services::drops::Dropped;
use crate::sheets::generate::SeedDetection;
use crate::sheets::new_project::{SeedStage, Stage};
use crate::state::Overlay;
use crate::ui::drop_overlay::SeedHover;

/// What the Next.js fixture detects, as "New project / detected / 1440" reads it: six keys in
/// `.env`, Redis found by its package.
fn frame_detection() -> berth_core::Detection {
    let mut d = generate::next_detection();
    for found in &mut d.found {
        if let FoundKind::Env { keys } = &mut found.kind {
            *keys = 6;
        }
    }
    if let Some(redis) = d.services_needed.get_mut(1) {
        redis.reason = "from ioredis in package.json".to_string();
    }
    d.confidence = Confidence::High;
    d
}

fn new_app() -> std::path::PathBuf {
    berth_sys::paths::home().join("Dev/side/new-app")
}

fn drop_folder(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    provide_context(SeedHover(Dropped {
        folders: vec![new_app()],
        rejected: Vec::new(),
    }));
}

fn drop_wrong_type(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    provide_context(SeedHover(Dropped {
        folders: Vec::new(),
        rejected: vec!["notes.txt".to_string()],
    }));
}

fn drop_several(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    let home = berth_sys::paths::home();
    provide_context(SeedHover(Dropped {
        folders: vec![new_app(), home.join("Dev/side/scratch")],
        rejected: vec!["notes.txt".to_string()],
    }));
}

fn open_detected(state: &mut AppState, detection: berth_core::Detection) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    let folder = detection.folder.clone();
    provide_context(SeedDetection(detection));
    state.overlays.write().push(Overlay::Detected(folder));
}

fn detected_frame(state: &mut AppState) {
    state
        .harnesses
        .set(vec![super::review_changes::claude_code()]);
    open_detected(state, frame_detection());
}

fn detected_compose(state: &mut AppState) {
    let mut d = frame_detection();
    d.folder = berth_sys::paths::home().join("Dev/side/shop-api");
    d.found.push(berth_core::FoundFile {
        kind: FoundKind::Compose,
        path: std::path::PathBuf::from("compose.yaml"),
    });
    open_detected(state, d);
}

fn new_project_sheet(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    state.overlays.write().push(Overlay::NewProject);
}

fn new_project_editor(state: &mut AppState) {
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    provide_context(SeedStage(Stage::Editor));
    state.overlays.write().push(Overlay::NewProject);
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Detected / compose present / 1440", detected_compose),
    Scene::dev("New project / editor / 1440", new_project_editor),
    Scene::design("New project / detected / 1440", detected_frame),
    Scene::dev("New project / sheet / 1440", new_project_sheet),
    Scene::design("Projects / drop folder / 1440", drop_folder),
    Scene::dev("Projects / drop folder / several / 1440", drop_several),
    Scene::dev("Projects / drop folder / wrong type / 1440", drop_wrong_type),
];
