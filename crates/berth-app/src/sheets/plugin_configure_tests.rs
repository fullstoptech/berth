//! P9-T2 tests for the Configure form: the save rules and the form as markup.

use std::collections::BTreeMap;

use serde_json::json;

use super::*;
use crate::scenes;

fn sheet(frame: &str) -> String {
    let mut html = scenes::render_scene_html(frame).expect("the frame has a scene");
    while let (Some(a), Some(b)) = (html.find("<style>"), html.find("</style>")) {
        if b < a {
            break;
        }
        html.replace_range(a..b + "</style>".len(), "");
    }
    let start = html.find("class=\"pl-sheet\"").expect("the sheet is drawn");
    html[start..].to_string()
}

fn entry() -> PluginEntry {
    use berth_plugin_protocol::{ConfigField, Manifest, SecretDecl};
    PluginEntry {
        plugin: berth_plugin_host::InstalledPlugin {
            manifest: Manifest {
                id: "demo".into(),
                name: "Demo".into(),
                version: "1.0.0".into(),
                protocol: 1,
                executable: "demo".into(),
                permissions: Vec::new(),
                contributes: Vec::new(),
                config: vec![
                    ConfigField {
                        key: "base_url".into(),
                        label: "Demo address".into(),
                        kind: FieldKind::Url,
                        required: true,
                    },
                    ConfigField {
                        key: "api_key".into(),
                        label: "API key".into(),
                        kind: FieldKind::Secret,
                        required: true,
                    },
                    ConfigField {
                        key: "verbose".into(),
                        label: "Verbose".into(),
                        kind: FieldKind::Bool,
                        required: false,
                    },
                ],
                secrets: vec![SecretDecl {
                    key: "other/token".into(),
                    label: "Other token".into(),
                }],
            },
            dir: std::path::PathBuf::from("/x"),
            state: berth_plugin_host::PluginState::Disabled,
            granted: Vec::new(),
        },
        config: json!({}),
        secrets_set: std::collections::BTreeSet::new(),
        crash_log: Vec::new(),
        restarts: 0,
    }
}

fn values(url: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([("base_url".to_string(), json!(url))])
}

fn typed(key: &str, text: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(key.to_string(), text.to_string())])
}

#[test]
fn plugin_configure_blocks_save_until_required_settings_are_valid() {
    let e = entry();
    assert_eq!(
        form_blocker(&e, &BTreeMap::new(), &BTreeMap::new()).as_deref(),
        Some("Demo address is required")
    );
    assert_eq!(
        form_blocker(&e, &values("nope"), &typed("api_key", "k")).as_deref(),
        Some("Demo address is not a web address")
    );
    assert_eq!(
        form_blocker(&e, &values("ftp://x.example"), &typed("api_key", "k")).as_deref(),
        Some("Demo address must start with http:// or https://")
    );
    // The required secret is the next blocker, then the form is clear.
    assert_eq!(
        form_blocker(&e, &values("https://x.example"), &BTreeMap::new()).as_deref(),
        Some("API key is required")
    );
    assert_eq!(
        form_blocker(&e, &values("https://x.example"), &typed("api_key", "k")),
        None
    );
}

#[test]
fn plugin_configure_a_stored_secret_satisfies_a_required_secret_field() {
    let mut e = entry();
    e.secrets_set.insert("api_key".to_string());
    assert_eq!(
        form_blocker(&e, &values("https://x.example"), &BTreeMap::new()),
        None
    );
}

#[test]
fn plugin_configure_starts_from_the_saved_settings_with_switches_off_by_default() {
    let mut e = entry();
    let fresh = initial_values(&e);
    assert_eq!(fresh.get("verbose"), Some(&json!(false)));
    assert!(!fresh.contains_key("base_url"));
    assert!(
        !fresh.contains_key("api_key"),
        "secrets are never prefilled"
    );
    e.config = json!({"base_url": "https://x.example", "verbose": true});
    let saved = initial_values(&e);
    assert_eq!(saved.get("base_url"), Some(&json!("https://x.example")));
    assert_eq!(saved.get("verbose"), Some(&json!(true)));
}

#[test]
fn plugin_configure_new_form_has_masked_secret_and_a_blocked_save() {
    let html = sheet("Dev / plugins / configure new");
    for needle in [
        "aria-label=\"Configure Coolify\"",
        "Coolify address",
        "Offer Coolify&#39;s MCP server to the Assistant",
        "API token",
        "Stored in your Keychain once you save.",
        "Save settings",
        "Saving restarts the plugin.",
    ] {
        assert!(
            html.contains(needle) || html.contains(&needle.replace("&#39;", "'")),
            "missing {needle:?}: {html}"
        );
    }
    assert!(
        html.contains("type=\"password\""),
        "the token is masked: {html}"
    );
    assert!(
        !html.contains("Remove saved value"),
        "nothing stored yet: {html}"
    );
    // Save is disabled and says why.
    let save = html.find("Save settings").expect("save button");
    let button = &html[html[..save].rfind("<button").expect("button")..save];
    assert!(button.contains("disabled"), "{button}");
    assert!(
        button.contains("data-tooltip=\"Plugins are not connected\"")
            || button.contains("data-tooltip=\"Coolify address is required\""),
        "{button}"
    );
}

#[test]
fn plugin_configure_saved_form_shows_the_address_and_a_saved_token_that_can_go() {
    let html = sheet("Dev / plugins / configure saved");
    assert!(html.contains("https://coolify.acme.dev"), "{html}");
    assert!(
        html.contains("Saved in your Keychain. Leave empty to keep it."),
        "{html}"
    );
    assert!(html.contains("Remove saved value"), "{html}");
    assert!(html.contains("placeholder=\"Saved\""), "{html}");
    // The stored token is never read back into the field.
    let field = html.find("type=\"password\"").expect("masked field");
    assert!(
        html[field..].starts_with("type=\"password\" initial_value=\"\""),
        "{html}"
    );
}
