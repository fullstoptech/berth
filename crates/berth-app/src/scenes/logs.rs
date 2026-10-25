//! Scenes of the logs screens.

use super::*;

/// "Logs / errors / 1440" and "Logs / paused / 1440" (P3-T2): dark, the five `shop-api`
/// containers followed, ECONNREFUSED searched. The scene gives the five containers ids that sort in
/// picker order (series colours 1..5 as the frames draw them) and a ready log session in place of
/// live streams.
fn logs_scene(state: &mut AppState, paused: bool, explained: bool) {
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
    let mut prefs = scene.prefs;
    if explained {
        prefs.explained = Some(Some(vec![explained_problem(state)]));
    }
    provide_context(ScenePrefs(prefs));
}

/// What Explain says about the two selected ECONNREFUSED lines of the worker: a dependency that
/// is not answering, with the restart the rules offer (a dev scene; the frames do not draw it).
fn explained_problem(state: &AppState) -> berth_core::Problem {
    use berth_core::{
        ContainerAction, EffectKind, EffectLine, EffectSummary, EngineAction, Fix, FixEffect,
        FixSource, HarnessId, Problem, ProblemId, ProblemKind, Severity, Subject,
    };
    let containers = state.containers.peek();
    let find = |name: &str| containers.iter().find(|c| c.name == name).cloned();
    let worker = find("shop-api-worker-1");
    let db = find("shop-api-db-1");
    let kind = ProblemKind::DependencyFailed {
        service: "db".to_string(),
    };
    let subject = Subject::Container(worker.map_or_else(
        || berth_core::ContainerId("worker".to_string()),
        |c| c.id,
    ));
    let restart = db.map(|db| Fix {
        label: format!("Restart {}", db.name),
        source: FixSource::Rule,
        effect: FixEffect::RunAction {
            action: EngineAction::Container {
                engine: db.engine.clone(),
                id: db.id.clone(),
                action: ContainerAction::Restart { timeout_s: None },
            },
            effect: EffectSummary {
                verb: "Restart".to_string(),
                noun_one: "container".to_string(),
                noun_many: "containers".to_string(),
                count: 1,
                bytes: None,
                lines: vec![EffectLine {
                    kind: EffectKind::Changes,
                    text: format!("{} stops and starts again", db.name),
                }],
                typed_name: None,
            },
        },
    });
    let mut fixes: Vec<Fix> = restart.into_iter().collect();
    fixes.push(Fix {
        label: "Fix with Claude Code".to_string(),
        source: FixSource::Harness(HarnessId("claude".to_string())),
        effect: FixEffect::Explain,
    });
    Problem {
        id: ProblemId::new(&kind, &subject),
        kind,
        severity: Severity::Error,
        subject,
        title: "shop-api-worker-1 cannot reach db".to_string(),
        explanation: "The worker tried to connect to db at 172.19.0.3:5432 and nothing answered. db exited with code 1, so the connection is refused.".to_string(),
        evidence: vec!["Error: connect ECONNREFUSED 172.19.0.3:5432".to_string()],
        fixes,
        first_seen: chrono::Utc::now(),
    }
}

fn logs_errors(state: &mut AppState) {
    logs_scene(state, false, false);
}

fn logs_paused(state: &mut AppState) {
    logs_scene(state, true, false);
}

fn logs_explained(state: &mut AppState) {
    logs_scene(state, false, true);
}

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Logs / errors / 1440", logs_errors),
    Scene::design("Logs / paused / 1440", logs_paused),
    Scene::dev("Logs / explained / 1440", logs_explained),
];
