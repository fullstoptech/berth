# .eng/stack.md — the project's stack, written once

Date checked: 2026-10-24 (planned stack; no code yet, versions pinned at scaffold)

## Product surfaces
- Platforms: macOS desktop (Apple Silicon) first, Linux second
- Repo shape: Cargo workspace: `crates/core` (engine clients, detection, recipes, problems), `crates/app` (Dioxus UI), `crates/plugin-protocol`, `plugins/coolify`

## Front end
- UI framework: Dioxus 0.7, desktop renderer (system webview) — chosen by Emmanuel 2026-10-24
- Styling: CSS variables generated from `design/tokens.json`
- Embedded web components: xterm.js (terminal), CodeMirror 6 (Compose/YAML editing and diffs)
- UI code: `crates/app/src/ui` (components), `crates/app/src/screens`
- Server state: engine data held in Dioxus signals fed by async tasks; mocks behind `trait EngineClient`

## Back end (in-process)
- Language: Rust (stable)
- Docker API: `bollard` against each engine socket
- Colima: `colima` CLI subprocess
- AI harnesses: subprocess (`claude -p --output-format stream-json`, `muse exec`)
- Plugins: separate processes, JSON-RPC over stdio, manifest with permissions
- Secrets: macOS Keychain
- Local storage: SQLite for pinned projects, settings and history

## Quality
- Lint: clippy (pedantic on core), rustfmt
- Tests: cargo test; UI checks against mock data
- Package manager: Cargo

## Defaults taken (override in one word)
- Desktop webview renderer over Blitz native: mature, hosts xterm.js and CodeMirror
- Plugins out of process over WASM: any language, crash isolation
