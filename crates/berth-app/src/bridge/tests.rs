//! Bridge tests: every message of PLAN 7.5 in both directions, the base64
//! used for terminal bytes, the pump that turns raw page messages into
//! events, and the link between the Rust names and the committed bundles.

use futures::channel::mpsc::unbounded;
use futures::executor::block_on;
use futures::StreamExt;
use serde_json::{json, Value};

use super::dev::echo;
use super::*;

fn wire(command: impl Into<Command>) -> Value {
    command.into().wire()
}

// --- Rust to JS: the exact shapes of the table in PLAN 7.5 ---

#[test]
fn terminal_commands_serialize_as_the_plan_table() {
    let options = TermOptions::new(13.0, Theme::Dark);
    assert_eq!(
        wire(TermCommand::Create {
            id: "t1".into(),
            options
        }),
        json!({"call": "term.create", "args": ["t1", {"fontSize": 13.0, "theme": "dark", "scrollback": 5000}]})
    );
    assert_eq!(
        wire(TermCommand::Write {
            id: "t1".into(),
            data: b"hi\r\n".to_vec()
        }),
        json!({"call": "term.write", "args": ["t1", "aGkNCg=="]})
    );
    assert_eq!(
        wire(TermCommand::Clear { id: "t1".into() }),
        json!({"call": "term.clear", "args": ["t1"]})
    );
    assert_eq!(
        wire(TermCommand::Focus { id: "t1".into() }),
        json!({"call": "term.focus", "args": ["t1"]})
    );
    assert_eq!(
        wire(TermCommand::CopySelection { id: "t1".into() }),
        json!({"call": "term.copySelection", "args": ["t1"]})
    );
    assert_eq!(
        wire(TermCommand::SetTheme(Theme::Light)),
        json!({"call": "term.setTheme", "args": ["light"]})
    );
    assert_eq!(
        wire(TermCommand::Dispose { id: "t1".into() }),
        json!({"call": "term.dispose", "args": ["t1"]})
    );
}

#[test]
fn editor_commands_serialize_as_the_plan_table() {
    let options = EditorOptions {
        lang: Lang::Dockerfile,
        value: "FROM scratch\n".into(),
        read_only: true,
        theme: Theme::Light,
    };
    assert_eq!(
        wire(EditorCommand::Create {
            id: "e1".into(),
            options
        }),
        json!({"call": "editor.create", "args": ["e1", {"lang": "dockerfile", "value": "FROM scratch\n", "readOnly": true, "theme": "light"}]})
    );
    for (lang, name) in [(Lang::Yaml, "yaml"), (Lang::Text, "text")] {
        assert_eq!(json!(lang), json!(name));
    }
    assert_eq!(
        wire(EditorCommand::SetValue {
            id: "e1".into(),
            text: "a: 1".into()
        }),
        json!({"call": "editor.setValue", "args": ["e1", "a: 1"]})
    );
    assert_eq!(
        wire(EditorCommand::GetValue { id: "e1".into() }),
        json!({"call": "editor.getValue", "args": ["e1"]})
    );
    assert_eq!(
        wire(EditorCommand::SetMarks {
            id: "e1".into(),
            marks: vec![Mark {
                line: 3,
                message: "bad indent".into()
            }]
        }),
        json!({"call": "editor.setMarks", "args": ["e1", [{"line": 3, "message": "bad indent"}]]})
    );
    assert_eq!(
        wire(EditorCommand::SetTheme(Theme::Dark)),
        json!({"call": "editor.setTheme", "args": ["dark"]})
    );
    assert_eq!(
        wire(EditorCommand::Dispose { id: "e1".into() }),
        json!({"call": "editor.dispose", "args": ["e1"]})
    );
}

#[test]
fn ui_commands_serialize_as_the_plan_table() {
    assert_eq!(
        wire(UiCommand::ScrollTo {
            element_id: "log".into(),
            top: 120.5
        }),
        json!({"call": "ui.scrollTo", "args": ["log", 120.5]})
    );
    assert_eq!(
        wire(UiCommand::Copy {
            text: "docker ps".into()
        }),
        json!({"call": "ui.copy", "args": ["docker ps"]})
    );
}

#[test]
fn a_hostile_id_stays_data() {
    // Ids reach the page as JSON values, never as script text.
    let id = "x\"); alert(1); (\"";
    let value = wire(TermCommand::Focus { id: id.into() });
    assert_eq!(value["args"][0], json!(id));
    assert_eq!(value["call"], json!("term.focus"));
}

// --- JS to Rust ---

#[test]
fn terminal_events_parse_from_the_plan_table() {
    assert_eq!(
        parse_event(r#"{"type":"term.ready","id":"t1"}"#),
        Some(Event::Term(TermEvent::Ready { id: "t1".into() }))
    );
    assert_eq!(
        parse_event(r#"{"type":"term.input","id":"t1","data":"bHMNCg=="}"#),
        Some(Event::Term(TermEvent::Input {
            id: "t1".into(),
            data: b"ls\r\n".to_vec()
        }))
    );
    assert_eq!(
        parse_event(r#"{"type":"term.resize","id":"t1","cols":120,"rows":32}"#),
        Some(Event::Term(TermEvent::Resize {
            id: "t1".into(),
            cols: 120,
            rows: 32
        }))
    );
    assert_eq!(
        parse_event(r#"{"type":"term.title","id":"t1","title":"api-1: bash"}"#),
        Some(Event::Term(TermEvent::Title {
            id: "t1".into(),
            title: "api-1: bash".into()
        }))
    );
}

#[test]
fn editor_events_parse_from_the_plan_table() {
    assert_eq!(
        parse_event(r#"{"type":"editor.ready","id":"e1"}"#),
        Some(Event::Editor(EditorEvent::Ready { id: "e1".into() }))
    );
    assert_eq!(
        parse_event(r#"{"type":"editor.change","id":"e1","text":"a: 1\n"}"#),
        Some(Event::Editor(EditorEvent::Change {
            id: "e1".into(),
            text: "a: 1\n".into()
        }))
    );
    assert_eq!(
        parse_event(r#"{"type":"editor.value","id":"e1","text":"b: 2"}"#),
        Some(Event::Editor(EditorEvent::Value {
            id: "e1".into(),
            text: "b: 2".into()
        }))
    );
    assert_eq!(
        parse_event(r#"{"type":"editor.save","id":"e1"}"#),
        Some(Event::Editor(EditorEvent::Save { id: "e1".into() }))
    );
}

#[test]
fn unknown_and_malformed_messages_are_ignored() {
    for raw in [
        r#"{"type":"term.future","id":"t1"}"#,
        r#"{"type":"editor.ready"}"#,
        r#"{"id":"t1"}"#,
        r#"{"type":"term.resize","id":"t1","cols":"wide","rows":32}"#,
        r#"{"type":"term.resize","id":"t1","cols":70000,"rows":32}"#,
        r#"{"type":"term.input","id":"t1","data":"not base64!"}"#,
        r#"{"type":"term.input","id":"t1","data":42}"#,
        r#"["term.ready"]"#,
        "not json",
        "",
    ] {
        assert_eq!(parse_event(raw), None, "{raw}");
    }
}

#[test]
fn extra_fields_are_tolerated() {
    assert_eq!(
        parse_event(r#"{"type":"term.ready","id":"t1","note":"from a newer bundle"}"#),
        Some(Event::Term(TermEvent::Ready { id: "t1".into() }))
    );
}

#[test]
fn event_ids_are_reachable_without_matching() {
    assert_eq!(TermEvent::Ready { id: "a".into() }.id(), "a");
    assert_eq!(EditorEvent::Save { id: "b".into() }.id(), "b");
}

// --- base64 ---

#[test]
fn base64_matches_the_rfc_4648_vectors() {
    for (plain, encoded) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64_encode(plain.as_bytes()), encoded);
        assert_eq!(base64_decode(encoded).as_deref(), Some(plain.as_bytes()));
    }
}

#[test]
fn base64_round_trips_every_byte_value_and_length() {
    let all: Vec<u8> = (0..=255u8).collect();
    for len in 0..all.len() {
        let slice = &all[..len];
        assert_eq!(base64_decode(&base64_encode(slice)).as_deref(), Some(slice));
    }
    // Terminal output is arbitrary bytes, UTF-8 or not.
    let utf8 = "héllo \u{1f433}\u{1b}[31m";
    assert_eq!(
        base64_decode(&base64_encode(utf8.as_bytes())).as_deref(),
        Some(utf8.as_bytes())
    );
}

#[test]
fn base64_rejects_bad_input() {
    for bad in ["Z", "Zg=", "Zg===", "Zm9v!", "Zm 9v", "Zm9v\n"] {
        assert_eq!(base64_decode(bad), None, "{bad:?}");
    }
}

// --- the pump ---

#[test]
fn pump_delivers_valid_events_in_order_and_drops_the_rest() {
    let raw = futures::stream::iter(vec![
        r#"{"type":"term.ready","id":"t1"}"#.to_string(),
        "garbage".to_string(),
        r#"{"type":"term.mystery","id":"t1"}"#.to_string(),
        r#"{"type":"term.input","id":"t1","data":"YQ=="}"#.to_string(),
        r#"{"type":"editor.save","id":"e1"}"#.to_string(),
    ]);
    let (tx, mut rx) = unbounded();
    block_on(pump(raw, tx));
    let mut seen = Vec::new();
    while let Ok(event) = rx.try_recv() {
        seen.push(event);
    }
    assert_eq!(
        seen,
        vec![
            Event::Term(TermEvent::Ready { id: "t1".into() }),
            Event::Term(TermEvent::Input {
                id: "t1".into(),
                data: b"a".to_vec()
            }),
            Event::Editor(EditorEvent::Save { id: "e1".into() }),
        ]
    );
}

#[test]
fn pump_stops_when_the_receiver_is_gone() {
    let raw = futures::stream::iter(vec![r#"{"type":"term.ready","id":"t1"}"#.to_string()]);
    let (tx, rx) = unbounded();
    drop(rx);
    block_on(pump(raw, tx)); // returns instead of looping or panicking
}

#[test]
fn receiver_sees_the_end_of_the_page() {
    let (tx, mut rx) = unbounded::<Event>();
    block_on(pump(futures::stream::iter(Vec::<String>::new()), tx));
    assert_eq!(block_on(rx.next()), None);
}

// --- the committed bundles ---

#[test]
fn every_asset_is_served_with_its_type() {
    for (file, content_type) in [
        ("terminal.js", "text/javascript"),
        ("terminal.css", "text/css"),
        ("editor.js", "text/javascript"),
        ("editor.css", "text/css"),
    ] {
        let (served, body) = web_asset(file).unwrap_or_else(|| panic!("{file} not embedded"));
        assert!(served.starts_with(content_type), "{file}: {served}");
        assert!(!body.is_empty(), "{file} is empty");
    }
    assert!(web_asset("../tokens.css").is_none());
    assert!(web_asset("").is_none());
}

/// `["create","write",...]` for the commands, as the bundles list them.
fn call_list<C: Into<Command>>(prefix: &str, commands: impl IntoIterator<Item = C>) -> String {
    let names: Vec<String> = commands
        .into_iter()
        .map(|command| {
            let name = command.into().name();
            let function = name.strip_prefix(prefix).expect("namespace prefix");
            format!("\"{function}\"")
        })
        .collect();
    format!("[{}]", names.join(","))
}

/// The Rust enums and the TypeScript must name the same functions and
/// message types. This reads the committed, minified bundles: a rename on
/// one side without a rebuild or an edit on the other fails here.
#[test]
fn bundles_define_every_function_and_message_rust_uses() {
    let (_, terminal) = web_asset("terminal.js").expect("terminal.js");
    let (_, editor) = web_asset("editor.js").expect("editor.js");

    let term_calls = [
        TermCommand::Create {
            id: String::new(),
            options: TermOptions::new(12.0, Theme::Light),
        },
        TermCommand::Write {
            id: String::new(),
            data: vec![],
        },
        TermCommand::Clear { id: String::new() },
        TermCommand::Focus { id: String::new() },
        TermCommand::CopySelection { id: String::new() },
        TermCommand::SetTheme(Theme::Dark),
        TermCommand::Dispose { id: String::new() },
    ];
    // Each bundle ends with sealApi(namespace, api, [the functions Rust calls]).
    assert!(
        terminal.contains(&call_list("term.", term_calls)),
        "terminal.js does not publish the functions TermCommand calls"
    );

    let editor_calls = [
        EditorCommand::Create {
            id: String::new(),
            options: EditorOptions {
                lang: Lang::Text,
                value: String::new(),
                read_only: false,
                theme: Theme::Light,
            },
        },
        EditorCommand::SetValue {
            id: String::new(),
            text: String::new(),
        },
        EditorCommand::GetValue { id: String::new() },
        EditorCommand::SetMarks {
            id: String::new(),
            marks: vec![],
        },
        EditorCommand::SetTheme(Theme::Dark),
        EditorCommand::Dispose { id: String::new() },
    ];
    assert!(
        editor.contains(&call_list("editor.", editor_calls)),
        "editor.js does not publish the functions EditorCommand calls"
    );

    for kind in ["term.ready", "term.input", "term.resize", "term.title"] {
        assert!(
            terminal.contains(&format!("\"{kind}\"")),
            "terminal.js never emits {kind}"
        );
    }
    for kind in [
        "editor.ready",
        "editor.change",
        "editor.value",
        "editor.save",
    ] {
        assert!(
            editor.contains(&format!("\"{kind}\"")),
            "editor.js never emits {kind}"
        );
    }

    // The ids the pages mount into are the ones Rust renders.
    assert!(terminal.contains("berth-term-"));
    assert!(editor.contains("berth-editor-"));
    assert_eq!(terminal::host_id("a"), "berth-term-a");
    assert_eq!(editor::host_id("a"), "berth-editor-a");
}

#[test]
fn the_boot_script_dispatches_to_both_bundles_and_ui() {
    assert!(BOOT_JS.contains("term: \"terminal\""));
    assert!(BOOT_JS.contains("editor: \"editor\""));
    assert!(BOOT_JS.contains("scrollTo(") && BOOT_JS.contains("copy("));
    assert!(BOOT_JS.contains("dioxus.send(JSON.stringify("));
}

#[test]
fn theme_follows_the_stored_setting() {
    assert_eq!(Theme::from(berth_store::Theme::Dark), Theme::Dark);
    assert_eq!(Theme::from(berth_store::Theme::Light), Theme::Light);
    assert_eq!(Theme::from(berth_store::Theme::System), Theme::Light);
}

// --- the dev scene's echo ---

#[test]
fn echo_shows_typed_text_and_handles_enter_and_backspace() {
    assert_eq!(echo(b"ls"), b"ls");
    assert_eq!(echo(b"\r"), b"\r\n$ ");
    assert_eq!(echo(&[0x7f]), b"\x08 \x08");
    assert_eq!(echo("é".as_bytes()), "é".as_bytes());
}
