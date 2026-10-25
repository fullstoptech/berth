//! Scenes of the containers screens.

use super::*;

/// Dark; the restarting worker selected (inspector body is P3-T3).
pub(super) fn containers_selected(state: &mut AppState) {
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

/// D22 (no frame yet, owner request 2026-10-25): the inspector docked at the
/// bottom beside the Terminal panel. The terminal scene with the inspector
/// shown instead of hidden and `inspector_dock = Bottom`; the region is 360 px
/// high so both panes have room to read.
fn inspector_bottom_split(state: &mut AppState) {
    super::terminal::terminal_active(state);
    state.settings.write().inspector_dock = berth_store::InspectorDock::Bottom;
    if let Some(view) = try_consume_context::<crate::screens::containers::ContainersView>() {
        let mut toggled = view.inspector_toggled;
        toggled.set(false);
    }
    state.bottom.write().height_px = 360;
}

/// "Containers / selected / 1024" after Space: below 1200 px the inspector
/// is an overlay over the table, at the stored width (at most half the window).
fn inspector_overlay(state: &mut AppState) {
    containers_selected(state);
    if let Some(view) = try_consume_context::<crate::screens::containers::ContainersView>() {
        let mut toggled = view.inspector_toggled;
        toggled.set(true);
    }
}

/// The same with the panel closed: the inspector alone, full width.
fn inspector_bottom_alone(state: &mut AppState) {
    containers_selected(state);
    state.settings.write().inspector_dock = berth_store::InspectorDock::Bottom;
    state.bottom.write().height_px = 320;
}

/// "Dev / edit recreate" (P3-T7, no frame): the edit and recreate sheet of
/// the loose `pg-scratch` with a port moved, a variable added
/// and a memory limit set, so the diff, the loss warning and the Compose offer show.
fn edit_recreate_sheet(state: &mut AppState) {
    use crate::state::Overlay;
    load_mock_world(state);
    state.section.set(Section::Containers);
    let Some(api) = state
        .containers
        .peek()
        .iter()
        .find(|c| c.name == "pg-scratch")
        .cloned()
    else {
        return;
    };
    state
        .selection
        .set(Selection::Containers(vec![api.id.clone()]));
    state.overlays.write().push(Overlay::EditRecreate(api.id));
    provide_context(crate::sheets::edit_recreate::DevEditDraft(|draft| {
        if let Some(port) = draft.ports.first().map(|(id, _)| *id) {
            draft.set_port(port, "5433:5432");
        }
        let env = draft.add_env();
        draft.set_env_key(env, "NEW_VAR");
        draft.set_env_value(env, "a-new-value");
        draft.memory = "256m".to_string();
    }));
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

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Containers / selected / 1440", containers_selected),
    Scene::dev("Containers / inspector bottom / 1440", inspector_bottom_split),
    Scene::dev("Containers / inspector bottom / alone / 1440", inspector_bottom_alone),
    // Rendered at 1024x700 (BERTH_WINDOW=1024x700 for shots).
    Scene::dev("Containers / inspector overlay / 1024", inspector_overlay),
    Scene::design("Containers / selected / 1440 / light", containers_selected_light),
    // Rendered at 1024x700 (BERTH_WINDOW=1024x700 for shots).
    Scene::design("Containers / selected / 1024", containers_selected),
    Scene::design("Containers / bulk / 1440", containers_bulk),
    Scene::design("Containers / confirm delete / 1440", containers_confirm_delete),
    Scene::design("Containers / loading / 1440", containers_loading),
    Scene::design("Containers / engine stopped / 1440", containers_engine_stopped),
    Scene::dev("Dev / edit recreate", edit_recreate_sheet),
];
