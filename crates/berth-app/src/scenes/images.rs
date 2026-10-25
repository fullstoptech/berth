//! Scenes of the images screens.

use super::*;

// --- Images (P4-T1; no frame draws "On this engine", so these are the
// task's own scenes, read beside "Images / find / 1440" and Containers) ---

const DEFAULT_ENGINE: &str = "colima:default";

fn images_base(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Images);
    let containers = state.containers.peek().clone();
    state.problems.set(mock_problems(&containers));
}

/// The id of the default engine's image with this tag.
fn default_image_id(tag: &str) -> Option<String> {
    let world = crate::services::mock_world_at(chrono::Utc::now());
    world
        .images_by_engine
        .get(DEFAULT_ENGINE)?
        .iter()
        .find(|i| i.repo_tags.iter().any(|t| t == tag))
        .map(|i| i.id.clone())
}

fn images_on_engine(state: &mut AppState) {
    images_base(state);
}

/// `shop-api-worker:latest` is used by two containers, so its Delete
/// shows "Used by 2 containers".
fn images_selected(state: &mut AppState) {
    images_base(state);
    if let Some(id) = default_image_id("shop-api-worker:latest") {
        state.selection.set(Selection::Image(id));
    }
}

/// Delete on an image nothing uses: the confirm dialog (AD25a).
fn images_confirm_delete(state: &mut AppState) {
    images_base(state);
    let world = crate::services::mock_world_at(chrono::Utc::now());
    let engine = berth_core::EngineId(DEFAULT_ENGINE.to_string());
    let Some(image) = world
        .images_by_engine
        .get(DEFAULT_ENGINE)
        .and_then(|images| {
            images
                .iter()
                .find(|i| i.repo_tags.iter().any(|t| t == "postgres:17"))
        })
    else {
        return;
    };
    state.selection.set(Selection::Image(image.id.clone()));
    let model = crate::services::confirm::delete_image_model(&engine, image);
    state
        .overlays
        .write()
        .push(crate::state::Overlay::Confirm(model));
}

fn images_run_sheet(state: &mut AppState) {
    images_base(state);
    if let Some(id) = default_image_id("postgres:17-alpine") {
        state.selection.set(Selection::Image(id.clone()));
        state.overlays.write().push(crate::state::Overlay::Run(id));
    }
}

/// A pull of `alpine:3` part way: three layers, one running, one waiting.
fn images_pulling(state: &mut AppState) {
    use berth_core::PullProgress;
    images_base(state);
    let Some(store) = try_consume_context::<crate::services::images::ImageStore>() else {
        return;
    };
    let event = |layer: Option<&str>, status: &str, current: Option<u64>, total: Option<u64>| {
        PullProgress {
            layer: layer.map(str::to_string),
            status: status.to_string(),
            current,
            total,
        }
    };
    let mut pull = crate::services::images::PullState::new(
        berth_core::EngineId(DEFAULT_ENGINE.to_string()),
        "postgres:17-alpine",
        None,
    );
    for e in [
        event(Some("3"), "Pulling from library/postgres", None, None),
        event(Some("a2318d6c47ec"), "Pulling fs layer", None, None),
        event(Some("1f8c0ab51b3e"), "Pulling fs layer", None, None),
        event(Some("5d2b9e1c7a40"), "Pulling fs layer", None, None),
        event(Some("e90a1c47d6b2"), "Pulling fs layer", None, None),
        event(Some("7c13f08ea9d5"), "Waiting", None, None),
        event(
            Some("a2318d6c47ec"),
            "Downloading",
            Some(3_600_000),
            Some(3_600_000),
        ),
        event(Some("a2318d6c47ec"), "Pull complete", None, None),
        event(
            Some("1f8c0ab51b3e"),
            "Downloading",
            Some(1_900_000),
            Some(1_900_000),
        ),
        event(Some("1f8c0ab51b3e"), "Pull complete", None, None),
        event(
            Some("5d2b9e1c7a40"),
            "Downloading",
            Some(12_800_000),
            Some(12_800_000),
        ),
        event(Some("5d2b9e1c7a40"), "Pull complete", None, None),
        event(
            Some("e90a1c47d6b2"),
            "Downloading",
            Some(31_100_000),
            Some(41_500_000),
        ),
    ] {
        pull.apply(&e);
    }
    let mut pulls = store.pulls;
    pulls.write().push(pull);
    let mut pull_open = store.pull_open;
    pull_open.set(true);
}

/// Hand the Images section what the live service would have fetched:
/// each engine's list, and the detail of the selected image or the Run
/// sheet's image (Scenes run no service, D58; the mock has no I/O).
pub fn seed_images(state: &AppState, store: crate::services::images::ImageStore) {
    let world = crate::services::mock_world_at(chrono::Utc::now());
    for (engine, images) in &world.images_by_engine {
        store.seed(&berth_core::EngineId(engine.clone()), images.clone());
    }
    let mut wanted: Vec<String> = Vec::new();
    if let Selection::Image(id) = &*state.selection.peek() {
        wanted.push(id.clone());
    }
    for overlay in state.overlays.peek().iter() {
        if let crate::state::Overlay::Run(id) = overlay {
            wanted.push(id.clone());
        }
    }
    let engine = berth_core::EngineId(DEFAULT_ENGINE.to_string());
    for id in wanted {
        let Ok(mock) = berth_engine::MockEngine::from_world(world.clone(), &engine) else {
            continue;
        };
        let detail = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_time()
                        .build()
                        .ok()?;
                    runtime
                        .block_on(berth_engine::EngineClient::image(&mock, &id))
                        .ok()
                })
                .join()
                .ok()
                .flatten()
        });
        if let Some(detail) = detail {
            store.seed_detail(&engine, detail);
        }
    }
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Images / on this engine / 1440", images_on_engine),
    Scene::dev("Images / selected / 1440", images_selected),
    Scene::dev("Images / pulling / 1440", images_pulling),
    Scene::dev("Images / run / 1440", images_run_sheet),
    Scene::dev("Images / confirm delete / 1440", images_confirm_delete),
];
