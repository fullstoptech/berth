//! Dev scene "Dev / terminal echo" (`BERTH_SCENE="Dev / terminal echo"`):
//! an xterm.js terminal whose typed keys travel to Rust through the bridge
//! and come back as output, and a CodeMirror editor below it. It exists to
//! prove both bundles load and both directions of the bridge work in the
//! real webview; no engine is involved. It is not a design frame, so it is
//! not in `scenes.rs`.

use dioxus::prelude::*;
use futures::StreamExt;

use super::editor::{self, EditorCommand, EditorEvent, EditorOptions, Lang};
use super::terminal::{self, TermCommand, TermEvent, TermOptions};
use super::{Bridge, Event, Theme};
use crate::state::AppState;

pub const FRAME: &str = "Dev / terminal echo";

const TERM_ID: &str = "echo";
const EDITOR_ID: &str = "compose";
const BANNER: &str = "Berth bridge echo: type here, Rust sends it back.\r\n$ ";
const SAMPLE: &str = "services:\n  web:\n    image: nginx:1.27\n    ports:\n      - \"56080:80\"\n";

/// True when `BERTH_SCENE` names this scene; `App` then renders [`DevEcho`]
/// in place of the shell.
pub fn active() -> bool {
    std::env::var("BERTH_SCENE").is_ok_and(|frame| frame == FRAME)
}

/// What the echo terminal writes back for typed bytes: Enter starts a new
/// prompt line, Backspace erases a cell, everything else is shown as typed.
pub fn echo(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    for &byte in input {
        match byte {
            b'\r' => out.extend_from_slice(b"\r\n$ "),
            0x7f => out.extend_from_slice(b"\x08 \x08"),
            other => out.push(other),
        }
    }
    out
}

/// Presses the keys of `keys` on xterm's input element, for shots made where
/// no real keyboard can be used (`BERTH_DEV_ECHO_KEYS`, no Accessibility
/// permission). xterm reacts as it does to the OS: keypress for text, keydown
/// for Enter. Everything after that is the real path through Rust.
fn type_script(keys: &str) -> String {
    let keys = serde_json::to_string(keys).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r##"
        const area = document.querySelector("#{host} .xterm-helper-textarea");
        if (area) {{
          area.focus();
          for (const ch of {keys}) {{
            const code = ch.charCodeAt(0);
            if (ch === "\n") {{
              area.dispatchEvent(new KeyboardEvent("keydown", {{ key: "Enter", keyCode: 13, bubbles: true }}));
            }} else {{
              area.dispatchEvent(new KeyboardEvent("keydown", {{ key: ch, bubbles: true }}));
              area.dispatchEvent(new KeyboardEvent("keypress", {{ key: ch, charCode: code, bubbles: true }}));
            }}
          }}
        }}
        "##,
        host = terminal::host_id(TERM_ID),
    )
}

#[component]
pub fn DevEcho() -> Element {
    let state = use_context::<AppState>();
    let theme = Theme::from(state.settings.peek().theme);
    let mut status = use_signal(|| "starting".to_string());

    use_hook(move || {
        let (bridge, mut events) = Bridge::start();
        bridge.send(TermCommand::Create {
            id: TERM_ID.into(),
            options: TermOptions::new(13.0, theme),
        });
        bridge.send(EditorCommand::Create {
            id: EDITOR_ID.into(),
            options: EditorOptions {
                lang: Lang::Yaml,
                value: SAMPLE.into(),
                read_only: false,
                theme,
            },
        });
        spawn(async move {
            while let Some(event) = events.next().await {
                match event {
                    Event::Term(TermEvent::Ready { id }) => {
                        bridge.send(TermCommand::Write {
                            id: id.clone(),
                            data: BANNER.as_bytes().to_vec(),
                        });
                        bridge.send(TermCommand::Focus { id });
                        status.set("terminal ready".into());
                        if let Ok(keys) = std::env::var("BERTH_DEV_ECHO_KEYS") {
                            let _ = document::eval(&type_script(&keys)).await;
                        }
                    }
                    Event::Term(TermEvent::Input { id, data }) => {
                        status.set(format!("echoed {} byte(s) through Rust", data.len()));
                        bridge.send(TermCommand::Write {
                            id,
                            data: echo(&data),
                        });
                    }
                    Event::Term(TermEvent::Resize { cols, rows, .. }) => {
                        status.set(format!("terminal {cols} x {rows}"));
                    }
                    Event::Term(TermEvent::Title { .. }) => {}
                    Event::Editor(EditorEvent::Ready { .. }) => {
                        status.set("editor ready".into());
                    }
                    Event::Editor(EditorEvent::Change { text, .. }) => {
                        status.set(format!("editor changed: {} bytes", text.len()));
                    }
                    Event::Editor(EditorEvent::Value { .. } | EditorEvent::Save { .. }) => {}
                }
            }
        });
    });

    rsx! {
        div {
            style: "display: flex; flex-direction: column; height: 100vh; background: var(--color-bg); color: var(--color-ink); font-family: var(--font-text); font-size: var(--text-body-size);",
            div {
                style: "padding: var(--space-12) var(--space-16) var(--space-12) 88px; border-bottom: 1px solid var(--color-line); display: flex; gap: var(--space-16);",
                strong { "{FRAME}" }
                span { style: "color: var(--color-ink-muted);", "{status}" }
            }
            div {
                id: terminal::host_id(TERM_ID),
                class: "berth-term",
                style: "flex: 3 1 0; min-height: 0;",
            }
            div {
                id: editor::host_id(EDITOR_ID),
                class: "berth-editor",
                style: "flex: 2 1 0; min-height: 0; border-top: 1px solid var(--color-line);",
            }
        }
    }
}
