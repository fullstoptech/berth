//! P9-T2 tests for the Plugins screen: the pure rules and every state as the
//! markup a person would see (empty, installed, enabled, error with its crash
//! log), rendered from the scenes.

use super::*;
use crate::scenes;

fn markup(frame: &str) -> String {
    let mut out = scenes::render_scene_html(frame).expect("the frame has a scene");
    // Class names in assertions mean elements, not rules of a stylesheet.
    while let (Some(a), Some(b)) = (out.find("<style>"), out.find("</style>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</style>".len(), "");
    }
    out
}

/// The Plugins screen alone: from its section to the end of the page.
fn screen(frame: &str) -> String {
    let html = markup(frame);
    let start = html
        .find("class=\"plugins-screen\"")
        .expect("the Plugins screen is drawn");
    html[start..].to_string()
}

fn entry(state: PluginState) -> PluginEntry {
    use berth_plugin_protocol::{ConfigField, FieldKind, Manifest, Permission};
    PluginEntry {
        plugin: berth_plugin_host::InstalledPlugin {
            manifest: Manifest {
                id: "demo".into(),
                name: "Demo".into(),
                version: "1.0.0".into(),
                protocol: 1,
                executable: "demo".into(),
                permissions: vec![Permission::ReadEngines],
                contributes: Vec::new(),
                config: vec![ConfigField {
                    key: "base_url".into(),
                    label: "Demo address".into(),
                    kind: FieldKind::Url,
                    required: true,
                }],
                secrets: Vec::new(),
            },
            dir: std::path::PathBuf::from("/x"),
            state,
            granted: Vec::new(),
        },
        config: serde_json::json!({}),
        secrets_set: std::collections::BTreeSet::new(),
        crash_log: Vec::new(),
        restarts: 0,
    }
}

// -- pure rules ---------------------------------------------------------

#[test]
fn plugins_state_looks_differ_by_word_tone_and_dot_shape() {
    let on = state_look(&PluginState::Enabled);
    let off = state_look(&PluginState::Disabled);
    let bad = state_look(&PluginState::Error {
        reason: "x".to_string(),
    });
    assert_eq!((on.label, on.tone), ("Enabled", "tone-success"));
    assert_eq!((off.label, off.tone), ("Disabled", "tone-stopped"));
    assert_eq!((bad.label, bad.tone), ("Error", "tone-danger"));
    // Never colour alone: only the disabled dot is hollow.
    assert!(off.hollow && !on.hollow && !bad.hollow);
}

#[test]
fn plugins_attention_names_missing_settings_then_restarts() {
    let mut e = entry(PluginState::Enabled);
    assert_eq!(
        attention_line(&e).as_deref(),
        Some("Needs setup: Demo address")
    );
    e.config = serde_json::json!({"base_url": "https://x.example"});
    assert_eq!(attention_line(&e), None);
    e.restarts = 1;
    assert_eq!(
        attention_line(&e).as_deref(),
        Some("Restarted once after it stopped")
    );
    e.restarts = 2;
    assert_eq!(
        attention_line(&e).as_deref(),
        Some("Restarted 2 times after it stopped")
    );
    // A failed plugin shows the failure box instead.
    e.plugin.state = PluginState::Error {
        reason: "gone".to_string(),
    };
    assert_eq!(attention_line(&e), None);
}

// -- the states, as markup ------------------------------------------------

#[test]
fn plugins_empty_says_nothing_is_installed() {
    let html = screen("Dev / plugins / empty");
    assert!(html.contains("No plugins installed"), "{html}");
    assert!(!html.contains("pl-card"), "{html}");
    assert!(html.contains(">0<"), "the count badge reads 0: {html}");
}

#[test]
fn plugins_installed_lists_name_version_state_and_what_it_asks_for() {
    let html = screen("Dev / plugins / installed");
    for needle in [
        "Coolify",
        "v0.6.0",
        "Disabled",
        "Asks to",
        "Connect to any address on the network",
        "Read these stored secrets: coolify/token",
        "Offer MCP servers to the Assistant",
        "Enable\u{2026}",
        "Configure\u{2026}",
    ] {
        assert!(html.contains(needle), "missing {needle:?}: {html}");
    }
    assert!(html.contains("status-dot-hollow"), "{html}");
    assert!(!html.contains("Allowed to"), "{html}");
    assert!(
        !html.contains(">Disable<"),
        "no Disable while disabled: {html}"
    );
    assert!(!html.contains("pl-failure"), "{html}");
}

#[test]
fn plugins_enabled_offers_disable_and_says_what_it_may_do() {
    let html = screen("Dev / plugins / enabled");
    assert!(html.contains("tone-success"), "{html}");
    assert!(html.contains("Allowed to"), "{html}");
    assert!(html.contains(">Disable<"), "{html}");
    assert!(!html.contains("Enable\u{2026}"), "{html}");
    assert!(!html.contains("Needs setup"), "configured: {html}");
    assert!(!html.contains("pl-failure"), "{html}");
}

#[test]
fn plugins_enabled_without_settings_says_what_is_missing() {
    let html = screen("Dev / plugins / configure new");
    assert!(html.contains("Needs setup: Coolify address"), "{html}");
}

#[test]
fn plugins_error_shows_the_reason_the_crash_log_and_try_again() {
    let html = screen("Dev / plugins / error");
    assert!(html.contains("tone-danger"), "{html}");
    assert!(
        html.contains("Coolify stopped: exit status: 101; restarts exhausted"),
        "{html}"
    );
    assert!(html.contains("Last output"), "{html}");
    assert!(
        html.contains("connection refused (os error 111)"),
        "the log lines are shown: {html}"
    );
    assert!(html.contains("aria-label=\"Crash log\""), "{html}");
    assert!(html.contains("Try again"), "{html}");
    assert!(html.contains("role=\"alert\""), "{html}");
}

#[test]
fn plugins_the_sidebar_reaches_the_screen_and_marks_it_current() {
    let html = markup("Dev / plugins / installed");
    let label = html
        .find("class=\"sidebar-label\">Plugins<")
        .expect("a Plugins item in the sidebar");
    let item = &html[html[..label].rfind("<button").expect("button")..label];
    assert!(item.contains("aria-current=\"page\""), "{item}");
}
