//! The bridge to the sealed web bundles (PLAN 7.5, D9, ADR 0004).
//!
//! Rust to JS and JS to Rust travel over one `document::eval` handle per
//! [`Bridge`]: commands go out through `Eval::send` as `{"call", "args"}`
//! objects that the boot script dispatches to `window.berth.<ns>.<fn>`;
//! messages come back through `dioxus.send(JSON.stringify(msg))` and are
//! parsed into [`Event`]. Unknown or malformed messages are dropped. The
//! bundles themselves load on first use from `/web/<name>.{js,css}`
//! ([`use_web_assets`]).
//!
//! Terminal bytes and the editor's text can hold secrets (a `cat .env` in a
//! shell): nothing in this module logs a payload, only message kinds.

pub mod dev;
pub mod editor;
pub mod terminal;

use dioxus::desktop::wry::http::{header, Response, StatusCode};
use dioxus::desktop::{use_asset_handler, AssetRequest};
use dioxus::document::{self, Eval};
use dioxus::prelude::spawn;
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

pub use editor::{EditorCommand, EditorEvent, EditorOptions, Lang, Mark};
pub use terminal::{TermCommand, TermEvent, TermOptions};

// --- Shared types ---

/// The mode a bundle is asked to draw in. The colours themselves come from
/// the design tokens on `<html>`, which Rust flips first (app.rs `theme_script`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}

impl From<berth_store::Theme> for Theme {
    /// `System` has no mode of its own here: the tokens already follow the
    /// OS, so it reads as light until the bundle samples them.
    fn from(theme: berth_store::Theme) -> Self {
        match theme {
            berth_store::Theme::Dark => Theme::Dark,
            berth_store::Theme::Light | berth_store::Theme::System => Theme::Light,
        }
    }
}

/// `berth.ui.*` (PLAN 7.5): helpers the boot script itself defines.
#[derive(Debug, Clone, PartialEq)]
pub enum UiCommand {
    ScrollTo { element_id: String, top: f64 },
    Copy { text: String },
}

/// One Rust-to-JS message.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Term(TermCommand),
    Editor(EditorCommand),
    Ui(UiCommand),
}

impl From<TermCommand> for Command {
    fn from(command: TermCommand) -> Self {
        Command::Term(command)
    }
}

impl From<EditorCommand> for Command {
    fn from(command: EditorCommand) -> Self {
        Command::Editor(command)
    }
}

impl From<UiCommand> for Command {
    fn from(command: UiCommand) -> Self {
        Command::Ui(command)
    }
}

/// A call into `window.berth`: `name` is `<namespace>.<function>`.
pub(crate) struct Call {
    pub name: &'static str,
    pub args: Vec<Value>,
}

pub(crate) fn by_id(name: &'static str, id: &str) -> Call {
    Call {
        name,
        args: vec![Value::String(id.to_string())],
    }
}

impl Command {
    fn call(&self) -> Call {
        match self {
            Command::Term(command) => command.call(),
            Command::Editor(command) => command.call(),
            Command::Ui(UiCommand::ScrollTo { element_id, top }) => Call {
                name: "ui.scrollTo",
                args: vec![json!(element_id), json!(top)],
            },
            Command::Ui(UiCommand::Copy { text }) => Call {
                name: "ui.copy",
                args: vec![json!(text)],
            },
        }
    }

    /// The JSON the boot script receives from `dioxus.recv()`.
    pub fn wire(&self) -> Value {
        let call = self.call();
        json!({ "call": call.name, "args": call.args })
    }

    /// The function name only, for logs (payloads never are).
    pub fn name(&self) -> &'static str {
        self.call().name
    }
}

/// One JS-to-Rust message (PLAN 7.5). Anything that is not one of these
/// eight shapes fails to parse and is dropped by [`parse_event`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Event {
    Term(TermEvent),
    Editor(EditorEvent),
}

/// Parse one raw message from the page. Unknown types, wrong field types and
/// invalid base64 all give `None`.
pub fn parse_event(raw: &str) -> Option<Event> {
    serde_json::from_str(raw).ok()
}

// --- base64 (standard alphabet, padded; PLAN D41 sends terminal bytes this way) ---

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        // 1 byte gives 2 characters, 2 bytes give 3, 3 bytes give 4; the rest is padding.
        for (index, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if index <= chunk.len() {
                out.push(char::from(B64[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let trimmed = text.trim_end_matches('=');
    if !text.len().is_multiple_of(4) || text.len() - trimmed.len() > 2 {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for ch in trimmed.bytes() {
        let value = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

/// `deserialize_with` for the `data` field of `term.input`.
pub(crate) fn base64_bytes<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<u8>, D::Error> {
    let text = String::deserialize(deserializer)?;
    base64_decode(&text).ok_or_else(|| serde::de::Error::custom("invalid base64"))
}

// --- The live handle ---

/// A running bridge. Cheap to copy; start it once inside a Dioxus scope.
#[derive(Clone, Copy)]
pub struct Bridge {
    eval: Eval,
}

impl Bridge {
    /// Install the boot script and begin listening. Must run inside a
    /// Dioxus scope (it uses `document::eval` and `spawn`); the receiver
    /// yields every parsed event until the page goes away.
    pub fn start() -> (Bridge, UnboundedReceiver<Event>) {
        let eval = document::eval(BOOT_JS);
        let (tx, rx) = unbounded();
        let raw = futures::stream::unfold(eval, |mut eval| async move {
            let message = eval.recv::<String>().await.ok()?;
            Some((message, eval))
        });
        spawn(pump(Box::pin(raw), tx));
        (Bridge { eval }, rx)
    }

    /// Queue a command for the page. Order is preserved. A failure means the
    /// page is gone; it is logged by name only.
    pub fn send(&self, command: impl Into<Command>) {
        let command = command.into();
        if self.eval.send(command.wire()).is_err() {
            tracing::warn!(call = command.name(), "bridge send failed");
        }
    }
}

/// Parse raw messages into events until the stream ends or the receiver is dropped.
pub(crate) async fn pump(mut raw: impl Stream<Item = String> + Unpin, tx: UnboundedSender<Event>) {
    while let Some(message) = raw.next().await {
        match parse_event(&message) {
            Some(event) => {
                if tx.unbounded_send(event).is_err() {
                    return;
                }
            }
            None => tracing::debug!(len = message.len(), "bridge message ignored"),
        }
    }
}

/// Loaded once per page by [`Bridge::start`]. Installs `berth.emit` and
/// `berth.ui`, loads a bundle the first time one of its namespaces is
/// called, then runs commands in order.
pub const BOOT_JS: &str = r#"
const berth = (window.berth ??= {});
berth.emit = (message) => dioxus.send(JSON.stringify(message));
berth.ui = {
  scrollTo(elementId, top) {
    const element = document.getElementById(elementId);
    if (element) element.scrollTop = top;
  },
  copy(text) {
    navigator.clipboard.writeText(text).catch(() => {});
  },
};
const bundles = { term: "terminal", editor: "editor" };
const loading = (berth.loading ??= {});
function load(name) {
  loading[name] ??= new Promise((resolve, reject) => {
    const style = document.createElement("link");
    style.rel = "stylesheet";
    style.href = `/web/${name}.css`;
    document.head.appendChild(style);
    const script = document.createElement("script");
    script.src = `/web/${name}.js`;
    script.onload = resolve;
    script.onerror = () => reject(new Error(`bundle ${name} failed to load`));
    document.head.appendChild(script);
  });
  return loading[name];
}
while (true) {
  const command = await dioxus.recv();
  const [namespace, name] = command.call.split(".");
  try {
    if (bundles[namespace] && !berth[namespace]) await load(bundles[namespace]);
    berth[namespace][name](...command.args);
  } catch (error) {
    console.error("berth bridge", command.call, String(error));
  }
}
"#;

// --- Serving the bundles ---

const TERMINAL_JS: &str = include_str!("../../assets/web/terminal.js");
const TERMINAL_CSS: &str = include_str!("../../assets/web/terminal.css");
const EDITOR_JS: &str = include_str!("../../assets/web/editor.js");
const EDITOR_CSS: &str = include_str!("../../assets/web/editor.css");

/// The embedded file behind a `/web/<file>` request.
pub fn web_asset(file: &str) -> Option<(&'static str, &'static str)> {
    match file {
        "terminal.js" => Some(("text/javascript; charset=utf-8", TERMINAL_JS)),
        "terminal.css" => Some(("text/css; charset=utf-8", TERMINAL_CSS)),
        "editor.js" => Some(("text/javascript; charset=utf-8", EDITOR_JS)),
        "editor.css" => Some(("text/css; charset=utf-8", EDITOR_CSS)),
        _ => None,
    }
}

/// Serve `/web/<file>` from the embedded bundles. Desktop root only, next to
/// `assets::use_assets`.
pub fn use_web_assets() {
    use_asset_handler("web", |request: AssetRequest, responder| {
        let file = request.uri().path().trim_start_matches("/web/");
        let response = match web_asset(file) {
            Some((content_type, body)) => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .body(body.as_bytes().to_vec()),
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Vec::new()),
        };
        responder.respond(response.unwrap_or_else(|_| Response::new(Vec::new())));
    });
}

#[cfg(test)]
mod tests;
