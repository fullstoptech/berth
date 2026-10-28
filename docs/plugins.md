# Writing a Berth plugin

A Berth plugin is an ordinary executable that Berth starts as a child process.
It speaks JSON-RPC 2.0 over stdio: one JSON object per line, UTF-8, each line
ended with `\n`. Any language can write one. Rust authors can use the
`berth-plugin-protocol` crate (serde only, no other Berth dependencies); its
types match this document exactly. The protocol version is `1`.

Plugins never draw UI. A plugin returns declarative `View` values and Berth
renders them with its own components, so every plugin looks like part of the
app and follows the user's theme and text size.

## The manifest

A plugin folder holds a `berth-plugin.json` manifest and the executable it
names. Installed plugins live in `<data dir>/plugins/<id>/`; Berth also looks
next to its own binary for bundled plugins.

The manifest's JSON Schema is
`crates/berth-plugin-protocol/manifest.schema.json` (generated from the Rust
types; do not edit it by hand). Unknown fields are ignored, so newer manifests
stay readable by older hosts, but an unknown permission or contribution `kind`
is an error: Berth refuses the manifest rather than silently granting or
dropping something the user never saw.

```json
{
  "id": "coolify",
  "name": "Coolify",
  "version": "1.2.0",
  "protocol": 1,
  "executable": "berth-coolify",
  "permissions": [
    { "kind": "network", "hosts": ["coolify.example.com"] },
    { "kind": "secrets", "keys": ["coolify_token"] }
  ],
  "contributes": [
    {
      "kind": "sidebar_section",
      "id": "coolify",
      "title": "Coolify",
      "items": [
        { "id": "deployments", "title": "Deployments", "view": "deployments" }
      ]
    },
    { "kind": "project_badge", "id": "coolify-badge" }
  ],
  "config": [
    { "key": "url", "label": "Instance URL", "kind": "url", "required": true }
  ],
  "secrets": [
    { "key": "coolify_token", "label": "API token" }
  ]
}
```

| Field | Meaning |
|---|---|
| `id` | Stable plugin id; also the folder name under `plugins/` |
| `name` | Display name |
| `version` | The plugin's own version string |
| `protocol` | Protocol version the plugin speaks (currently `1`) |
| `executable` | Executable path, relative to the manifest file |
| `permissions` | What the plugin asks for; the user grants these on enable |
| `contributes` | Sidebar sections, project badges, palette commands |
| `config` | Settings fields Berth renders on the Configure form |
| `secrets` | Secret values the plugin may ask for at runtime |

The list fields (`permissions`, `contributes`, `config`, `secrets`) may be
omitted when empty.

### Permissions

Permissions are declared in the manifest and shown to the user when they
enable the plugin. Berth enforces the ones a host call depends on (secret keys
and MCP, below); `network` hosts are declared, not enforced, in protocol 1.

| `kind` | Extra fields | Meaning |
|---|---|---|
| `network` | `hosts: [string]` | The plugin contacts these hosts |
| `read_engines` | — | The plugin may read engine state Berth passes it |
| `run_engine_actions` | — | The plugin may ask Berth to act on engines |
| `secrets` | `keys: [string]` | The plugin may fetch these secret keys |
| `mcp` | — | The plugin may offer MCP servers to the Assistant |

### Contributions

| `kind` | Fields | Where it appears |
|---|---|---|
| `sidebar_section` | `id`, `title`, `items: [{id, title, view}]` | A sidebar group; each item opens the named view |
| `project_badge` | `id` | A badge on local project rows; Berth asks for badge text via `project/badges` |
| `palette_commands` | `commands: [{id, title, action}]` | Command-palette entries that invoke the named action |

Berth also lists one palette command per sidebar item ("Coolify: Deployments"),
which opens that view. Plugin commands appear once the user types a query, and
only while the plugin is enabled. A `palette_commands` entry invokes `action`
with empty params, so name an action that needs none.

### Config fields and secrets

Each `config` entry is `{key, label, kind, required}` where `kind` is one of
`text`, `url`, `secret`, `bool`. Berth renders the Configure form from these
and hands the values to the plugin in `initialize`. Fields of kind `secret`
and every `secrets` declaration (`{key, label}`) are stored in the macOS
Keychain, never in plugin files or Berth's database. A plugin reads a secret
value only at runtime, through `host/secret.get`, and only for keys it
declared and the user granted.

## Messages

Every line is one of three envelopes. `jsonrpc` is always `"2.0"`.

```json
{ "jsonrpc": "2.0", "id": 1, "method": "view/load", "params": { "view": "deployments" } }
{ "jsonrpc": "2.0", "id": 1, "result": { "kind": "message", "tone": "info", "text": "…" } }
{ "jsonrpc": "2.0", "id": 1, "error": { "code": -32000, "message": "token invalid" } }
{ "jsonrpc": "2.0", "method": "logs/lines", "params": { "stream": "deployment/d1", "lines": ["…"] } }
```

- A **request** has `id` and `method`; the other side answers with one
  response carrying the same `id`.
- A **response** has `id` and either `result` or `error`
  (`{code, message}`); the absent side is omitted.
- A **notification** has `method` and no `id`; it is never answered.

Unknown fields in any object are ignored. Optional fields may be omitted
where this document says so; Berth treats a missing optional field as its
empty value.

### Host → plugin methods

| Method | Params → result |
|---|---|
| `initialize` | `{protocol, host_version, config}` → `{capabilities}`; sent once at start |
| `view/load` | `{view, params}` → a `View` |
| `action/invoke` | `{action, params}` → an `ActionResult` |
| `logs/open` | `{stream, params}` → `{}`; lines follow as `logs/lines` notifications |
| `logs/close` | `{stream}` → `{}`; stop sending lines for that stream |
| `project/badges` | `{projects: [{id, git_remote}]}` → `[{id, text, tone}]` |
| `shutdown` | `{}` → `{}`; the plugin should exit promptly |

### Plugin → host methods

| Method | Shape |
|---|---|
| `host/secret.get` | Request `{key}` → `{value}`; fails unless the key is declared in the manifest and granted by the user |
| `host/log` | Notification `{line}` (fields per host logging); written to Berth's log. Never log secret values |
| `logs/lines` | Notification `{stream, lines, final_status?}`; `final_status` marks the stream finished |
| `view/invalidate` | Notification `{view}`; Berth reloads that view if it is on screen |
| `host/mcp.servers` | Notification `[{name, url, headers}]`; needs the `mcp` permission |

An `ActionResult` is `{message, open?, invalidate?}`: `message` is shown to
the user, `open` names a view to navigate to, and `invalidate` lists views to
reload.

## Views

A `View` is tagged by `kind`. Berth renders each kind with the same components
it uses for its own screens.

### `table`

```json
{
  "kind": "table",
  "title": "Deployments",
  "columns": [
    { "key": "app", "label": "App", "mono": false, "align_right": false },
    { "key": "status", "label": "Status", "mono": false, "align_right": false }
  ],
  "rows": [
    {
      "id": "d1",
      "cells": {
        "app": { "kind": "text", "value": "shop-api" },
        "status": { "kind": "badge", "value": { "text": "running", "tone": "success" } }
      },
      "open": "detail/d1",
      "actions": [],
      "group": "hetzner-1"
    }
  ],
  "empty": "No deployments",
  "groups": ["hetzner-1"]
}
```

- `columns` fix the order and headings; `mono` uses the monospace face and
  `align_right` right-aligns the column.
- Each row's `cells` map a column key to a cell. A cell is
  `{"kind": "text"|"mono"|"time", "value": "…"}` or
  `{"kind": "badge", "value": {"text": "…", "tone": "…"}}`.
- `open` (optional) names the view a row click loads. Berth selects the row
  and shows that view in a pane beside the table, under a header built from
  the row: its first cell is the title, its first badge the status, and its
  other cells (except right-aligned columns, then any cell that is not a
  column) make one muted line.
- `actions` (optional) are the row's `ActionSpec`s, below. Plain buttons show
  on the hovered or selected row, a `danger` one always.
- A cell whose key is not a column is an extra fact about the row. A
  `repository` cell (`owner/repo`) lets Berth link a local project by its git
  remote (shown as "Linked folder …" and used by "Fix with <assistant>"); any
  other extra text cell with the same value on every row is shown as a chip
  under the title ("Server: hetzner-1").
- `group` (optional) puts the row under a group heading; `groups` fixes the
  group order. `empty` is the text shown when there are no rows.

### `detail`

`{kind, title, status?, fields?, tabs?, actions?}` — a title, an optional
status badge (`{text, tone}`), `fields` as `[label, value]` pairs, `tabs` as
`{id, title, view}` (each tab loads its view), and detail-level actions.

### `log_stream`

`{kind, stream, title}` — Berth opens the stream with `logs/open` and renders
`logs/lines` notifications in its log view until `logs/close` or a
`final_status`.

### `message`

`{kind, tone, text, action?}` — a single message (an empty state, a warning, a
"not configured yet" prompt) with an optional action button.

### `form`

`{kind, title, fields, submit}` — `fields` are config fields as in the
manifest; `submit` is the `ActionSpec` invoked with the entered values.

### Tones and actions

`Tone` is one of `neutral`, `success`, `warning`, `danger`, `info`; use
`danger` only for failures and destructive states.

An `ActionSpec` is
`{id, label, danger, params, confirm?, options?}`:

- Berth renders it as a button or menu item labelled `label`.
- `danger: true` marks a destructive action.
- `params` is free-form JSON handed back to `action/invoke` untouched. An
  action whose params are only `{ "open": "<view>" }` is not sent to the
  plugin: Berth opens that view in the pane (a "Logs" button).
- `confirm` is `{title, lines, button}`: Berth shows this confirmation before
  invoking, so name everything the action touches (app, server, branch) in
  `lines`. Destructive actions should always confirm. A line of the form
  `Label: value` (a label of at most three words) is drawn as a row
  (`Application: landing`); a line that starts `Warning: ` is drawn as an amber
  warning box; any other line is a note under the options.
- `options` are `{key, label, default}` toggles shown with the confirm (for
  example "Force rebuild"); the chosen values arrive in the invoke params.

### The `status` view

A plugin may answer a hidden view called `status` (it is in no sidebar item).
When it is a `message` of tone `warning` or `danger`, Berth shows its text as
a banner at the top of the plugin's section. Coolify uses it for the "Built for
Coolify 4.5; some actions may fail" warning when the instance is a newer major
version. An `info` answer shows nothing.

## Startup and failure behaviour

Berth starts the executable with a `PATH` that includes the user's login-shell
path, then sends `initialize`. Requests have timeouts; a plugin that stops
answering, writes a malformed line, or exits is restarted with backoff (1 s,
5 s, 30 s) and then marked in error. Writing anything to stdout that is not a
protocol line breaks the stream: log through `host/log` or stderr instead.

## A minimal plugin

The smallest useful plugin answers `initialize`, returns a `message` view,
and exits on `shutdown`. In Rust with `berth-plugin-protocol`:

```rust
use std::io::{BufRead, Write};

use berth_plugin_protocol::{decode, encode, methods, Message, Response, Tone, View};

fn main() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        let Ok(Message::Request(req)) = decode(line.as_bytes()) else {
            continue; // notifications need no answer; skip anything else
        };
        let result = match req.method.as_str() {
            methods::INITIALIZE => serde_json::json!({ "capabilities": {} }),
            methods::VIEW_LOAD => serde_json::to_value(View::Message {
                tone: Tone::Info,
                text: "Hello from a Berth plugin.".into(),
                action: None,
            })
            .unwrap_or_default(),
            methods::SHUTDOWN => return Ok(()),
            _ => serde_json::Value::Null,
        };
        let resp = Response {
            jsonrpc: "2.0".into(),
            id: req.id,
            result: Some(result),
            error: None,
        };
        stdout.write_all(&encode(&resp))?;
        stdout.flush()?;
    }
    Ok(())
}
```

Its manifest needs no permissions:

```json
{
  "id": "hello",
  "name": "Hello",
  "version": "0.1.0",
  "protocol": 1,
  "executable": "hello-plugin",
  "contributes": [
    {
      "kind": "sidebar_section",
      "id": "hello",
      "title": "Hello",
      "items": [{ "id": "main", "title": "Hello", "view": "main" }]
    }
  ]
}
```

The first-party Coolify plugin (`plugins/coolify`) is a full worked example:
sidebar section, tables and details, confirmed deploy actions, log streams,
project badges, and MCP passthrough.
