//! Frame scenes for screenshots and SSR tests (D58, ADR 0002).
//!
//! Each area declares its own scenes in `scenes/<area>.rs` as
//! `pub(super) const SCENES: &[Scene]`. A new area adds its file and one name
//! to the `scene_modules!` list below; nothing else in this file changes.

use dioxus::prelude::*;

use crate::app::Shell;
use crate::state::{use_app_state, AppState, EngineScope, Section, Selection};

/// One frame's state: `frame` is the name `BERTH_SCENE` and
/// `scripts/ssr.sh` look it up by, `setup` puts the app state into it.
/// `design` marks a scene that draws a frame of `design/berth.figcraft`; its
/// PNG must exist in `design/shots/figcraft/screens/`.
#[derive(Clone, Copy)]
pub struct Scene {
    pub frame: &'static str,
    pub setup: fn(&mut AppState),
    pub design: bool,
}

impl Scene {
    /// A scene that draws the figcraft frame of the same name.
    pub const fn design(frame: &'static str, setup: fn(&mut AppState)) -> Self {
        Self {
            frame,
            setup,
            design: true,
        }
    }

    /// A scene with no frame of its own (a dev or task scene).
    pub const fn dev(frame: &'static str, setup: fn(&mut AppState)) -> Self {
        Self {
            frame,
            setup,
            design: false,
        }
    }
}

/// Declares the area modules and gathers their `SCENES`. One line per area.
macro_rules! scene_modules {
    ($($area:ident),* $(,)?) => {
        $(mod $area;)*

        fn registry() -> Vec<Scene> {
            let mut all = Vec::new();
            $(all.extend_from_slice($area::SCENES);)*
            all
        }
    };
}

scene_modules! {
    activity,
    add_service,
    clean_up,
    containers,
    detected,
    dev,
    engines,
    find_images,
    generate,
    images,
    logs,
    networks,
    open_folder,
    palette,
    project_detail,
    projects,
    review_changes,
    templates,
    terminal,
    volumes,
}

#[cfg(test)]
pub(crate) use add_service::SCENE_PASSWORD;
pub(crate) use dev::{DevFilesPanel, DevStatsPanel};
#[cfg(test)]
pub(crate) use generate::next_detection;
#[cfg(test)]
pub(crate) use projects::loose_projects;
#[cfg(test)]
pub(crate) use review_changes::{new_app_plan as new_app_pending, NEW_APP_PASSWORD};

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

/// Fill the stores a scene's screens read from, after its setup ran.
pub(crate) fn seed_stores(state: &AppState, views: &crate::providers::ScreenViews) {
    containers::seed_details(state, views.details);
    images::seed_images(state, views.images);
}

pub fn scenes() -> &'static [Scene] {
    static ALL: std::sync::OnceLock<Vec<Scene>> = std::sync::OnceLock::new();
    ALL.get_or_init(registry)
}

pub fn find(frame: &str) -> Option<&'static Scene> {
    scenes().iter().find(|s| s.frame == frame)
}

/// The window width a frame is drawn at: the number its name ends with
/// ("Containers / selected / 1024"), when it is a window width.
fn frame_width(frame: &str) -> Option<f64> {
    let last = frame.rsplit(" / ").next()?;
    let width: f64 = last.trim().parse().ok()?;
    (900.0..=3000.0).contains(&width).then_some(width)
}

#[component]
fn SceneRoot(frame: String) -> Element {
    let mut state = use_app_state();
    use_context_provider(|| state);
    let views = crate::providers::provide_screen_views(state);
    crate::services::exec::provide_scene_terminals();
    let setup_frame = frame.clone();
    use_hook(move || {
        // A frame is drawn at the width its name ends with.
        if let Some(width) = frame_width(&setup_frame) {
            let mut window_w = views.dock.window_w;
            window_w.set(width);
        }
        if let Some(scene) = find(&setup_frame) {
            (scene.setup)(&mut state);
            seed_stores(&state, &views);
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
    use super::dev::synthetic_sample;
    use super::*;

    #[test]
    fn scenes_registry_contains_projects_empty() {
        assert!(find("Projects / empty / 1440").is_some());
        assert!(find("Components / primitives").is_some());
        assert!(find("Containers / selected / 1440").is_some());
        assert!(find("No such frame").is_none());
    }

    #[test]
    fn scenes_no_two_scenes_share_a_frame_name() {
        let mut seen = std::collections::BTreeSet::new();
        for scene in scenes() {
            assert!(
                seen.insert(scene.frame),
                "frame {:?} is declared twice",
                scene.frame
            );
        }
        assert!(!seen.is_empty());
    }

    /// The exported figcraft frame of a scene: `design/shots/figcraft/screens/`
    /// names a PNG after the frame with every `/` written as `_`.
    fn design_png(frame: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../design/shots/figcraft/screens")
            .join(format!("{}.png", frame.replace('/', "_")))
    }

    #[test]
    fn scenes_that_claim_a_design_frame_have_its_png() {
        let claimed: Vec<_> = scenes().iter().filter(|s| s.design).collect();
        assert!(!claimed.is_empty(), "no scene claims a design frame");
        for scene in claimed {
            let png = design_png(scene.frame);
            assert!(
                png.exists(),
                "{:?} claims a frame but {} is missing",
                scene.frame,
                png.display()
            );
        }
    }

    #[test]
    fn scenes_every_registered_scene_renders_a_shell() {
        for scene in scenes() {
            let html = render_scene_html(scene.frame)
                .unwrap_or_else(|| panic!("{:?} is registered but does not render", scene.frame));
            assert!(
                html.contains("titlebar"),
                "{:?} renders no shell",
                scene.frame
            );
        }
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
    fn scenes_render_the_stats_panel_with_seeded_history() {
        let html = render_scene_html("Stats / panel").expect("scene renders");
        assert!(html.contains("shop-api-api-1"), "{html}");
        for label in ["CPU", "Memory", "Network in", "Network out"] {
            assert!(html.contains(label), "{label}: {html}");
        }
        assert!(html.contains("<path d=\"M"), "sparklines are drawn: {html}");
        assert!(html.contains("of 512 MB"), "{html}");
        assert!(!html.contains("Waiting for the first sample"), "{html}");
    }

    #[test]
    fn scenes_synthetic_stats_counters_only_grow() {
        let start = chrono::Utc::now();
        let mut last = synthetic_sample(start);
        for s in 1..600 {
            let next = synthetic_sample(start + chrono::Duration::seconds(s));
            assert!(next.net_rx >= last.net_rx && next.net_tx >= last.net_tx);
            assert!(next.cpu_pct > 0.0);
            last = next;
        }
    }

    #[test]
    fn scenes_unknown_frame_renders_nothing() {
        assert!(render_scene_html("Projects / bogus").is_none());
    }
}
