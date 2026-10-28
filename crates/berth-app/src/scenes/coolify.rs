//! Scenes of the Coolify section (P9-T5; frames "Coolify / deployments /
//! 1440" and "Coolify / confirm redeploy / 1440"). Every view is a recorded
//! answer of the real `berth-coolify` plugin, read from
//! `fixtures/plugin_views/coolify/` (the `coolify_views` test compares those
//! files with what the plugin answers today). A scene runs no plugin (D58).

use super::*;

use berth_plugin_host::PluginState;
use berth_plugin_protocol::View;
use chrono::TimeZone;

use crate::services::plugin_views::{LogState, PendingAction, PluginNav, PluginViewStore, ViewState};
use crate::services::plugins::PluginsView;

const RESOURCES: &str = include_str!("../../fixtures/plugin_views/coolify/resources.json");
const DEPLOYMENTS: &str = include_str!("../../fixtures/plugin_views/coolify/deployments.json");
const APPLICATION: &str = include_str!("../../fixtures/plugin_views/coolify/application.json");
const LOG_VIEW: &str = include_str!("../../fixtures/plugin_views/coolify/deployment-log-view.json");
const LOG_LINES: &str = include_str!("../../fixtures/plugin_views/coolify/deployment-log-lines.json");
const STATUS: &str = include_str!("../../fixtures/plugin_views/coolify/status.json");

/// Coolify enabled and configured, as the scenes show it.
pub(crate) fn enabled_entry() -> crate::services::plugins::PluginEntry {
    super::plugins::configured(super::plugins::coolify(PluginState::Enabled, true))
}

/// The failed deployment of the frames and its log stream.
const FAILED_ROW: &str = "dep-uuid-00ff";
const FAILED_STREAM: &str = "deployment/dep-uuid-00ff/logs";

#[allow(clippy::expect_used)] // checked-in recordings
fn recorded(text: &str) -> View {
    serde_json::from_str(text).expect("a recorded view parses")
}

/// The failed deployment's lines as the frame times them: 14:30:41 on.
#[allow(clippy::expect_used)] // checked-in recording
fn recorded_log() -> LogState {
    let value: serde_json::Value = serde_json::from_str(LOG_LINES).expect("recorded log parses");
    // Seconds after 14:30:41 of each line (the plugin's lines carry no time).
    const AT: [i64; 10] = [0, 1, 7, 8, 8, 10, 10, 10, 11, 11];
    let start = chrono::Utc
        .with_ymd_and_hms(2026, 10, 24, 14, 30, 41)
        .single()
        .unwrap_or_default();
    let lines = value["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .enumerate()
        .map(|(i, text)| {
            let offset = AT.get(i).copied().unwrap_or(11);
            (start + chrono::Duration::seconds(offset), text.to_string())
        })
        .collect();
    LogState {
        lines,
        final_status: value["final_status"].as_str().map(str::to_string),
        dropped: 0,
        version: 1,
    }
}

/// The section as the frames show it: the dark theme, the mock world's
/// projects, Coolify enabled and its recorded views in the store, the failed
/// deployment selected with its log open.
fn deployments_setup(state: &mut AppState) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Dark;
    state.section.set(Section::Plugin);
    let containers = state.containers.peek().clone();
    state.problems.set(mock_problems(&containers));
    if let Some(mut plugins) = try_consume_context::<PluginsView>() {
        plugins.entries.set(vec![enabled_entry()]);
    }
    let Some(store) = try_consume_context::<PluginViewStore>() else {
        return;
    };
    store.seed_view("coolify", "resources", ViewState::Ready(recorded(RESOURCES)));
    store.seed_view("coolify", "deployments", ViewState::Ready(recorded(DEPLOYMENTS)));
    store.seed_view("coolify", "status", ViewState::Ready(recorded(STATUS)));
    store.seed_view(
        "coolify",
        "application/app-uuid-0001",
        ViewState::Ready(recorded(APPLICATION)),
    );
    store.seed_view(
        "coolify",
        FAILED_STREAM,
        ViewState::Ready(recorded(LOG_VIEW)),
    );
    store.seed_log("coolify", FAILED_STREAM, recorded_log());
    let mut nav = store.nav;
    nav.set(PluginNav {
        plugin: "coolify".to_string(),
        view: "deployments".to_string(),
        row: Some(FAILED_ROW.to_string()),
        open: Some(FAILED_STREAM.to_string()),
    });
}

/// "Coolify / deployments / 1440".
fn deployments(state: &mut AppState) {
    deployments_setup(state);
}

/// "Coolify / confirm redeploy / 1440": Deploy's own confirm, the one the
/// plugin sends for the landing application (its action has the Force rebuild
/// option), over the same screen.
#[allow(clippy::expect_used)] // checked-in recording
fn confirm_redeploy(state: &mut AppState) {
    deployments_setup(state);
    let View::Detail { actions, .. } = recorded(APPLICATION) else {
        return;
    };
    let deploy = actions
        .into_iter()
        .find(|a| a.id == "deploy")
        .expect("the recorded application has Deploy");
    if let Some(store) = try_consume_context::<PluginViewStore>() {
        let mut confirm = store.confirm;
        confirm.set(Some(PendingAction {
            plugin: "coolify".to_string(),
            action: deploy,
        }));
    }
}

pub(super) const SCENES: &[Scene] = &[
    Scene::design("Coolify / confirm redeploy / 1440", confirm_redeploy),
    Scene::design("Coolify / deployments / 1440", deployments),
];
