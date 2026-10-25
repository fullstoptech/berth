//! Frame scenes for screenshots and SSR tests (D58, ADR 0002).
//! Every UI task adds the scenes for its frames here.

use dioxus::prelude::*;

use crate::app::Shell;
use crate::state::{use_app_state, AppState, EngineScope, Section, Selection};

pub struct Scene {
    pub frame: &'static str,
    pub setup: fn(&mut AppState),
}

// --- Projects frames (P2-T6) ---

/// The mock world with this Mac's home standing in for the mock's
/// "/Users/berth", so `~/...` paths read the same on any machine; the
/// three problems of the frames, aged as the frames show (2, 2 and 18 min).
fn load_projects_world(state: &mut AppState, theme: berth_store::Theme) {
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

fn components_primitives(state: &mut AppState) {
    state.dev_gallery.set(true);
}

/// The bundled mock world as the live app holds it under `BERTH_MOCK=1`:
/// every engine, container, project and resource list, scoped to the
/// default Colima profile (D81).
fn load_mock_world(state: &mut AppState) {
    let world = crate::services::mock_world_at(chrono::Utc::now());
    state.engines.set(world.engines.clone());
    state
        .engine_scope
        .set(EngineScope::One(berth_core::EngineId(
            "colima:default".to_string(),
        )));
    state.containers.set(world.containers.clone());
    state.projects.set(world.projects.clone());
    state
        .images
        .set(world.images_by_engine.values().flatten().cloned().collect());
    state.volumes.set(
        world
            .volumes_by_engine
            .values()
            .flatten()
            .cloned()
            .collect(),
    );
    state.networks.set(
        world
            .networks_by_engine
            .values()
            .flatten()
            .cloned()
            .collect(),
    );
}

/// The three problems the frame's title bar and status bar count.
fn mock_problems(containers: &[berth_core::Container]) -> Vec<berth_core::Problem> {
    use berth_core::{Problem, ProblemId, ProblemKind, Severity, Subject};
    let id_of = |name: &str| {
        containers.iter().find(|c| c.name == name).map_or_else(
            || berth_core::ContainerId(name.to_string()),
            |c| c.id.clone(),
        )
    };
    let first_seen = chrono::DateTime::from_timestamp(1_792_000_000, 0).unwrap_or_default();
    let rows = [
        (
            ProblemKind::DependencyFailed {
                service: "db".to_string(),
            },
            Severity::Error,
            id_of("shop-api-worker-1"),
            "worker keeps restarting",
        ),
        (
            ProblemKind::PortAllocated { port: 5432 },
            Severity::Error,
            id_of("shop-api-db-1"),
            "Port 5432 is already in use",
        ),
        (
            ProblemKind::Unhealthy,
            Severity::Warning,
            id_of("analytics-ingest-1"),
            "analytics-ingest-1 is unhealthy",
        ),
    ];
    rows.into_iter()
        .map(|(kind, severity, container, title)| {
            let subject = Subject::Container(container);
            Problem {
                id: ProblemId::new(&kind, &subject),
                kind,
                severity,
                subject,
                title: title.to_string(),
                explanation: String::new(),
                evidence: Vec::new(),
                fixes: Vec::new(),
                first_seen,
            }
        })
        .collect()
}

/// Dark; the restarting worker selected (inspector body is P3-T3).
fn containers_selected(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Dark;
    state.section.set(Section::Containers);
    let containers = state.containers.peek().clone();
    if let Some(worker) = containers.iter().find(|c| c.name == "shop-api-worker-1") {
        state
            .selection
            .set(Selection::Containers(vec![worker.id.clone()]));
    }
    state.problems.set(mock_problems(&containers));
}

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

/// Dark; the worker selected as in "Containers / selected", the palette
/// open on the query "rest".
fn palette_open(state: &mut AppState) {
    containers_selected(state);
    crate::screens::palette::set_initial_query("rest");
    state.overlays.write().push(crate::state::Overlay::Palette);
}

/// Same state as the dark frame, in the light theme.
fn containers_selected_light(state: &mut AppState) {
    containers_selected(state);
    state.settings.write().theme = berth_store::Theme::Light;
}

/// The mock world on the Containers screen in the light theme with the
/// frames' three problems, nothing selected yet.
fn containers_light_world(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Containers);
    let containers = state.containers.peek().clone();
    state.problems.set(mock_problems(&containers));
}

/// Light; the frame's four rows ticked: the api, the restarting worker,
/// redis and mailpit of shop-api (P2-T8).
fn containers_bulk(state: &mut AppState) {
    containers_light_world(state);
    let ids: Vec<berth_core::ContainerId> = [
        "shop-api-api-1",
        "shop-api-worker-1",
        "shop-api-redis-1",
        "shop-api-mailpit-1",
    ]
    .iter()
    .filter_map(|name| {
        state
            .containers
            .peek()
            .iter()
            .find(|c| c.name == *name)
            .map(|c| c.id.clone())
    })
    .collect();
    state.selection.set(Selection::Containers(ids));
}

/// Light; nothing selected, the delete dialog open for the exited
/// `shop-api-db-1` that keeps volume `shop-api_pgdata` (P2-T8).
fn containers_confirm_delete(state: &mut AppState) {
    containers_light_world(state);
    let db = state
        .containers
        .peek()
        .iter()
        .find(|c| c.name == "shop-api-db-1")
        .map(|c| c.id.clone());
    if let Some(id) = db {
        crate::services::confirm::open_delete(state, &[id]);
    }
}

/// The default engine has not answered yet: nothing loaded, no version.
fn containers_loading(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Containers);
    state.containers.set(Vec::new());
    for engine in state.engines.write().iter_mut() {
        if engine.id.0 == "colima:default" {
            engine.docker_version = None;
        }
    }
}

/// The default Colima profile is stopped.
fn containers_engine_stopped(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Containers);
    for engine in state.engines.write().iter_mut() {
        if engine.id.0 == "colima:default" {
            engine.state = berth_core::EngineState::Stopped;
        }
    }
}

const SCENES: &[Scene] = &[
    Scene {
        frame: "Projects / empty / 1440",
        setup: projects_empty,
    },
    Scene {
        frame: "Projects / default / 1440",
        setup: projects_default,
    },
    Scene {
        frame: "Projects / default / 1440 / dark",
        setup: projects_default_dark,
    },
    Scene {
        frame: "Components / primitives",
        setup: components_primitives,
    },
    Scene {
        frame: "Containers / selected / 1440",
        setup: containers_selected,
    },
    Scene {
        frame: "Activity / live / 1440",
        setup: activity_live,
    },
    Scene {
        frame: "Activity / paused / 1440",
        setup: activity_paused,
    },
    Scene {
        frame: "Palette / open / 1440",
        setup: palette_open,
    },
    Scene {
        frame: "Containers / selected / 1440 / light",
        setup: containers_selected_light,
    },
    // Rendered at 1024x700 (BERTH_WINDOW=1024x700 for shots).
    Scene {
        frame: "Containers / selected / 1024",
        setup: containers_selected,
    },
    Scene {
        frame: "Containers / bulk / 1440",
        setup: containers_bulk,
    },
    Scene {
        frame: "Containers / confirm delete / 1440",
        setup: containers_confirm_delete,
    },
    Scene {
        frame: "Containers / loading / 1440",
        setup: containers_loading,
    },
    Scene {
        frame: "Containers / engine stopped / 1440",
        setup: containers_engine_stopped,
    },
];

pub fn scenes() -> &'static [Scene] {
    SCENES
}

pub fn find(frame: &str) -> Option<&'static Scene> {
    SCENES.iter().find(|s| s.frame == frame)
}

#[component]
fn SceneRoot(frame: String) -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    crate::screens::containers::provide_containers_view();
    let setup_frame = frame.clone();
    use_hook(move || {
        if let Some(scene) = find(&setup_frame) {
            (scene.setup)(&mut state);
        }
    });
    rsx! { Shell {} }
}

/// Render a frame's scene to HTML with dioxus-ssr, for tests and scripts/ssr.sh.
pub fn render_scene_html(frame: &str) -> Option<String> {
    find(frame)?;
    let mut dom = VirtualDom::new_with_props(
        SceneRoot,
        SceneRootProps {
            frame: frame.to_string(),
        },
    );
    let _ = dom.rebuild_to_vec();
    Some(dioxus_ssr::render(&dom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_registry_contains_projects_empty() {
        assert!(find("Projects / empty / 1440").is_some());
        assert!(find("Components / primitives").is_some());
        assert!(find("Containers / selected / 1440").is_some());
        assert_eq!(scenes().len(), 14);
        assert!(find("No such frame").is_none());
    }

    #[test]
    fn scenes_render_projects_empty_shell() {
        let html = render_scene_html("Projects / empty / 1440").expect("scene renders");
        assert!(html.contains("titlebar"), "{html}");
        assert!(html.contains("sidebar"));
        assert!(html.contains("Projects"));
        assert!(html.contains("Containers"));
        assert!(html.contains("No problems"));
        assert!(html.contains("/icons.svg#folder"));
    }

    #[test]
    fn scenes_render_primitives_gallery() {
        let html = render_scene_html("Components / primitives").expect("scene renders");
        assert!(html.contains("Components / primitives"), "{html}");
        assert!(html.contains("btn-primary"));
        assert!(html.contains("status-badge"));
        assert!(html.contains("Exited (137)"));
        assert!(html.contains("12 of 48"));
    }

    #[test]
    fn scenes_unknown_frame_renders_nothing() {
        assert!(render_scene_html("Projects / bogus").is_none());
    }
}
