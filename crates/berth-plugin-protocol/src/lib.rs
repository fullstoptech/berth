//! Wire protocol between Berth and its out-of-process plugins (PLAN §6.10, ADR 0008).
//!
//! Plugins speak JSON-RPC 2.0, one JSON object per line on stdio. This crate
//! depends on serde only, so plugin authors can use it from any Rust binary;
//! authors in other languages need only the JSON shapes defined here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub protocol: u32,
    /// Path of the plugin executable, relative to the manifest file.
    pub executable: String,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub contributes: Vec<Contribution>,
    #[serde(default)]
    pub config: Vec<ConfigField>,
    #[serde(default)]
    pub secrets: Vec<SecretDecl>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Permission {
    Network { hosts: Vec<String> },
    ReadEngines,
    RunEngineActions,
    Secrets { keys: Vec<String> },
    Mcp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Contribution {
    SidebarSection {
        id: String,
        title: String,
        items: Vec<SidebarEntry>,
    },
    /// "Deployed on Coolify: running" on a local project row.
    ProjectBadge {
        id: String,
    },
    PaletteCommands {
        commands: Vec<PaletteCommand>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct SidebarEntry {
    pub id: String,
    pub title: String,
    pub view: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct PaletteCommand {
    pub id: String,
    pub title: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ConfigField {
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    Url,
    Secret,
    Bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct SecretDecl {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

pub mod methods {
    // host -> plugin
    /// `{protocol, host_version, config}` -> `{capabilities}`.
    pub const INITIALIZE: &str = "initialize";
    /// `{view, params}` -> `View`.
    pub const VIEW_LOAD: &str = "view/load";
    /// `{action, params}` -> `ActionResult`.
    pub const ACTION_INVOKE: &str = "action/invoke";
    /// `{stream, params}` -> `{}`, then `logs/lines` notifications.
    pub const LOGS_OPEN: &str = "logs/open";
    pub const LOGS_CLOSE: &str = "logs/close";
    /// `{projects: [{id, git_remote}]}` -> `[{id, text, tone}]`.
    pub const BADGES: &str = "project/badges";
    pub const SHUTDOWN: &str = "shutdown";
    // plugin -> host
    /// `{key}` -> `{value}`; only declared and granted keys.
    pub const SECRET_GET: &str = "host/secret.get";
    pub const LOG: &str = "host/log";
    /// Notification `{stream, lines, final_status?}`.
    pub const LOG_LINES: &str = "logs/lines";
    /// Notification `{view}`.
    pub const INVALIDATE: &str = "view/invalidate";
    /// Notification `[{name, url, headers}]` (Mcp permission).
    pub const MCP_SERVERS: &str = "host/mcp.servers";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum View {
    Table {
        title: String,
        columns: Vec<Column>,
        rows: Vec<Row>,
        empty: String,
        #[serde(default)]
        groups: Vec<String>,
    },
    Detail {
        title: String,
        #[serde(default)]
        status: Option<Badge>,
        #[serde(default)]
        fields: Vec<(String, String)>,
        #[serde(default)]
        tabs: Vec<Tab>,
        #[serde(default)]
        actions: Vec<ActionSpec>,
    },
    LogStream {
        stream: String,
        title: String,
    },
    Message {
        tone: Tone,
        text: String,
        #[serde(default)]
        action: Option<ActionSpec>,
    },
    Form {
        title: String,
        fields: Vec<ConfigField>,
        submit: ActionSpec,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub key: String,
    pub label: String,
    pub mono: bool,
    pub align_right: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub id: String,
    #[serde(default)]
    pub cells: BTreeMap<String, Cell>,
    #[serde(default)]
    pub open: Option<String>,
    #[serde(default)]
    pub actions: Vec<ActionSpec>,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Cell {
    Text(String),
    Mono(String),
    Badge(Badge),
    Time(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Badge {
    pub text: String,
    pub tone: Tone,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Neutral,
    Success,
    Warning,
    Danger,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: String,
    pub title: String,
    pub view: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionSpec {
    pub id: String,
    pub label: String,
    pub danger: bool,
    pub params: serde_json::Value,
    #[serde(default)]
    pub confirm: Option<Confirm>,
    #[serde(default)]
    pub options: Vec<ActionOption>,
}

/// "Force rebuild"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionOption {
    pub key: String,
    pub label: String,
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confirm {
    pub title: String,
    pub lines: Vec<String>,
    pub button: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResult {
    pub message: String,
    #[serde(default)]
    pub open: Option<String>,
    #[serde(default)]
    pub invalidate: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

/// Serialize one message as JSON followed by `'\n'` (the line framing).
///
/// Serialization of the protocol types cannot fail; a `serde_json::Value`
/// holding something unserializable encodes as an empty line.
pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    let mut out = serde_json::to_vec(msg).unwrap_or_default();
    out.push(b'\n');
    out
}

/// Parse one line (with or without the trailing newline) into a message.
///
/// A line with both `id` and `method` is a request, an `id` without a
/// method is a response, and a method without an `id` is a notification.
/// Unknown fields are ignored.
pub fn decode(line: &[u8]) -> Result<Message, serde_json::Error> {
    #[derive(Deserialize)]
    struct Probe {
        #[serde(default)]
        id: Option<serde_json::Value>,
        #[serde(default)]
        method: Option<String>,
    }
    let probe: Probe = serde_json::from_slice(line)?;
    match (probe.id.is_some(), probe.method.is_some()) {
        (true, true) => Ok(Message::Request(serde_json::from_slice(line)?)),
        (true, false) => Ok(Message::Response(serde_json::from_slice(line)?)),
        (false, _) => Ok(Message::Notification(serde_json::from_slice(line)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip<T>(v: &T)
    where
        T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let text = serde_json::to_string(v).unwrap();
        let back: T = serde_json::from_str(&text).unwrap();
        assert_eq!(&back, v, "round trip failed for {text}");
    }

    fn sample_manifest() -> Manifest {
        Manifest {
            id: "coolify".into(),
            name: "Coolify".into(),
            version: "1.2.0".into(),
            protocol: PROTOCOL_VERSION,
            executable: "berth-coolify".into(),
            permissions: vec![
                Permission::Network {
                    hosts: vec!["coolify.example.com".into()],
                },
                Permission::ReadEngines,
                Permission::RunEngineActions,
                Permission::Secrets {
                    keys: vec!["coolify_token".into()],
                },
                Permission::Mcp,
            ],
            contributes: vec![
                Contribution::SidebarSection {
                    id: "coolify".into(),
                    title: "Coolify".into(),
                    items: vec![SidebarEntry {
                        id: "deployments".into(),
                        title: "Deployments".into(),
                        view: "deployments".into(),
                    }],
                },
                Contribution::ProjectBadge {
                    id: "coolify-badge".into(),
                },
                Contribution::PaletteCommands {
                    commands: vec![PaletteCommand {
                        id: "deploy".into(),
                        title: "Deploy".into(),
                        action: "deploy".into(),
                    }],
                },
            ],
            config: vec![
                ConfigField {
                    key: "url".into(),
                    label: "Instance URL".into(),
                    kind: FieldKind::Url,
                    required: true,
                },
                ConfigField {
                    key: "label".into(),
                    label: "Label".into(),
                    kind: FieldKind::Text,
                    required: false,
                },
                ConfigField {
                    key: "token".into(),
                    label: "Token".into(),
                    kind: FieldKind::Secret,
                    required: true,
                },
                ConfigField {
                    key: "poll".into(),
                    label: "Poll".into(),
                    kind: FieldKind::Bool,
                    required: false,
                },
            ],
            secrets: vec![SecretDecl {
                key: "coolify_token".into(),
                label: "API token".into(),
            }],
        }
    }

    fn sample_action() -> ActionSpec {
        ActionSpec {
            id: "deploy".into(),
            label: "Deploy".into(),
            danger: true,
            params: json!({"uuid": "abc"}),
            confirm: Some(Confirm {
                title: "Deploy shop-api?".into(),
                lines: vec!["Server: hetzner-1".into()],
                button: "Deploy".into(),
            }),
            options: vec![ActionOption {
                key: "force".into(),
                label: "Force rebuild".into(),
                default: false,
            }],
        }
    }

    #[test]
    fn manifest_round_trip_covers_every_permission_and_contribution() {
        round_trip(&sample_manifest());
    }

    #[test]
    fn manifest_permission_kinds_are_snake_case_on_the_wire() {
        let text = serde_json::to_string(&sample_manifest()).unwrap();
        for kind in [
            "\"kind\":\"network\"",
            "\"kind\":\"read_engines\"",
            "\"kind\":\"run_engine_actions\"",
            "\"kind\":\"secrets\"",
            "\"kind\":\"mcp\"",
            "\"kind\":\"sidebar_section\"",
            "\"kind\":\"project_badge\"",
            "\"kind\":\"palette_commands\"",
        ] {
            assert!(text.contains(kind), "missing {kind} in {text}");
        }
    }

    #[test]
    fn manifest_with_an_unknown_permission_fails_with_a_clear_message() {
        let bad = json!({
            "id": "x", "name": "X", "version": "1", "protocol": 1,
            "executable": "x",
            "permissions": [{"kind": "filesystem", "paths": ["/"]}]
        });
        let err = serde_json::from_value::<Manifest>(bad).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("filesystem"), "error names the kind: {msg}");
        assert!(msg.contains("unknown variant"), "error is clear: {msg}");
    }

    #[test]
    fn manifest_with_an_unknown_contribution_fails() {
        let bad = json!({
            "id": "x", "name": "X", "version": "1", "protocol": 1,
            "executable": "x",
            "contributes": [{"kind": "menu_bar"}]
        });
        assert!(serde_json::from_value::<Manifest>(bad).is_err());
    }

    #[test]
    fn manifest_ignores_unknown_fields_and_defaults_missing_lists() {
        let v = json!({
            "id": "x", "name": "X", "version": "1", "protocol": 1,
            "executable": "x", "author": "someone", "future": {"a": 1}
        });
        let m: Manifest = serde_json::from_value(v).unwrap();
        assert_eq!(m.id, "x");
        assert!(m.permissions.is_empty() && m.contributes.is_empty());
    }

    #[test]
    fn request_response_notification_round_trip_through_encode_decode() {
        let req = Request {
            jsonrpc: "2.0".into(),
            id: 7,
            method: methods::VIEW_LOAD.into(),
            params: json!({"view": "deployments"}),
        };
        assert_eq!(decode(&encode(&req)).unwrap(), Message::Request(req));
        let ok = Response {
            jsonrpc: "2.0".into(),
            id: 7,
            result: Some(json!({"ok": true})),
            error: None,
        };
        assert_eq!(decode(&encode(&ok)).unwrap(), Message::Response(ok));
        let err = Response {
            jsonrpc: "2.0".into(),
            id: 7,
            result: None,
            error: Some(RpcError {
                code: -32000,
                message: "token invalid".into(),
            }),
        };
        assert_eq!(decode(&encode(&err)).unwrap(), Message::Response(err));
        let note = Notification {
            jsonrpc: "2.0".into(),
            method: methods::INVALIDATE.into(),
            params: json!({"view": "deployments"}),
        };
        assert_eq!(decode(&encode(&note)).unwrap(), Message::Notification(note));
    }

    #[test]
    fn encode_frames_exactly_one_line_and_decode_tolerates_no_newline() {
        let note = Notification {
            jsonrpc: "2.0".into(),
            method: methods::LOG.into(),
            params: json!({"line": "hi"}),
        };
        let bytes = encode(&note);
        assert_eq!(bytes.iter().filter(|b| **b == b'\n').count(), 1);
        assert_eq!(bytes.last(), Some(&b'\n'));
        let bare = &bytes[..bytes.len() - 1];
        assert_eq!(decode(bare).unwrap(), Message::Notification(note));
    }

    #[test]
    fn decode_rejects_garbage_and_missing_method_shapes() {
        assert!(decode(b"not json").is_err());
        assert!(decode(b"").is_err());
        // An id without result/error still reads as a response.
        let m = decode(br#"{"jsonrpc":"2.0","id":3}"#).unwrap();
        assert!(matches!(m, Message::Response(_)));
        // No id and no method cannot be any message.
        assert!(decode(br#"{"jsonrpc":"2.0"}"#).is_err());
    }

    #[test]
    fn messages_ignore_unknown_fields() {
        let m = decode(br#"{"jsonrpc":"2.0","id":1,"method":"view/load","params":{},"trace":"x"}"#)
            .unwrap();
        let Message::Request(r) = m else {
            panic!("expected request, got {m:?}")
        };
        assert_eq!(r.method, "view/load");
    }

    #[test]
    fn response_omits_the_absent_side_on_the_wire() {
        let ok = Response {
            jsonrpc: "2.0".into(),
            id: 1,
            result: Some(json!(null)),
            error: None,
        };
        let text = serde_json::to_string(&ok).unwrap();
        assert!(text.contains("\"result\":null"), "{text}");
        assert!(!text.contains("\"error\""), "{text}");
    }

    #[test]
    fn every_view_kind_round_trips() {
        let views = vec![
            View::Table {
                title: "Deployments".into(),
                columns: vec![Column {
                    key: "status".into(),
                    label: "Status".into(),
                    mono: false,
                    align_right: false,
                }],
                rows: vec![Row {
                    id: "d1".into(),
                    cells: BTreeMap::from([
                        ("app".into(), Cell::Text("shop-api".into())),
                        ("sha".into(), Cell::Mono("abc123".into())),
                        (
                            "status".into(),
                            Cell::Badge(Badge {
                                text: "running".into(),
                                tone: Tone::Success,
                            }),
                        ),
                        ("at".into(), Cell::Time("2026-10-26T10:00:00Z".into())),
                    ]),
                    open: Some("detail/d1".into()),
                    actions: vec![sample_action()],
                    group: Some("hetzner-1".into()),
                }],
                empty: "No deployments".into(),
                groups: vec!["hetzner-1".into()],
            },
            View::Detail {
                title: "shop-api".into(),
                status: Some(Badge {
                    text: "failed".into(),
                    tone: Tone::Danger,
                }),
                fields: vec![("Branch".into(), "main".into())],
                tabs: vec![Tab {
                    id: "logs".into(),
                    title: "Logs".into(),
                    view: "logs/shop-api".into(),
                }],
                actions: vec![sample_action()],
            },
            View::LogStream {
                stream: "deployment/d1".into(),
                title: "Deployment log".into(),
            },
            View::Message {
                tone: Tone::Warning,
                text: "Built for Coolify 4.5".into(),
                action: Some(sample_action()),
            },
            View::Form {
                title: "Configure".into(),
                fields: vec![ConfigField {
                    key: "url".into(),
                    label: "URL".into(),
                    kind: FieldKind::Url,
                    required: true,
                }],
                submit: sample_action(),
            },
        ];
        for view in &views {
            round_trip(view);
        }
    }

    #[test]
    fn view_kinds_and_tones_are_snake_case_on_the_wire() {
        for (view, kind) in [
            (
                View::Table {
                    title: "t".into(),
                    columns: vec![],
                    rows: vec![],
                    empty: "e".into(),
                    groups: vec![],
                },
                "\"kind\":\"table\"",
            ),
            (
                View::LogStream {
                    stream: "s".into(),
                    title: "t".into(),
                },
                "\"kind\":\"log_stream\"",
            ),
            (
                View::Message {
                    tone: Tone::Info,
                    text: "x".into(),
                    action: None,
                },
                "\"kind\":\"message\"",
            ),
        ] {
            let text = serde_json::to_string(&view).unwrap();
            assert!(text.contains(kind), "missing {kind} in {text}");
        }
        assert_eq!(
            serde_json::to_string(&Tone::Success).unwrap(),
            "\"success\""
        );
        let cell = serde_json::to_string(&Cell::Mono("a".into())).unwrap();
        assert_eq!(cell, r#"{"kind":"mono","value":"a"}"#);
    }

    #[test]
    fn views_tolerate_omitted_optional_fields() {
        let v = json!({"kind": "detail", "title": "x"});
        let view: View = serde_json::from_value(v).unwrap();
        assert!(matches!(view, View::Detail { .. }));
        let v = json!({"kind": "message", "tone": "neutral", "text": "hi"});
        let view: View = serde_json::from_value(v).unwrap();
        assert!(matches!(view, View::Message { action: None, .. }));
        let row: Row = serde_json::from_value(json!({"id": "r1"})).unwrap();
        assert!(row.cells.is_empty() && row.open.is_none());
    }

    #[test]
    fn action_result_round_trips_and_defaults() {
        round_trip(&ActionResult {
            message: "Deploy started".into(),
            open: Some("deployment/d1".into()),
            invalidate: vec!["deployments".into()],
        });
        let r: ActionResult = serde_json::from_value(json!({"message": "done"})).unwrap();
        assert!(r.open.is_none() && r.invalidate.is_empty());
    }

    #[test]
    fn method_constants_match_plan_section_6_10() {
        assert_eq!(methods::INITIALIZE, "initialize");
        assert_eq!(methods::VIEW_LOAD, "view/load");
        assert_eq!(methods::ACTION_INVOKE, "action/invoke");
        assert_eq!(methods::LOGS_OPEN, "logs/open");
        assert_eq!(methods::LOGS_CLOSE, "logs/close");
        assert_eq!(methods::BADGES, "project/badges");
        assert_eq!(methods::SHUTDOWN, "shutdown");
        assert_eq!(methods::SECRET_GET, "host/secret.get");
        assert_eq!(methods::LOG, "host/log");
        assert_eq!(methods::LOG_LINES, "logs/lines");
        assert_eq!(methods::INVALIDATE, "view/invalidate");
        assert_eq!(methods::MCP_SERVERS, "host/mcp.servers");
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    #[test]
    fn manifest_json_schema_is_written_and_current() {
        let schema = schemars::schema_for!(Manifest);
        let mut text = serde_json::to_string_pretty(&schema).unwrap();
        text.push('\n');
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("manifest.schema.json");
        match std::fs::read_to_string(&path) {
            Ok(existing) if existing == text => {}
            _ => {
                std::fs::write(&path, &text).unwrap();
                panic!(
                    "manifest.schema.json was stale or missing and has been \
                     regenerated; commit the new file and rerun"
                );
            }
        }
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let props = parsed["properties"].as_object().unwrap();
        for field in [
            "id",
            "name",
            "version",
            "protocol",
            "executable",
            "permissions",
            "contributes",
            "config",
            "secrets",
        ] {
            assert!(props.contains_key(field), "schema lacks {field}");
        }
    }
}
