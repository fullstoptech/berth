# Berth status

## Current state (2026-10-24)

Phase 0 complete (P0-T1..P0-T5, acceptance run 2026-10-24; atomic commits on `phase/0-scaffold`, nothing pushed). The app opens a 1440x900 window with the integrated title bar, token-driven shell and light/dark themes; fixtures work against the local engine.

- Current task: **P1-T4** (Phase 1 on `phase/1-engine`; from 2026-10-24 the owner runs an orchestrated swarm: `muse-wonder-full` decides and reviews, `muse-wonder` workers implement bounded tasks in isolated worktrees, commits stay atomic per reviewed task).
- Workspace version: `0.0.0`.

## What works

- Cargo workspace: eleven crates under `crates/` plus `plugins/coolify` (PLAN §6.1), each with one `compiles()` test. `berth-app` builds the `berth` binary, `plugins/coolify` the `berth-coolify` binary.
- P0-T1 done-when (2026-10-24, resumed session):

```
$ cargo nextest run --workspace
     Summary [   0.037s] 12 tests run: 12 passed, 0 skipped
$ cargo tree --workspace -d -e normal | grep -E "^(dioxus|bollard|tokio|serde|reqwest|rustls|keyring|rusqlite) v"
(nothing printed)
$ cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
(clean)
```

## Half done

Phase 1 in progress: P1-T1 and P1-T2 complete (uncommitted on `phase/1-engine`); P1-T3 next.

- P1-T1 done-when (2026-10-24): `cargo nextest run -p berth-sys` → 24 passed (fake shell supplies dirs, hanging shell killed by timeout, disabled lookup proven not to run the shell, cancel kills `sleep 30` inside 500 ms, per-stream order kept); under a Finder PATH (`env -i … PATH=/usr/bin:/bin:/usr/sbin:/sbin`) the `which` example prints `/Users/emmanuel/.local/bin/claude`, `…/muse`, `/opt/homebrew/bin/docker`, `/opt/homebrew/bin/colima`. (Cargo itself must be invoked by absolute path under `env -i`.)
- P1-T2 done-when (2026-10-24): `cargo nextest run -p berth-store` → 19 passed (≥ 18); `BERTH_KEYRING_TEST=1 … --run-ignored only keyring` → 1 passed; afterwards `security find-generic-password -s dev.berth -a test/roundtrip` exits 44 (item deleted). Activity origin keys are `engine`/`berth`/`plugin:<id>`.
- P1-T3 done-when (2026-10-24): `cargo nextest run -p berth-engine colima discovery` → 17 passed (≥ 14; NDJSON + array list forms, stopped/containerd profiles, yaml rosetta/vmType, contexts from a temp home, `DOCKER_HOST` wins); `BERTH_IT=1 … --run-ignored only colima_it` prints `profile=default vm_type=Some(Vz) rosetta=Some(false) socket=Some("…/.colima/default/docker.sock")`. Fixtures in `fixtures/colima/`. `MockColima::binary()` reports a binary (in the mock world colima exists). No colima start/stop ran.

- P0-T2 done-when (2026-10-24): `cargo nextest run -p berth-core` → 50 passed (≥ 45 required), workspace 61 passed; fmt and clippy clean. Mutation check: flipping `Exited(0)` tone to Danger failed `tone_map_matches_components_spec`, then restored.
- P0-T3 done-when (2026-10-24): gen-tokens diff clean; `grep -c "<symbol"` on `icons.svg` → 56 (no hex colours in the sprite); `css-lint.sh && secret-scan.sh` exit 0 and each exits 1 on a planted violation; `data-theme="dark"` present in tokens.css. Fonts copied with an OFL.txt built from the two fonts' own copyright lines (from their TTF name tables) plus the standard OFL 1.1 text.
- P0-T4 done-when (2026-10-24): `cargo nextest run -p berth-app scenes` → 3 passed; workspace 73 passed, fmt/clippy clean; `berth mcp` prints "not implemented yet", exit 2. Shots `p0`, `p0-light`, `p0-dark` are 2880x1800: traffic lights inside the 38 pt title bar, sidebar at 220 px, dark shot shows the dark `--color-bg`, text in IBM Plex Sans (letterforms match; font serving also unit-tested). **Differences from the "Projects / empty / 1440" frame, all planned:** main is empty (no header, empty-state art, drop zone or first-run card — P2-T6/P6); no engine picker or ⌘K chip (P2 shell); no sidebar counts or COOLIFY group (P2/P9); no bottom status bar (later shell tasks). No open layout differences for the P0 shell scope ("title bar, empty sidebar, empty main").
- P0-T5 done-when (2026-10-24): `scripts/fx-up.sh` → `berthfx-cache-1, berthfx-db-1, berthfx-logger-1, berthfx-web-1`; `curl -s 127.0.0.1:56080 | grep -c -i nginx` → 7; logger emits the scripted info/warn/JSON/error+stack cycle; `scripts/fx-down.sh` → 0 labelled containers and volumes, owner's containers untouched; `scripts/it.sh` → 73 passed; `scripts/check.sh` exits 0. CI file written (macos-14), not run (no push without approval).

## Decisions made

- **P0-T1 edition:** PLAN does not name an edition; used `2021` with resolver `2`, matching Ferry's workspace on this Mac.
- **P0-T1 dependencies per crate:** each crate declares only its internal dependencies from the §6.1 direction graph; no external crate is depended on yet, because no crate uses one at scaffold stage. All §5 externals live in `[workspace.dependencies]` for later tasks.
- **P0-T1 `rfd` feature (PLAN §5 correction needed):** §5 lists feature `tokio`, which does not exist in `rfd` 0.17.2 (verified in the crates.io registry copy on 2026-10-24; resolution of a throwaway crate fails with it). Used dioxus-desktop 0.7.10's exact spec instead: `default-features = false, features = ["xdg-portal"]`. Same crate and version; the architect should amend the §5 row.
- **P0-T1 `clippy::pedantic` in `berth-core`:** denied with `#![deny(clippy::pedantic)]` in `lib.rs`; `unwrap_used`/`expect_used` denied workspace-wide through `[workspace.lints]` (D54), allowed in tests by `clippy.toml`.
- **P0-T2 placements:** `CoreError` lives in `berth-core/src/lib.rs` (the task names no `error.rs`); `FileTarget`/`FileKind`/`FileEntry` live in `volume.rs`. `fmt::age` always ends in "ago" ("3 seconds ago", "2 min ago"); `GenerationPlan::effect` uses verb "Write" with one line per file plus the stays-in-folder line; `EffectSummary` past tense maps Write→Wrote and otherwise appends d/ed. `spec_diff` displays env as `KEY=••••` but detects value changes from the raw vectors.
- **P0-T3 details:** the compact-density block emits all three density tokens (`--size-row`, `--size-control-md`, `--space-12`), dwell timers are `--dwell-*` (so the reduced-motion block, which zeroes the `--motion-*` durations, never kills a toast timer), and `secret-scan.sh` scans only shipped code (`crates/`, `plugins/`, `web-src/`) because docs legitimately name the forbidden patterns; masked values (`PASSWORD=••••`) pass. `build-icons.py` finds the 56 `Icon/*` components in the `.figcraft` zip's `document.json` itself (`figcraft-cli info --json` has no node tree), exports each, and maps hex strokes and filled glyphs to `currentColor`.
- **P0-T4 details:** `berth-store` has only the §6.4 `Theme`/`Density`/`Settings` types so far (the Store is P1-T2); `dioxus-ssr` is a normal (not dev) dependency of `berth-app` because `scenes::render_scene_html` is library API used by the `ssr` example, which also means `berth-app` gained a `lib.rs` beside the `berth` binary; `Overlay` carries `#[allow(clippy::large_enum_variant)]` because Boxing `Confirm(ConfirmModel)` would change §6.11; `PendingKey` orders by its `Subject` debug string (`Subject` has no `Ord` in §6.2 — architect may prefer adding one); `shot.sh` finds the window with a small CoreGraphics helper (`scripts/winbounds.swift`, compiled to `target/winbounds`) and captures with `screencapture -l`, because this session's terminal lacks the Accessibility grant osascript needs (osascript stays as the fallback); tracing goes to stderr until `berth_sys::paths` lands in P1-T1.

## Pinned versions

Resolved 2026-10-24 with a throwaway crate depending on every PLAN §5 crate at the workspace requirement (the workspace itself uses none yet, so its `Cargo.lock` holds only the internal crates). Direct picks:

dioxus 0.7.10, dioxus-ssr 0.7.10, tokio 1.54.1, tokio-util 0.7.20, tokio-stream 0.1.19, futures 0.3.34, async-trait 0.1.92, serde 1.0.229, serde_json 1.0.154, serde_norway 0.9.42, toml 1.1.8+spec-1.1.0, thiserror 2.0.21, anyhow 1.0.104, tracing 0.1.44, tracing-subscriber 0.3.23, tracing-appender 0.2.5, uuid 1.29.0, chrono 0.4.45, url 2.5.8, bytesize 2.7.0, sha2 0.11.1, hex 0.4.3, directories 6.0.0, rusqlite 0.41.0, keyring 3.6.3, secrecy 0.10.3, bollard 0.21.1, regex 1.13.1, strip-ansi-escapes 0.2.1, similar 3.2.1, shell-words 1.1.1, nucleo-matcher 0.3.1, which 8.0.6, semver 1.0.28, reqwest 0.13.5, rustls 0.23.46, rmcp 3.7.0, schemars 1.2.3, flate2 1.1.10, tar 0.4.46, bytes 1.12.1, rfd 0.17.2, insta 1.49.0, tempfile 3.27.0, pretty_assertions 1.4.1, wiremock 0.6.5. (Older transitive copies of toml, thiserror, sha2 and similar also appear in that throwaway graph; they are other crates' dependencies, not Berth's picks.)

## Machine facts (confirmed on this machine, 2026-10-24)

| Fact | Value | How checked |
|---|---|---|
| OS, architecture | macOS 27.0 (26A428), arm64, Apple M3 Pro, 18 GB | `sw_vers`, `uname -m`, `sysctl` |
| Xcode | `/Applications/Xcode.app/Contents/Developer` | `xcode-select -p` |
| Rust | rustc 1.99.1, cargo 1.99.1, toolchain `stable-aarch64-apple-darwin` via rustup in `~/.cargo/bin` (not on a non-login shell's PATH; `~/.zshenv` sources `~/.cargo/env`) | `~/.cargo/bin/rustc --version` |
| Dioxus CLI | dx 0.7.10 | `dx --version` |
| Test tools | cargo-nextest 0.9.149, cargo-insta 1.49.0 | `--version` |
| Colima | 0.10.3; profile `default` Running, aarch64, 2 CPU, 8 GiB, 60 GiB, runtime docker, driver "macOS Virtualization.Framework", mount virtiofs, `rosetta: false`, `vmType: vz` | `colima version`, `colima list --json`, `colima status --json`, `~/.colima/default/colima.yaml` |
| Colima output shapes | `colima list --json` prints one JSON object per line (fields name, status, arch, cpus, memory, disk, runtime); `colima status --json` has `docker_socket`, `containerd_socket`, `driver`, `mount_type`, `kubernetes`; start flags `--cpus --memory --disk --vm-type --vz-rosetta --arch --runtime` | `colima … --json`, `colima start --help` |
| Docker | client 29.8.0 (`/opt/homebrew/bin/docker`), server 29.5.2, API 1.54; Compose 5.5.1 with `--progress json`, `config --format json`, `up --wait` | `docker version`, `docker compose … --help` |
| Docker contexts | `colima *` → `unix:///Users/emmanuel/.colima/default/docker.sock`; `default` → `unix:///var/run/docker.sock` | `docker context ls` |
| Compose labels | `com.docker.compose.project`, `.project.working_dir`, `.project.config_files`, `.service`, `.container-number`, `.oneoff`, `.depends_on`, `.config-hash`, `.version` | `docker inspect` of a running Compose container |
| Owner's containers on the engine | `server-db-1` (0.0.0.0:5432), `ferry-pg` (127.0.0.1:55432), `ferry-mysql` (127.0.0.1:53306); 41 images | `docker ps`, `docker images -q` |
| Free fixture ports | 56080, 56379, 56432, 56025, 58080, 59000 free; 5432 taken (ssh forward) | `lsof -iTCP -sTCP:LISTEN` |
| Claude Code | 2.1.309 at `~/.local/bin/claude`; flags `-p`, `--input-format stream-json`, `--output-format stream-json`, `--verbose`, `--include-partial-messages`, `--mcp-config`, `--strict-mcp-config`, `--tools`, `--allowedTools`, `--setting-sources`, `--disable-slash-commands`, `--session-id`, `--resume`, `--permission-mode`, `--permission-prompts host|none` | `claude --version`, `claude --help` |
| muse | Muse Code 1.4.12 (1.4.12-R6909.1) at `~/.local/bin/muse`; `exec` (`--json`, `--prompt-file`, `--workspace`, `--model`, `--reasoning-effort`, `--disable-shell`, `--disable-write`, `--disable-web-tools`, `--no-session-log`, `--approval-mode`), `serve` (MSP over stdio; `--disable-shell`, `--disable-write`, `--no-session-log`), `schema generate-json-schema` | `muse --help`, `muse exec --help`, `muse serve --help` |
| MSP schema | stable surface, fingerprint `sha256:cfdb6d2525601b9bac76584ccb0610ab3a5539b5e4f51141f079b99b39fcb4e0`; `session/start` params `workspaceRoot`, `modelId`, `config.mcpServers` (stdio: `command`, `args`, `env`, `framing`, `mode`); `turn/start`, `turn/interrupt`, `approval/decide`; notifications `item/delta`, `item/started`, `item/completed`, `turn/completed`, `approval/requested` | `muse schema generate-json-schema --out <scratch>` |
| Login-shell PATH | contains `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, fnm, pyenv, bun, opencode | `$SHELL -ilc 'echo $PATH'` |
| Node tooling | Node 24.20.0, pnpm 12.9.1 (corepack), npm via fnm | `--version` |
| Other tools | Python 3.14.7, git 2.54.0, gh (logged in as iambadu), jq, sqlite3 (Android platform-tools copy), Homebrew 7.0.10, figcraft-cli at `~/.local/bin` | `which`, `--version` |
| Terminal apps | Terminal.app only (`/System/Applications/Utilities`); no iTerm, Ghostty, WezTerm | `ls /Applications` |
| Editor | VS Code `code` at `/usr/local/bin/code` | `which code` |
| Screen capture | `screencapture -x` works from this session; `osascript` System Events works | probe on 2026-10-24 |
| Signing | "Apple Development: … (M8632JZV68)" and "Developer ID Application: Emmanuel Badu Sarpong (RVPL25L2BR)"; notarytool profile `ferry-notary` answers | `security find-identity -v -p codesigning`, `xcrun notarytool history` |
| Disk | 64 GiB free on the data volume | `df -h ~` |
| Registry facts | Docker Hub `/v2/repositories/library/postgres/tags` has `results[].images[].architecture` and `size`; `/v2/search/repositories/` has `repo_name`, `star_count`, `pull_count`, `is_official`; Coolify `openapi.json` at tag v4.5.0 has 212 paths including every brief §6.11 endpoint | `curl` on 2026-10-24 |
| Crate facts | dioxus-desktop 0.7.10 exports `trayicon` (tray-icon 0.21), `WindowCloseBehaviour::WindowHides`, `NativeFileHover` drag-drop paths, and re-exports `wry` 0.53 (`set_traffic_light_inset`), `tao`, `muda`; bollard 0.21.1 has `connect_with_ssh`, `connect_with_ssl`, `search_images`, `start_exec` → `StartExecResults::Attached{output,input}`, `df`, `events`, query params in `bollard::query_parameters` | source in `~/.cargo/registry` |

## Not verified (each has a task that verifies it first)

- `--allowedTools mcp__berth` letting MCP tools run without prompts under `claude -p` (P7-T3).
- How a stream-json Claude session is interrupted (P7-T3).
- MSP framing and handshake of `muse serve`, and how muse asks approval for MCP tools (P7-T4).
- A Docker Hub endpoint for Verified Publisher and Sponsored OSS badges (P4-T2).
- `docker build --progress rawjson` (P8-T3).
- `colima list --json` with several profiles, and `colima status` of a stopped profile (only one running profile exists here; P1-T3 tests both shapes from fixtures).
- Template image tags on Docker Hub (P5-T2).

## Inputs that differ from what the planning brief named

- The FigCraft file is `design/berth.figcraft` (not `design/figcraft/berth.figcraft`).
- `design/mocks/berth.json` and `design/mocks/build_mocks.py` do not exist; P1-T8 writes `fixtures/mock/berth.json` from the frames.
- `design/shots/figcraft/screens/` holds the 34 frame PNGs (exported 2026-10-24); there are no component PNGs.
