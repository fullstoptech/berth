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

/// Marks the "Stats / panel" dev scene (context provided by its setup, read
/// by the shell): one mock container's Stats panel on a synthetic feed.
#[derive(Clone)]
pub(crate) struct DevStatsPanel(pub berth_core::ContainerId);

/// A smooth, deterministic reading for the dev scene: a container at ~3 %
/// CPU with bursts, memory creeping up and sawing down, network counters
/// that only grow. A function of absolute time, so a live feed continues
/// the seeded history without a seam.
pub(crate) fn synthetic_sample(at: chrono::DateTime<chrono::Utc>) -> berth_core::LiveStats {
    // Seconds since the mock world's reference moment, so the counters
    // start small instead of at the Unix epoch's 1.7 billion seconds.
    #[allow(clippy::cast_precision_loss)]
    let since = (at - crate::services::mock_reference_now())
        .num_seconds()
        .max(0) as f64;
    #[allow(clippy::cast_precision_loss)]
    let t = at.timestamp() as f64;
    let burst = ((t / 90.0).sin().max(0.8) - 0.8) * 30.0;
    let cpu = 3.2 + 2.2 * (t / 17.0).sin() + 1.4 * (t / 5.0).sin() + burst;
    let mem = 150_000_000.0 + 30_000_000.0 * ((t % 300.0) / 300.0) + 4_000_000.0 * (t / 11.0).sin();
    let rx = 1_200_000.0 + 3_000.0 * since + 2_500.0 * (since + 10.0 * (t / 10.0).sin());
    let tx = 340_000.0 + 900.0 * since + 700.0 * (since + 7.0 * (t / 7.0).sin());
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    berth_core::LiveStats {
        cpu_pct: cpu.max(0.1) as f32,
        mem_used: bytesize::ByteSize::b(mem as u64),
        mem_limit: bytesize::ByteSize::mb(512),
        net_rx: bytesize::ByteSize::b(rx as u64),
        net_tx: bytesize::ByteSize::b(tx as u64),
        at,
    }
}

/// "Stats / panel": the Stats panel of `shop-api-api-1` with five minutes of
/// synthetic history and a new sample every second while the window runs.
fn stats_panel(state: &mut AppState) {
    use crate::services::stats::{StatsKey, StatsService};
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Settings);
    let Some(container) = state
        .containers
        .peek()
        .iter()
        .find(|c| c.name == "shop-api-api-1")
        .cloned()
    else {
        return;
    };
    let service = try_consume_context::<StatsService>().unwrap_or_else(|| {
        provide_context(StatsService::new(
            crate::services::engines::client_map(),
            tokio_util::sync::CancellationToken::new(),
        ))
    });
    let key = StatsKey::new(&container.engine, &container.id);
    let now = chrono::Utc::now();
    service.seed(
        &key,
        (0..150).rev().map(|i| {
            let at = now - chrono::Duration::seconds(i * 2);
            (at, synthetic_sample(at))
        }),
    );
    let feed = service.clone();
    let feed_key = key.clone();
    spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let at = chrono::Utc::now();
            feed.seed(&feed_key, [(at, synthetic_sample(at))]);
        }
    });
    provide_context(DevStatsPanel(container.id));
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

/// Dark; nothing selected (so no inspector, and the table keeps its Uptime
/// column), the palette open on the query "rest". The bare verb still picks
/// the container that needs the most attention, the restarting worker.
///
/// The palette searches every engine (D4), and "rest" would also reach
/// `backup-1` and its restic image on hetzner-1. The frame was drawn with
/// one engine's data, so this scene keeps the default engine's records
/// only (the other engines stay listed).
fn palette_open(state: &mut AppState) {
    containers_selected(state);
    state.selection.set(Selection::None);
    keep_default_engine_records(state);
    crate::screens::palette::set_initial_query("rest");
    state.overlays.write().push(crate::state::Overlay::Palette);
}

/// Drop the records of every engine but `colima:default` from the lists:
/// its containers, and the images, volumes and networks that
/// are unused or used by one of those containers.
fn keep_default_engine_records(state: &mut AppState) {
    let default = berth_core::EngineId("colima:default".to_string());
    state.containers.write().retain(|c| c.engine == default);
    let mine: Vec<berth_core::ContainerId> = state
        .containers
        .read()
        .iter()
        .map(|c| c.id.clone())
        .collect();
    let used_here = |users: &[berth_core::ContainerId]| {
        users.is_empty() || users.iter().any(|id| mine.contains(id))
    };
    state.images.write().retain(|i| used_here(&i.used_by));
    state.volumes.write().retain(|v| used_here(&v.used_by));
    state.networks.write().retain(|n| used_here(&n.containers));
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
        frame: "Projects / with unassigned / 1440",
        setup: projects_with_unassigned,
    },
    Scene {
        frame: "Components / primitives",
        setup: components_primitives,
    },
    Scene {
        frame: "Stats / panel",
        setup: stats_panel,
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
        assert_eq!(scenes().len(), 16);
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
