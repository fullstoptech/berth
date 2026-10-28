//! Conformance fixture plugin for `berth-plugin-host` (P9-T2).
//!
//! Speaks the plugin protocol on stdio using only
//! `berth-plugin-protocol`, the way a third-party plugin would. Behaviour
//! is driven by the `config` object in the `initialize` params:
//!
//! - `crash_on_init: bool` — exit(3) before answering `initialize`.
//! - `starts_log: string` — append one line per `initialize`.
//! - `shutdown_log: string` — append one line on `shutdown`.
//! - `counter_path: string` — used by the `crash-once` action.
//!
//! Actions: `echo-secret` (params `{key}`: asks the host for the secret
//! and reports `got:<value>` or `error:<message>`), `crash` (exit 4),
//! `crash-once` (exit 4 the first time, then answers), anything else
//! answers "did <action>" and emits `view/invalidate`. View `slow`
//! sleeps 30 s before answering (the host's timeout test kills us).

use std::collections::VecDeque;
use std::io::{BufRead, Write};

use berth_plugin_protocol::{
    decode, encode, methods, ActionResult, Column, Message, Notification, Request, Response, Row,
    RpcError, View,
};
use serde_json::{json, Value};

fn send(out: &mut (impl Write + ?Sized), msg: &impl serde::Serialize) {
    let bytes = encode(msg);
    if out.write_all(&bytes).is_err() || out.flush().is_err() {
        std::process::exit(2);
    }
}

fn ok(id: u64, result: Value) -> Response {
    Response {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

fn err(id: u64, code: i64, message: &str) -> Response {
    Response {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(RpcError {
            code,
            message: message.into(),
        }),
    }
}

fn notify(method: &str, params: Value) -> Notification {
    Notification {
        jsonrpc: "2.0".into(),
        method: method.into(),
        params,
    }
}

fn append_line(path: &str, line: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

struct Plugin {
    config: Value,
    next_host_id: u64,
    stash: VecDeque<Message>,
}

impl Plugin {
    fn config_str(&self, key: &str) -> Option<&str> {
        self.config.get(key).and_then(Value::as_str)
    }

    /// Read messages until the host answers our request `id`; anything
    /// else is stashed for the main loop.
    fn await_response(
        &mut self,
        lines: &mut impl Iterator<Item = std::io::Result<String>>,
        id: u64,
    ) -> Option<Response> {
        loop {
            if let Some(pos) = self
                .stash
                .iter()
                .position(|m| matches!(m, Message::Response(r) if r.id == id))
            {
                if let Some(Message::Response(response)) = self.stash.remove(pos) {
                    return Some(response);
                }
            }
            match lines.next() {
                Some(Ok(line)) => {
                    if let Ok(msg) = decode(line.as_bytes()) {
                        self.stash.push_back(msg);
                    }
                }
                _ => return None,
            }
        }
    }

    fn handle(
        &mut self,
        request: &Request,
        lines: &mut impl Iterator<Item = std::io::Result<String>>,
        out: &mut impl Write,
    ) -> bool {
        match request.method.as_str() {
            methods::INITIALIZE => {
                self.config = request.params.get("config").cloned().unwrap_or(Value::Null);
                if self.config.get("crash_on_init").and_then(Value::as_bool) == Some(true) {
                    std::process::exit(3);
                }
                if let Some(path) = self.config_str("starts_log") {
                    append_line(path, "init");
                }
                send(
                    out,
                    &notify(
                        methods::LOG,
                        json!({"level": "info", "message": "fixture ready"}),
                    ),
                );
                send(
                    out,
                    &ok(
                        request.id,
                        json!({"capabilities": ["views", "actions", "logs"]}),
                    ),
                );
            }
            methods::VIEW_LOAD => {
                let view = request
                    .params
                    .get("view")
                    .and_then(Value::as_str)
                    .unwrap_or("main");
                if view == "slow" {
                    std::thread::sleep(std::time::Duration::from_secs(30));
                }
                let table = View::Table {
                    title: format!("Fixture {view}"),
                    columns: vec![Column {
                        key: "name".into(),
                        label: "Name".into(),
                        mono: false,
                        align_right: false,
                    }],
                    rows: vec![Row {
                        id: "one".into(),
                        cells: [(
                            "name".into(),
                            berth_plugin_protocol::Cell::Text("alpha".into()),
                        )]
                        .into_iter()
                        .collect(),
                        open: None,
                        actions: Vec::new(),
                        group: None,
                    }],
                    empty: "Nothing here".into(),
                    groups: Vec::new(),
                };
                send(
                    out,
                    &ok(
                        request.id,
                        serde_json::to_value(table).unwrap_or(Value::Null),
                    ),
                );
            }
            methods::ACTION_INVOKE => {
                let action = request
                    .params
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.action(&action, request, lines, out);
            }
            methods::LOGS_OPEN => {
                let stream = request
                    .params
                    .get("stream")
                    .and_then(Value::as_str)
                    .unwrap_or("s")
                    .to_string();
                send(out, &ok(request.id, json!({})));
                send(
                    out,
                    &notify(
                        methods::LOG_LINES,
                        json!({"stream": stream, "lines": ["line one", "line two"]}),
                    ),
                );
                send(
                    out,
                    &notify(
                        methods::LOG_LINES,
                        json!({"stream": stream, "lines": [], "final_status": "finished"}),
                    ),
                );
            }
            methods::LOGS_CLOSE => send(out, &ok(request.id, json!({}))),
            methods::BADGES => {
                let projects = request
                    .params
                    .get("projects")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let badges: Vec<Value> = projects
                    .iter()
                    .map(|p| {
                        json!({
                            "id": p.get("id").cloned().unwrap_or(Value::Null),
                            "text": "Fixture: ok",
                            "tone": "success",
                        })
                    })
                    .collect();
                send(out, &ok(request.id, Value::Array(badges)));
            }
            methods::SHUTDOWN => {
                if let Some(path) = self.config_str("shutdown_log") {
                    append_line(path, "shutdown");
                }
                send(out, &ok(request.id, json!({})));
                return false;
            }
            other => {
                send(out, &err(request.id, -32601, &format!("no method {other}")));
            }
        }
        true
    }

    fn action(
        &mut self,
        action: &str,
        request: &Request,
        lines: &mut impl Iterator<Item = std::io::Result<String>>,
        out: &mut impl Write,
    ) {
        let answer = |out: &mut dyn Write, message: String| {
            let result = ActionResult {
                message,
                open: None,
                invalidate: vec!["main".into()],
            };
            send(
                out,
                &ok(
                    request.id,
                    serde_json::to_value(result).unwrap_or(Value::Null),
                ),
            );
        };
        match action {
            "crash" => std::process::exit(4),
            "crash-once" => {
                let path = self.config_str("counter_path").unwrap_or("").to_string();
                if std::fs::metadata(&path).is_err() {
                    let _ = std::fs::write(&path, "1");
                    std::process::exit(4);
                }
                answer(out, "survived".into());
            }
            "echo-secret" => {
                let key = request
                    .params
                    .get("params")
                    .and_then(|p| p.get("key"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.next_host_id += 1;
                let id = self.next_host_id;
                send(
                    out,
                    &Request {
                        jsonrpc: "2.0".into(),
                        id,
                        method: methods::SECRET_GET.into(),
                        params: json!({"key": key}),
                    },
                );
                let message = match self.await_response(lines, id) {
                    Some(response) => {
                        if let Some(error) = response.error {
                            format!("error:{}", error.message)
                        } else {
                            let value = response
                                .result
                                .as_ref()
                                .and_then(|r| r.get("value"))
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            format!("got:{value}")
                        }
                    }
                    None => "error:host went away".into(),
                };
                answer(out, message);
            }
            other => {
                answer(out, format!("did {other}"));
                send(out, &notify(methods::INVALIDATE, json!({"view": "main"})));
            }
        }
    }
}

fn main() {
    let stdin = std::io::stdin();
    let mut lines = std::io::BufReader::new(stdin).lines();
    let mut out = std::io::BufWriter::new(std::io::stdout());
    let mut plugin = Plugin {
        config: Value::Null,
        next_host_id: 9000,
        stash: VecDeque::new(),
    };
    loop {
        let message = match plugin.stash.pop_front() {
            Some(message) => Some(message),
            None => match lines.next() {
                Some(Ok(line)) => decode(line.as_bytes()).ok(),
                _ => None,
            },
        };
        match message {
            Some(Message::Request(request)) => {
                if !plugin.handle(&request, &mut lines, &mut out) {
                    return;
                }
            }
            Some(_) => {}
            None => return,
        }
    }
}
