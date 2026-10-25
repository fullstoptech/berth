//! Scenes of the engines screens.

use super::*;

// --- Engines frames (P4-T6) ---

/// The Engines screen over the mock world, as the frames show it: the three
/// engines with the unreachable one last seen four minutes ago, the colima
/// version, `default`'s disk usage and five minutes of load ending at the
/// frame's 23.7 % and 1.9 GB, the engine's thirteen recent lines.
fn load_engines_world(state: &mut AppState) {
    use crate::services::engine_dashboard::{DashboardStore, EngineSeries, UsageEntry};
    super::projects::load_projects_world(state, berth_store::Theme::Light);
    state.section.set(Section::Engines);
    let default = berth_core::EngineId("colima:default".to_string());
    state.selection.set(Selection::Engine(default.clone()));
    let now = chrono::Utc::now();
    for engine in state.engines.write().iter_mut() {
        match engine.id.0.as_str() {
            // The frame's stopped profile holds six containers.
            "colima:work" => engine.counts.containers_total = 6,
            "ssh:hetzner-1" => {
                if let berth_core::EngineState::Unreachable { last_seen, .. } = &mut engine.state {
                    *last_seen = Some(now - chrono::Duration::minutes(4));
                }
            }
            _ => {}
        }
    }
    crate::screens::recent_activity::seed_recent(engine_recent_entries(
        &berth_engine::MockWorld::bundled(),
        now,
    ));
    let world = crate::services::mock_world_at(now);
    // The frame's disk: 22.5 of 60 GB used (volumes 6.2 GB), 5.7 GB reclaimable.
    let usage = world
        .disk_usage_by_engine
        .get(&default.0)
        .cloned()
        .map(|mut u| {
            u.volumes.size = bytesize::ByteSize::mb(6_200);
            u.volumes.reclaimable = bytesize::ByteSize(0);
            u
        });
    let mut series = EngineSeries::default();
    // 150 points, two seconds apart; the last is the frame's reading.
    for step in (0..150_i64).rev() {
        let at = now - chrono::Duration::seconds(step * 2);
        #[allow(clippy::cast_precision_loss)]
        let t = (150 - step) as f64;
        let cpu = if step == 0 {
            23.7
        } else {
            13.0 + 8.0 * (t / 150.0) + 3.0 * (t / 11.0).sin() + 1.8 * (t / 3.7).sin()
        };
        let mem = 1.55e9 + 0.35e9 * (t / 150.0) + 0.02e9 * (t / 6.0).sin();
        series.push(at, cpu, if step == 0 { 1.9e9 } else { mem });
    }
    if let Some(store) = try_consume_context::<DashboardStore>() {
        store.seed(&default, usage.map(UsageEntry::Ready), Some(series));
        store.seed_version("0.10.3");
    }
}

/// "Recent activity" of the Engines frame: the Projects frame's eight lines
/// and the five older ones the dashboard has room for.
fn engine_recent_entries(
    world: &berth_engine::MockWorld,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<berth_core::ActivityEntry> {
    use berth_core::{ActivityOrigin, Outcome, Subject};
    let mut entries = crate::screens::recent_activity::mock_entries(world, now);
    let default = berth_core::EngineId("colima:default".to_string());
    let container = |name: &str| {
        world
            .containers
            .iter()
            .find(|c| c.name == name)
            .map_or_else(
                || Subject::Container(berth_core::ContainerId(name.to_string())),
                |c| Subject::Container(c.id.clone()),
            )
    };
    let Some(template) = entries.last().cloned() else {
        return entries;
    };
    let extra = |ago_s: i64, action: &str, subject: Subject, name: &str| {
        let mut entry = template.clone();
        entry.id = uuid::Uuid::now_v7();
        entry.ts = now - chrono::Duration::seconds(ago_s);
        entry.engine = default.clone();
        entry.origin = ActivityOrigin::Engine;
        entry.action = action.to_string();
        entry.project = None;
        entry.subject = subject;
        entry.subject_name = name.to_string();
        entry.outcome = Outcome::Ok;
        entry
    };
    entries.push(extra(
        11_000,
        "container.health_status: healthy",
        container("shop-api-redis-1"),
        "shop-api-redis-1",
    ));
    entries.push(extra(
        11_400,
        "container.health_status: healthy",
        container("shop-api-api-1"),
        "shop-api-api-1",
    ));
    entries.push(extra(
        11_700,
        "image.pull",
        Subject::Engine(default.clone()),
        "node:22-alpine",
    ));
    entries.push(extra(
        12_000,
        "container.start",
        container("shop-api-api-1"),
        "shop-api-api-1",
    ));
    entries.push(extra(
        12_300,
        "image.pull",
        Subject::Engine(berth_core::EngineId("colima:default".to_string())),
        "axllent/mailpit:v1.21",
    ));
    entries
}

/// "Engines / dashboard / 1440": `default` selected and running.
fn engines_dashboard(state: &mut AppState) {
    load_engines_world(state);
}

/// "Dev / edit resources": the sheet over the dashboard (no frame).
fn edit_resources_sheet(state: &mut AppState) {
    load_engines_world(state);
    state
        .overlays
        .write()
        .push(crate::state::Overlay::EditResources(
            "colima:default".to_string(),
        ));
}

/// "Engines / starting / 1440": `default` starting, 8 s into about 20.
fn engines_starting(state: &mut AppState) {
    use crate::services::colima::{RunLine, RunView};
    load_engines_world(state);
    let default = berth_core::EngineId("colima:default".to_string());
    if let Some(engine) = state.engines.write().iter_mut().find(|e| e.id == default) {
        engine.state = berth_core::EngineState::Starting;
    }
    state.pending.write().insert(
        crate::state::PendingKey {
            subject: berth_core::Subject::Engine(default.clone()),
            action: "engine.start",
        },
        crate::state::PendingState::Running {
            label: "Starting\u{2026}".to_string(),
        },
    );
    let line = |at_s: f64, text: &str| RunLine {
        at_s,
        text: text.to_string(),
    };
    provide_context(crate::screens::engines::SceneRun {
        engine: default,
        view: RunView {
            action: "engine.start",
            elapsed_s: 8.0,
            lines: vec![
                line(0.0, "INFO[0000] starting colima [profile=default]"),
                line(0.1, "INFO[0000] runtime: docker"),
                line(1.4, "INFO[0001] starting ...  context=vm"),
                line(5.8, "INFO[0005] provisioning ...  context=docker"),
                line(
                    7.9,
                    "INFO[0007] waiting for docker socket ...  context=docker",
                ),
            ],
            failure: None,
        },
    });
}

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Engines / dashboard / 1440", engines_dashboard),
    Scene::design("Engines / starting / 1440", engines_starting),
    Scene::dev("Dev / edit resources", edit_resources_sheet),
];
