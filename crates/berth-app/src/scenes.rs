//! Frame scenes for screenshots and SSR tests (D58, ADR 0002).
//! Every UI task adds the scenes for its frames here.

use dioxus::prelude::*;

use crate::app::Shell;
use crate::state::{use_app_state, AppState, EngineScope, Section, Selection};

pub struct Scene {
    pub frame: &'static str,
    pub setup: fn(&mut AppState),
}

fn projects_empty(state: &mut AppState) {
    state.section.set(Section::Projects);
    state.projects.set(Vec::new());
}

fn components_primitives(state: &mut AppState) {
    state.dev_gallery.set(true);
}

/// The bundled mock world as the live app holds it under `BERTH_MOCK=1`:
/// every engine, container, project and resource list, scoped to the
/// default Colima profile (D81).
fn load_mock_world(state: &mut AppState) {
    let world = berth_engine::MockWorld::bundled();
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

/// Dark; the restarting worker selected; chrome only (table P2-T7,
/// inspector body P3-T3).
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

const SCENES: &[Scene] = &[
    Scene {
        frame: "Projects / empty / 1440",
        setup: projects_empty,
    },
    Scene {
        frame: "Components / primitives",
        setup: components_primitives,
    },
    Scene {
        frame: "Containers / selected / 1440",
        setup: containers_selected,
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
        assert_eq!(scenes().len(), 3);
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
