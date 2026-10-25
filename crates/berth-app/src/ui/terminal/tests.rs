use super::*;
use crate::bridge::{base64_decode, Command};

#[test]
fn terminal_output_survives_the_trip_to_the_page_as_base64() {
    // Raw bytes: a split multibyte character, an escape sequence and a NUL.
    let mut original = Vec::new();
    original.extend_from_slice("caf\u{e9} \u{1f40b}".as_bytes());
    original.extend_from_slice(b"\x1b[31mred\x1b[0m\x00\xff\xfe\r\n");
    original.extend(std::iter::repeat_n(b'x', WRITE_CHUNK * 2 + 17));

    let paints = coalesce(vec![
        ViewMsg::Output(original[..5].to_vec()),
        ViewMsg::Output(original[5..].to_vec()),
    ]);
    // Every write is one message to the page; decoding them in order gives
    // the original bytes back.
    let mut received = Vec::new();
    for paint in &paints {
        let Paint::Write(data) = paint else {
            panic!("only writes expected");
        };
        assert!(data.len() <= WRITE_CHUNK);
        let wire = Command::Term(TermCommand::Write {
            id: "tab.0".to_string(),
            data: data.clone(),
        })
        .wire();
        assert_eq!(wire["call"], "term.write");
        let text = wire["args"][1].as_str().expect("base64 text");
        received.extend(base64_decode(text).expect("valid base64"));
    }
    assert_eq!(received, original);
    assert_eq!(paints.len(), 3, "chunks of at most {WRITE_CHUNK} bytes");
}

#[test]
fn terminal_clear_drops_earlier_output_and_a_view_change_keeps_its_place() {
    let view = Box::new(crate::services::exec::PaneView {
        title: "t".to_string(),
        status: Status::Live,
        spec: crate::services::exec::Spec {
            engine: berth_core::EngineId("e".to_string()),
            project: None,
            kind: Kind::Docker { args: vec![] },
        },
        session: None,
        shell_used: None,
        generation: 1,
    });
    let paints = coalesce(vec![
        ViewMsg::Output(b"old".to_vec()),
        ViewMsg::Clear,
        ViewMsg::Output(b"new".to_vec()),
        ViewMsg::Snapshot(view.clone()),
        ViewMsg::Output(b"tail".to_vec()),
    ]);
    assert_eq!(
        paints,
        vec![
            Paint::Clear,
            Paint::Write(b"new".to_vec()),
            Paint::View(view),
            Paint::Write(b"tail".to_vec()),
        ]
    );
}

#[test]
fn terminal_keeps_typing_but_lets_command_chords_through() {
    // J, K, Space, Backspace and Escape belong to the shell inside xterm.
    for key in ["j", "k", " ", "Backspace", "Escape", "t", "F8"] {
        assert!(pane_keeps_key(key, false, false), "{key}");
    }
    // Control keys are the shell's own (^C, ^D), except the panel toggle.
    assert!(pane_keeps_key("c", false, true));
    assert!(
        !pane_keeps_key("`", false, true),
        "\u{2303}` toggles the panel"
    );
    // \u{2318}T, \u{2318}K and friends reach Berth.
    assert!(!pane_keeps_key("t", true, false));
    assert!(!pane_keeps_key("k", true, false));
}

#[test]
fn terminal_shown_tab_is_the_chosen_one_else_the_first() {
    let make = |title: &str| crate::state::TerminalTab {
        id: Uuid::now_v7(),
        title: title.to_string(),
        session: None,
        exited: false,
        unread: false,
    };
    let tabs = vec![make("a"), make("b")];
    assert_eq!(effective_active(&tabs, Some(tabs[1].id)), Some(tabs[1].id));
    assert_eq!(effective_active(&tabs, None), Some(tabs[0].id));
    assert_eq!(
        effective_active(&tabs, Some(Uuid::now_v7())),
        Some(tabs[0].id)
    );
    assert_eq!(effective_active(&[], None), None);
}

fn scene_html(frame: &str) -> String {
    crate::scenes::render_scene_html(frame).expect("scene renders")
}

#[test]
fn terminal_scene_shows_the_tabs_the_toolbar_and_a_host_for_xterm() {
    let html = scene_html("Container / terminal / 1440");
    for text in [
        "shop-api/api-1 \u{b7} bash",
        "shop-api/redis-1 \u{b7} sh",
        "shop-api/worker-1 \u{b7} Exited",
        "class=\"term-dot\"",
        "Reconnect",
        "Clear",
        "Copy",
        "Split",
        "Pop out",
        "Open in Terminal app",
        "Shell",
        "User",
        "Close shop-api/api-1 \u{b7} bash",
        "berth-term",
    ] {
        assert!(html.contains(text), "{text}: {html}");
    }
    // Only the shown tab has a host; the others stay in the service.
    assert_eq!(html.matches("class=\"berth-term\"").count(), 1, "{html}");
    assert!(!html.contains("This image has no shell"));
}

#[test]
fn terminal_scene_without_a_shell_says_so_and_offers_the_files() {
    let html = scene_html("Container / terminal / no shell / 1440");
    for text in [
        "This image has no shell",
        "gcr.io/distroless/static-debian12 is built without bash or sh. Browse its files instead.",
        "Files",
        "Open a debug shell",
        "metrics-exporter-1",
    ] {
        assert!(html.contains(text), "{text}: {html}");
    }
    assert!(!html.contains("class=\"term-dot\""));
}

#[path = "tabs_tests.rs"]
mod tabs_tests;

#[path = "launcher_tests.rs"]
mod launcher_tests;
