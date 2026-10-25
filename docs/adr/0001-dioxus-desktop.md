# ADR 0001: Dioxus 0.7 desktop for the UI

Status: accepted (architect, 2026-10-24; PLAN §4 D1, D5, D11, D22)

## Context

Berth is a dense desktop tool for one developer on an Apple Silicon Mac. The owner chose Rust and said "i prefer dioxus" (`.eng/notes.md`). The UI must host a terminal emulator and a code editor, render 1,000-row tables and stream 10,000 log lines a second (brief §1.3). Dioxus 0.7.10 is the current stable release on crates.io (2026-10-24); 0.8 is alpha. The owner's sibling app Ferry shipped 1.0 on this Mac with Dioxus 0.7.10 desktop.

Alternatives: the Dioxus native renderer (Blitz), which cannot host xterm.js or CodeMirror and is younger; Tauri with a JS front end, which breaks the all-Rust rule; GPUI, which has no terminal or editor components to reuse.

## Decision

- D1: Dioxus 0.7 desktop. Logic is native Rust; rendering is the system webview (WKWebView). `dx bundle` makes the app bundle.
- D5: CSS, fonts, icons and web bundles are embedded with `include_str!`/`include_bytes!`; fonts and icons are served through `use_asset_handler` (Ferry P0-T3 pattern).
- D11: dioxus-desktop's tokio runtime; services use `tokio::spawn`; nothing over 4 ms on the UI path.
- D22: signals in one `AppState` context; no router.
- Nothing outside `berth-app` depends on Dioxus (PLAN §6.1).

## Consequences

- One language for the product; the only non-Rust runtime code is the sealed web bundles (ADR 0004).
- Performance depends on keeping the DOM small: virtual lists and Rust-side log buffers are mandatory (ADR 0011).
- Dioxus quirks Ferry found are rules here: uncontrolled inputs (D59), `data-theme` on the document element (D7), an Edit menu on macOS (D39).
- A Dioxus upgrade needs an ADR.
