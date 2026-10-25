# ADR 0004: xterm.js and CodeMirror as sealed, prebuilt bundles

Status: accepted (architect, 2026-10-24; PLAN §4 D9, §7.5)

## Context

Berth needs a terminal emulator for `docker exec` sessions (brief §6.16) and a code editor for Compose files and Dockerfiles (brief §1.1 "Project from editor", Build image). Writing either is months of work. xterm.js 6.0.0 and CodeMirror 6 are the standard choices and run in the webview. Ferry sealed CodeMirror the same way and shipped it (Ferry ADR 0005); its Mac session found that pnpm 12 needs `approve-builds esbuild` and refuses packages younger than 24 hours.

## Decision

- `web-src/` holds the TypeScript sources and a `package.json` pinning `@xterm/xterm` 6.0.0, `@xterm/addon-fit` 0.11.0, `@xterm/addon-search` 0.16.0, `@xterm/addon-web-links` 0.12.0, `codemirror` 6.0.2, `@codemirror/lang-yaml` 6.1.3 and `esbuild` 0.28.2 (npm, 2026-10-24).
- `scripts/build-web.sh` bundles them into `crates/berth-app/assets/web/{terminal,editor}.{js,css}`, which are committed. Only that script needs Node.
- The bundles expose `window.berth.term` and `window.berth.editor` and talk to Rust only through the messages in PLAN §7.5. Rust owns all state; the bundles hold only screen contents and text.
- Terminal output travels as base64 chunks batched per animation frame.

## Consequences

- Sessions and CI build the app without Node.
- Generated bundles diff badly; review `web-src/` instead.
- A bridge message change updates PLAN §7.5, the TypeScript and the Rust enum together.
