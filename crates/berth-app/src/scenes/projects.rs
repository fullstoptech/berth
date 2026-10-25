//! Scenes of the projects screens.

use super::*;

// --- Projects frames (P2-T6) ---

/// The mock world with this Mac's home standing in for the mock's
/// "/Users/berth", so `~/...` paths read the same on any machine; the
/// three problems of the frames, aged as the frames show (2, 2 and 18 min).
pub(super) fn load_projects_world(state: &mut AppState, theme: berth_store::Theme) {
    load_mock_world(state);
    state.settings.write().theme = theme;
    state.section.set(Section::Projects);
    let home = berth_sys::paths::home();
    let mut projects = state.projects.peek().clone();
    for project in &mut projects {
        if let Some(rest) = project
            .folder
            .as_deref()
            .and_then(|f| f.strip_prefix("/Users/berth").ok())
        {
            project.folder = Some(home.join(rest));
        }
    }
    state.projects.set(projects);
    let containers = state.containers.peek().clone();
    let now = chrono::Utc::now();
    let frame_problems = [
        ("worker keeps restarting", 130),
        ("Port 5432 is already in use", 120),
        ("ingest health check is failing", 1110),
    ];
    let problems = mock_problems(&containers)
        .into_iter()
        .zip(frame_problems)
        .map(|(mut problem, (title, age_s))| {
            problem.title = title.to_string();
            problem.first_seen = now - chrono::Duration::seconds(age_s);
            problem
        })
        .collect();
    state.problems.set(problems);
    crate::screens::recent_activity::seed_recent(crate::screens::recent_activity::mock_entries(
        &berth_engine::MockWorld::bundled(),
        now,
    ));
}

fn projects_default(state: &mut AppState) {
    load_projects_world(state, berth_store::Theme::Light);
}

/// The mock world's "Unassigned" projects, grouped from its containers by
/// `projects::group` as the live path does: one per engine that has
/// containers outside any Compose project. The fixture's own project list
/// leaves them out because the frames do.
pub(crate) fn loose_projects(world: &berth_engine::MockWorld) -> Vec<berth_core::Project> {
    world
        .engines
        .iter()
        .flat_map(|engine| {
            let containers = world.containers_for(&engine.id);
            berth_engine::projects::group(&engine.id, &containers, &[], false)
        })
        .filter(|p| matches!(p.source, berth_core::ProjectSource::Loose))
        .collect()
}

/// "Projects / default / 1440" plus the Unassigned row the live app shows
/// (not in the frame: the owner keeps the row, so it gets its own scene).
fn projects_with_unassigned(state: &mut AppState) {
    load_projects_world(state, berth_store::Theme::Light);
    let world = crate::services::mock_world_at(chrono::Utc::now());
    let mut projects = state.projects.peek().clone();
    // The default profile's bucket only: the other engines' buckets are
    // the same row again, on an engine that is stopped or unreachable.
    projects.extend(
        loose_projects(&world)
            .into_iter()
            .filter(|p| p.engine.0 == "colima:default"),
    );
    state.projects.set(projects);
}

fn projects_default_dark(state: &mut AppState) {
    load_projects_world(state, berth_store::Theme::Dark);
}

/// An engine that answered and knows no project: the mock world's
/// engines and counts, an empty project list, no problems.
fn projects_empty(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Projects);
    state.projects.set(Vec::new());
    state.problems.set(Vec::new());
}

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Projects / empty / 1440", projects_empty),
    Scene::design("Projects / default / 1440", projects_default),
    Scene::design("Projects / default / 1440 / dark", projects_default_dark),
    Scene::dev("Projects / with unassigned / 1440", projects_with_unassigned),
];
