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

// --- The toolbar gives buttons to a ⋯ menu as the pane narrows (D22) ---

fn all_tools() -> Vec<Tool> {
    Tool::ALL.to_vec()
}

#[test]
fn terminal_toolbar_keeps_every_button_when_the_pane_is_wide_or_a_window_of_its_own() {
    let (shown, moved) = toolbar_tools(None, 1.0, true, &all_tools());
    assert_eq!(shown, all_tools());
    assert!(moved.is_empty());
    // 1220 px, the whole bottom region at 1440.
    let (shown, moved) = toolbar_tools(Some(1220), 1.0, true, &all_tools());
    assert_eq!(shown, all_tools());
    assert!(moved.is_empty());
}

#[test]
fn terminal_toolbar_gives_way_in_order_as_the_pane_narrows() {
    // The 1440 split leaves the panel 732 px: Open in Terminal app goes first.
    let (shown, moved) = toolbar_tools(Some(732), 1.0, true, &all_tools());
    assert_eq!(moved, [Tool::OpenApp]);
    assert!(shown.contains(&Tool::PopOut));
    // Narrower: the menu only grows, and what is in it is always the next
    // buttons of the give-way order, none skipped.
    let mut gone = 0;
    for width in (320..732).rev().step_by(4) {
        let (_, moved) = toolbar_tools(Some(width), 1.0, true, &all_tools());
        assert!(moved.len() >= gone, "{width}: the menu only ever grows");
        gone = moved.len();
        let mut got = moved.clone();
        got.sort_by_key(|t| Tool::GIVE_WAY.iter().position(|g| g == t));
        assert_eq!(got, Tool::GIVE_WAY[..moved.len()].to_vec(), "{width} px");
    }
    assert!(gone >= 4, "a 320 px pane keeps at most Clear and Reconnect");
}

#[test]
fn terminal_toolbar_fits_its_pane_with_the_more_button_at_every_width_and_text_size() {
    for scale in crate::text_scale::STEPS {
        for width in (480..1300).step_by(7) {
            let (shown, moved) = toolbar_tools(Some(width), scale, true, &all_tools());
            if moved.is_empty() || shown.is_empty() {
                continue;
            }
            let used = (100.0 + 112.0 + 60.0 + 16.0) * scale
                + shown.iter().map(|t| t.width() * scale + 8.0).sum::<f32>()
                + 28.0 * scale
                + 8.0;
            assert!(
                used <= width as f32 + 0.5,
                "{width} px at {scale}: needs {used} for {shown:?}"
            );
        }
    }
    // A pane too narrow for even the pickers keeps no button but the ⋯.
    let (shown, moved) = toolbar_tools(Some(320), 1.5, true, &all_tools());
    assert!(shown.is_empty());
    assert_eq!(moved.len(), 6);
}

#[test]
fn terminal_toolbar_without_pickers_or_with_fewer_buttons_gives_way_later() {
    // A docker run session (no Shell, User, directory) has room for more.
    let with = toolbar_tools(Some(600), 1.0, true, &all_tools()).1.len();
    let without = toolbar_tools(Some(600), 1.0, false, &all_tools()).1.len();
    assert!(without < with, "{without} vs {with}");
    // A popped-out window has no Split and Pop out to give.
    let tools = [Tool::Reconnect, Tool::Clear, Tool::Copy, Tool::OpenApp];
    let (_, moved) = toolbar_tools(Some(700), 1.0, true, &tools);
    assert!(!moved.contains(&Tool::Split) && !moved.contains(&Tool::PopOut));
}

#[test]
fn terminal_split_scene_collapses_the_lower_priority_buttons_into_a_menu() {
    let html = scene_html("Containers / inspector bottom / 1440");
    assert!(
        html.contains("aria-label=\"More terminal actions\""),
        "{html}"
    );
    // Pop out still fits at 732 px; Open in Terminal app does not.
    assert!(html.contains("Pop out"), "{html}");
    assert!(!html.contains(">Open in Terminal app<"), "{html}");
    // With the whole region the toolbar keeps every button.
    let wide = scene_html("Container / terminal / 1440");
    assert!(!wide.contains("More terminal actions"), "{wide}");
    assert!(wide.contains("Open in Terminal app"));
}
