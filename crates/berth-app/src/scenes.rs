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

/// Marks the "Files / panel" dev scene (context provided by its setup, read
/// by the shell): one mock container's Files tab over an in-memory folder.
#[derive(Clone)]
pub(crate) struct DevFilesPanel(pub berth_core::ContainerId);

/// "Files / panel": the Files tab of `shop-api-api-1`, opened on its `/app`
/// mount, over a small in-memory Node project.
fn files_panel(state: &mut AppState) {
    use crate::services::files::{FilesBackend, MemoryFiles};
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Settings);
    let mut containers = state.containers.peek().clone();
    let Some(container) = containers.iter_mut().find(|c| c.name == "shop-api-api-1") else {
        return;
    };
    container.mounts.insert(
        0,
        berth_core::Mount {
            kind: berth_core::MountKind::Bind,
            source: "/Users/berth/Dev/clients/acme/shop-api".to_string(),
            destination: "/app".to_string(),
            read_only: false,
            volume_name: None,
        },
    );
    let id = container.id.clone();
    state.containers.set(containers);
    let now = chrono::Utc::now().timestamp();
    let day = 86_400;
    let files = MemoryFiles::new()
        .with_dir("/app/node_modules", now - 3 * day)
        .with_file(
            "/app/node_modules/.package-lock.json",
            &[b'x'; 41_000],
            now - 3 * day,
        )
        .with_dir("/app/src", now - 3_600)
        .with_file("/app/src/server.js", &[b'x'; 3_210], now - 3_600)
        .with_dir("/app/public", now - 5 * day)
        .with_file("/app/.env.example", &[b'x'; 212], now - 40 * day)
        .with_file("/app/Dockerfile", &[b'x'; 418], now - 40 * day)
        .with_file("/app/package.json", &[b'x'; 1_187], now - 2 * day)
        .with_file("/app/package-lock.json", &[b'x'; 284_300], now - 2 * day)
        .with_file("/app/README.md", &[b'x'; 5_940], now - 400 * day)
        .with_file("/app/server.log", &[b'x'; 12_582_912], now - 90);
    provide_context(FilesBackend(std::sync::Arc::new(files)));
    provide_context(DevFilesPanel(id));
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
    frame_worker_state(state);
}

/// What the frame's inspector shows of the restarting worker: it last
/// started 3 seconds ago, and its problem (2 min old) has the explanation,
/// evidence and fixes of the card (P3-T3).
fn frame_worker_state(state: &mut AppState) {
    use berth_core::{
        ContainerAction, ContainerId, EffectSummary, EngineAction, Fix, FixEffect, FixSource,
        HarnessId,
    };
    let now = chrono::Utc::now();
    let worker = {
        let mut containers = state.containers.write();
        containers
            .iter_mut()
            .find(|c| c.name == "shop-api-worker-1")
            .map(|worker| {
                worker.started = Some(now - chrono::Duration::seconds(3));
                (worker.engine.clone(), worker.id.clone())
            })
    };
    let Some((engine, id)) = worker else { return };
    let mut problems = state.problems.peek().clone();
    if let Some(problem) = problems.first_mut() {
        problem.first_seen = now - chrono::Duration::seconds(130);
        problem.explanation = "The worker exits with code 1 about 3 seconds after starting \
            because it cannot reach the database. It has restarted 14 times."
            .to_string();
        problem.evidence = vec![
            "Error: connect ECONNREFUSED 172.19.0.3:5432".to_string(),
            "at TCPConnectWrap.afterConnect [as oncomplete] (node:net:1611:16)".to_string(),
        ];
        problem.fixes = vec![
            Fix {
                label: "Stop the restart loop".to_string(),
                source: FixSource::Rule,
                effect: FixEffect::RunAction {
                    action: EngineAction::Container {
                        engine,
                        id: ContainerId(id.0),
                        action: ContainerAction::Stop { timeout_s: None },
                    },
                    effect: EffectSummary {
                        verb: "Stop".to_string(),
                        noun_one: "container".to_string(),
                        noun_many: "containers".to_string(),
                        count: 1,
                        bytes: None,
                        lines: Vec::new(),
                        typed_name: None,
                    },
                },
            },
            Fix {
                label: "Fix with Claude Code".to_string(),
                source: FixSource::Harness(HarnessId("claude".to_string())),
                effect: FixEffect::Explain,
            },
        ];
    }
    state.problems.set(problems);
}

/// Give the selected containers their detail from the mock engine, as the
/// detail service would have loaded it. Scenes run no service (D58), so
/// this is the one place a scene reads an engine, and the mock has no I/O.
pub fn seed_details(state: &AppState, store: crate::services::detail::DetailStore) {
    let Selection::Containers(ids) = state.selection.peek().clone() else {
        return;
    };
    let world = crate::services::mock_world_at(chrono::Utc::now());
    let containers = state.containers.peek().clone();
    for container in ids
        .iter()
        .filter_map(|id| containers.iter().find(|c| c.id == *id))
    {
        let Ok(engine) = berth_engine::MockEngine::from_world(world.clone(), &container.engine)
        else {
            continue;
        };
        let target = container.clone();
        // A short-lived thread with its own runtime: the log read has a
        // timeout, and this may run inside another runtime already.
        let loaded = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_time()
                        .build()
                        .ok()?;
                    runtime
                        .block_on(crate::services::detail::load(
                            &engine,
                            &target,
                            tokio_util::sync::CancellationToken::new(),
                        ))
                        .ok()
                })
                .join()
                .ok()
                .flatten()
        });
        if let Some(loaded) = loaded {
            store.seed(container, loaded);
        }
    }
}

/// The Containers screen with the bottom panel open on its Terminal tab
/// (frames "Container / terminal / 1440" and "... / no shell / 1440"):
/// dark, the inspector hidden, every group collapsed but `open_group`.
fn terminal_base(state: &mut AppState, selected: &str, open_group: &str) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Dark;
    state.section.set(Section::Containers);
    let containers = state.containers.peek().clone();
    state.problems.set(mock_problems(&containers));
    if let Some(container) = containers.iter().find(|c| c.name == selected) {
        state
            .selection
            .set(Selection::Containers(vec![container.id.clone()]));
    }
    if let Some(view) = try_consume_context::<crate::screens::containers::ContainersView>() {
        let mut toggled = view.inspector_toggled;
        toggled.set(true);
        let mut collapsed = view.collapsed;
        let mut map = collapsed.write();
        for project in state.projects.peek().iter() {
            map.insert(
                format!("project:{}", project.id.0),
                project.name != open_group,
            );
        }
        map.insert("unassigned".to_string(), open_group != "unassigned");
    }
    let mut bottom = state.bottom.write();
    bottom.open = true;
    bottom.height_px = 408;
    bottom.tab = crate::state::BottomTab::Terminal;
}

/// Seed one terminal tab with its output and no engine behind it.
fn seed_terminal(
    state: &mut AppState,
    container: &str,
    status: crate::services::exec::Status,
    shell: Option<&str>,
    output: &str,
    unread: bool,
) -> Option<uuid::Uuid> {
    use crate::services::exec::{container_spec, ExecCore, Kind};
    let core = try_consume_context::<ExecCore>()?;
    let found = state
        .containers
        .peek()
        .iter()
        .find(|c| c.name == container)
        .cloned()?;
    let project = crate::services::exec::project_name(state, &found);
    let mut spec = container_spec(&found, project.as_deref(), shell.map(str::to_string));
    if let Kind::Exec(exec) = &mut spec.kind {
        exec.wait_running = false;
        exec.workdir = Some("/app".to_string());
    }
    let id = uuid::Uuid::now_v7();
    core.seed(id, spec, status.clone(), shell, output.as_bytes());
    let title = core.view(id).map(|v| v.title)?;
    state
        .bottom
        .write()
        .terminals
        .push(crate::state::TerminalTab {
            id,
            title,
            session: None,
            exited: status.is_ended(),
            unread,
        });
    Some(id)
}

/// Show `tab` in the terminal panel.
fn show_terminal(tab: Option<uuid::Uuid>) {
    if let (Some(tab), Some(view)) = (
        tab,
        try_consume_context::<crate::services::exec::TerminalView>(),
    ) {
        let mut active = view.active;
        active.set(Some(tab));
    }
}

/// Dark; the api's bash session on top, redis (sh) with new output and the
/// worker's ended session behind it.
fn terminal_active(state: &mut AppState) {
    use crate::services::exec::Status;
    terminal_base(state, "shop-api-api-1", "shop-api");
    let prompt = "root@a3f9c2e1b7d4:/app#";
    let transcript = format!(
        "{prompt} \x1b[1mls\x1b[0m\r\n\
         \x1b[37mDockerfile  dist  node_modules  package.json  pnpm-lock.yaml  prisma  src  tsconfig.json\x1b[0m\r\n\
         {prompt} \x1b[1mpnpm prisma migrate status\x1b[0m\r\n\
         \x1b[37mPrisma schema loaded from prisma/schema.prisma\r\n\
         Datasource \"db\": PostgreSQL database \"shop\", schema \"public\" at \"db:5432\"\x1b[0m\r\n\r\n\
         \x1b[91mError: P1001: Can't reach database server at `db:5432`\x1b[0m\r\n\
         \x1b[37mPlease make sure your database server is running at `db:5432`.\x1b[0m\r\n\
         {prompt} "
    );
    let first = seed_terminal(
        state,
        "shop-api-api-1",
        Status::Live,
        Some("bash"),
        &transcript,
        false,
    );
    seed_terminal(
        state,
        "shop-api-redis-1",
        Status::Live,
        Some("sh"),
        "/data # ",
        true,
    );
    seed_terminal(
        state,
        "shop-api-worker-1",
        Status::Exited,
        Some("sh"),
        "",
        false,
    );
    show_terminal(first);
}

/// Dark; the distroless metrics exporter has neither bash nor sh.
fn terminal_no_shell(state: &mut AppState) {
    use crate::services::exec::Status;
    terminal_base(state, "metrics-exporter-1", "unassigned");
    let first = seed_terminal(
        state,
        "metrics-exporter-1",
        Status::NoShell {
            tried: vec!["bash".to_string(), "sh".to_string()],
        },
        None,
        "",
        false,
    );
    show_terminal(first);
}

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
        frame: "Files / panel",
        setup: files_panel,
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
        frame: "Logs / errors / 1440",
        setup: logs_errors,
    },
    Scene {
        frame: "Logs / paused / 1440",
        setup: logs_paused,
    },
    Scene {
        frame: "Palette / open / 1440",
        setup: palette_open,
    },
    Scene {
        frame: "Container / terminal / 1440",
        setup: terminal_active,
    },
    Scene {
        frame: "Container / terminal / no shell / 1440",
        setup: terminal_no_shell,
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
    crate::services::exec::provide_scene_terminals();
    let details = crate::services::detail::provide_detail_store();
    let setup_frame = frame.clone();
    use_hook(move || {
        if let Some(scene) = find(&setup_frame) {
            (scene.setup)(&mut state);
            seed_details(&state, details);
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
        assert_eq!(scenes().len(), 21);
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
