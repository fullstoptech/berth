//! Scenes of first run and Settings (P6-T2). The checks card on "Projects / empty / 1440" is the
//! one frame; the full-screen setup and Settings have none, so their scenes are dev scenes.
//! A scene never scans: the facts come from here, the same for every Mac.

use super::*;
use crate::services::first_run::{FirstRunStore, Probe};

/// The assistants the frame lists (its Claude Code 2.4.1 is illustrative).
pub(crate) fn frame_harnesses() -> Vec<berth_core::Harness> {
    let harness = |id: &str, name: &str, binary: &str, version: &str, transport| berth_core::Harness {
        id: berth_core::HarnessId(id.to_string()),
        name: name.to_string(),
        binary: std::path::PathBuf::from(binary),
        version: Some(version.to_string()),
        transport,
        available: true,
    };
    vec![
        harness(
            "claude-code",
            "Claude Code",
            "/Users/berth/.local/bin/claude",
            "2.4.1",
            berth_core::HarnessTransport::ClaudeStreamJson,
        ),
        harness(
            "muse",
            "Muse Code",
            "/Users/berth/.local/bin/muse",
            "1.4.12",
            berth_core::HarnessTransport::MuseMsp,
        ),
    ]
}

/// Put what a scan would have found into the store: the probe, the assistants, and no look at the
/// real file system.
pub(crate) fn seed_checks(state: &mut AppState, probe: Probe) {
    if let Some(mut store) = try_consume_context::<FirstRunStore>() {
        store.probe.set(Some(probe));
        store.check_sockets.set(false);
    }
    state.harnesses.set(frame_harnesses());
}

/// "Projects / empty / 1440" (and every state that shows the card): everything found, the engine
/// running.
pub(super) fn seed_all_found(state: &mut AppState) {
    seed_checks(state, Probe::all_found());
}

fn setup_colima_missing(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.engines.set(Vec::new());
    state.containers.set(Vec::new());
    state.projects.set(Vec::new());
    state.problems.set(Vec::new());
    state.section.set(Section::FirstRun);
    seed_checks(
        state,
        Probe {
            colima: None,
            colima_version: None,
            ..Probe::all_found()
        },
    );
}

fn setup_default_stopped(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    let mut engines = state.engines.peek().clone();
    for engine in &mut engines {
        if engine.id.0 == "colima:default" {
            engine.state = berth_core::EngineState::Stopped;
        }
    }
    state.engines.set(engines);
    state.section.set(Section::FirstRun);
    seed_all_found(state);
}

fn settings_default(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Settings);
    seed_all_found(state);
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("First run / colima missing / 1440", setup_colima_missing),
    Scene::dev("First run / default stopped / 1440", setup_default_stopped),
    Scene::dev("Settings / default / 1440", settings_default),
];
