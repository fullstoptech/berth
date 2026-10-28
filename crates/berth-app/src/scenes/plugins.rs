//! Scenes of the Plugins screen and its two sheets (P9-T2, AD48; no frame).
//! They use the real `plugins/coolify/berth-plugin.json`, so the sheets list
//! what the shipped manifest really asks for. A scene runs no service (D58):
//! each one puts the entries into the view as the service would publish them.

use super::*;

use berth_plugin_host::{InstalledPlugin, PluginState};

use crate::services::plugins::{PluginEntry, PluginsView};
use crate::state::Overlay;

const COOLIFY_MANIFEST: &str = include_str!("../../../../plugins/coolify/berth-plugin.json");

#[allow(clippy::expect_used)] // the manifest is a checked-in file
pub(super) fn coolify(state: PluginState, granted: bool) -> PluginEntry {
    let manifest: berth_plugin_protocol::Manifest =
        serde_json::from_str(COOLIFY_MANIFEST).expect("the Coolify manifest parses");
    let granted = if granted {
        manifest.permissions.clone()
    } else {
        Vec::new()
    };
    PluginEntry {
        plugin: InstalledPlugin {
            manifest,
            dir: std::path::PathBuf::from("/Applications/Berth.app/Contents/MacOS"),
            state,
            granted,
        },
        config: serde_json::json!({}),
        secrets_set: std::collections::BTreeSet::new(),
        crash_log: Vec::new(),
        restarts: 0,
    }
}

/// Coolify set up: an address and a stored token.
pub(super) fn configured(mut entry: PluginEntry) -> PluginEntry {
    entry.config = serde_json::json!({
        "base_url": "https://coolify.acme.dev",
        "mcp_enabled": true,
    });
    entry.secrets_set.insert("coolify/token".to_string());
    entry
}

fn base(state: &mut AppState, entries: Vec<PluginEntry>) {
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Plugins);
    let containers = state.containers.peek().clone();
    state.problems.set(mock_problems(&containers));
    if let Some(mut view) = try_consume_context::<PluginsView>() {
        view.entries.set(entries);
    }
}

fn empty(state: &mut AppState) {
    base(state, Vec::new());
}

/// Installed, never enabled: Enable is the one way forward.
fn installed(state: &mut AppState) {
    base(state, vec![coolify(PluginState::Disabled, false)]);
}

/// The permission request sheet over the installed list.
fn enable_sheet(state: &mut AppState) {
    base(state, vec![coolify(PluginState::Disabled, false)]);
    state
        .overlays
        .write()
        .push(Overlay::PluginEnable("coolify".to_string()));
}

/// Enabled but not configured yet: the card says what is missing, and the
/// Configure form is open on empty fields.
fn configure_new(state: &mut AppState) {
    base(state, vec![coolify(PluginState::Enabled, true)]);
    state
        .overlays
        .write()
        .push(Overlay::PluginConfigure("coolify".to_string()));
}

/// Configure again once an address and token are saved: the token reads
/// "Saved" and can be removed.
fn configure_saved(state: &mut AppState) {
    base(
        state,
        vec![configured(coolify(PluginState::Enabled, true))],
    );
    state
        .overlays
        .write()
        .push(Overlay::PluginConfigure("coolify".to_string()));
}

fn enabled(state: &mut AppState) {
    base(
        state,
        vec![configured(coolify(PluginState::Enabled, true))],
    );
}

/// The host gave up restarting it: the reason and the plugin's last output.
fn error(state: &mut AppState) {
    let mut entry = configured(coolify(
        PluginState::Error {
            reason: "exit status: 101; restarts exhausted".to_string(),
        },
        true,
    ));
    entry.restarts = 3;
    entry.crash_log = [
        "thread 'main' panicked at plugins/coolify/src/plugin.rs:212:",
        "called `Result::unwrap()` on an `Err` value: connection refused (os error 111)",
        "note: run with `RUST_BACKTRACE=1` to display a backtrace",
    ]
    .map(str::to_string)
    .to_vec();
    base(state, vec![entry]);
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Dev / plugins / configure new", configure_new),
    Scene::dev("Dev / plugins / configure saved", configure_saved),
    Scene::dev("Dev / plugins / empty", empty),
    Scene::dev("Dev / plugins / enable sheet", enable_sheet),
    Scene::dev("Dev / plugins / enabled", enabled),
    Scene::dev("Dev / plugins / error", error),
    Scene::dev("Dev / plugins / installed", installed),
];
