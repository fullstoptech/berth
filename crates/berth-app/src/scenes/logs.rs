//! Scenes of the logs screens.

use super::*;

/// "Logs / errors / 1440" and "Logs / paused / 1440" (P3-T2): dark, the five `shop-api`
/// containers followed, ECONNREFUSED searched. The scene gives the five containers ids that sort in
/// picker order (series colours 1..5 as the frames draw them) and a ready log session in place of
/// live streams.
fn logs_scene(state: &mut AppState, paused: bool) {
    use crate::screens::logs::ScenePrefs;
    use crate::screens::logs_scene as data;
    use crate::services::logs::{LogCore, SeededLogs};

    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Dark;
    state.section.set(Section::Logs);
    let mut ids = Vec::new();
    let mut renames = Vec::new();
    {
        let mut containers = state.containers.write();
        for (index, name) in data::NAMES.iter().enumerate() {
            if let Some(container) = containers.iter_mut().find(|c| c.name == *name) {
                let old = container.id.clone();
                container.id = data::scene_id(index, &old.0);
                renames.push((old, container.id.clone()));
                ids.push(container.id.clone());
            }
        }
    }
    // The projects list its service containers by id: follow the rename.
    for project in state.projects.write().iter_mut() {
        for service in &mut project.services {
            for id in &mut service.containers {
                if let Some((_, new)) = renames.iter().find(|(old, _)| old == id) {
                    *id = new.clone();
                }
            }
        }
    }
    state.selection.set(Selection::Containers(ids.clone()));
    let containers = state.containers.peek().clone();
    state.problems.set(mock_problems(&containers));
    let names: Vec<String> = data::NAMES.iter().map(|n| (*n).to_string()).collect();
    let scene = data::scene_view(&ids, &names, paused);
    let core = LogCore::seeded(scene.view);
    if paused {
        core.seed_new_lines(142);
    }
    state.harnesses.set(vec![berth_core::Harness {
        id: berth_core::HarnessId("claude".to_string()),
        name: "Claude Code".to_string(),
        binary: std::path::PathBuf::from("claude"),
        version: None,
        transport: berth_core::HarnessTransport::ClaudeStreamJson,
        available: true,
    }]);
    provide_context(SeededLogs(core));
    provide_context(ScenePrefs(scene.prefs));
}

fn logs_errors(state: &mut AppState) {
    logs_scene(state, false);
}

fn logs_paused(state: &mut AppState) {
    logs_scene(state, true);
}

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Logs / errors / 1440", logs_errors),
    Scene::design("Logs / paused / 1440", logs_paused),
];
