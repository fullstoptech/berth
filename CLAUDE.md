# Berth — agent instructions

Berth is a desktop Docker manager for Colima on macOS, written in Rust with Dioxus 0.7 desktop. A container is known by the folder it came from.

## Start of every session

1. Read `docs/STATUS.md`, then the current phase header (its "Learned" paragraph first) and your task in `docs/TASKS.md`, then the sections of `docs/PLAN.md` the task cites.
2. For a UI task, read the frame PNG in `design/shots/figcraft/screens/` before writing any code.
3. Run `. "$HOME/.cargo/env"` if `cargo` is not on PATH, then `scripts/check.sh`. It must exit 0 before you change anything (from P0-T5 on; before that run `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo nextest run --workspace`).

## Roles

- **Opus = architect.** Opens and closes phases, edits PLAN.md, TASKS.md and the ADRs, answers `docs/QUESTIONS.md`, runs the review gate (PLAN §3), signs off UI shots.
- **Sonnet = workhorse.** One task from TASKS.md per session, in order. Implements PLAN §6 interfaces exactly.

If you are the workhorse and any of these is true, stop, write an entry in `docs/QUESTIONS.md` (task id, blocker, two options, your pick), note it in STATUS.md, commit, and end the session:

1. A type or trait in PLAN §6 must change, or a new cross-crate public type is needed.
2. A crate outside PLAN §5 is needed, or a different major version.
3. The done-when check fails after two different honest attempts.
4. A rule below would have to be broken.
5. The task text contradicts existing code.

Everything else you decide yourself and list under "Decisions made" in STATUS.md.

## Rules

- **Secrets.** Passwords, tokens and keys go through `berth_store::SecretStore` (Keychain service `dev.berth`). Never write one to SQLite, logs, tracing fields, plugin configs, fixtures or committed tests. The fixture password `berthfx-dev` appears only in `fixtures/` and `scripts/`. `.env` values and container env values are masked everywhere except the one-click reveal in the UI; they never reach a harness, a plugin or the log file.
- **Nothing changes without a click.** Every file write goes through Review changes (`berth_rules::write::apply`, ADR 0012). Harness tools of risk Change need Approve; Destructive need the name typed. Watch mode never applies a change. Destructive confirms render their button, text and result from one `EffectSummary`.
- **Cancellation.** Every stream and subprocess takes a `CancellationToken`; subprocesses use `kill_on_drop(true)`.
- **Paths and binaries.** Engine sockets come only from `berth_engine::discovery`. Binaries come only from `berth_sys::SearchPath`. Data paths come only from `berth_sys::paths`. Never hardcode `~/.colima/...`, `/opt/homebrew/bin/...` or `~/.local/bin/...` elsewhere.
- **The UI never calls an engine.** UI code reads signals; services own I/O (D23). No work over 4 ms on the UI path.
- **Dioxus rules (Ferry lessons).** Text inputs are uncontrolled (`ui::TextField` with `initial_value`); never `value: "{signal}"` on an input. Theme and density attributes go on `document.documentElement`. A lone keyed element that must remount is wrapped in a one-item `for` loop. Clipboard through `document::eval` + `navigator.clipboard`. No file named `tailwind.css` anywhere in the app crate; no Tailwind.
- **Styling.** Components use only the variables in `assets/css/tokens.css` (generated, never edited by hand). No hex colours in component CSS.
- **Lints.** `cargo clippy --workspace --all-targets -- -D warnings`. `unwrap`/`expect` are denied in library crates (fine in tests). `clippy::pedantic` in `berth-core`.
- **Tests fail when the code breaks.** Before committing a test, break the code it covers and see it fail. Assert behaviour, not implementation.
- **Docker safety.** Tests and scripts touch only resources labelled `dev.berth.fixture=1` (names `berthfx…`). Never stop, remove or prune anything else on the engine. Never start, stop, restart or resize a Colima profile unless the task says so and the owner is present.
- **No refactors outside the task's files.** Note the wish in STATUS.md instead.
- **UI matches the frame.** Read the frame PNG before; add the scene to `scenes.rs`; finish with `scripts/shot.sh <name> --scene "<frame>"` and read the shot beside the PNG. List every difference in STATUS.md as fixed or as a question.
- **Logging.** `bollard`, `hyper`, `reqwest` and `rmcp` stay at `warn`; never log env values, prompts or tokens.
- **Git.** Branch `phase/N-name`, commits `P<N>-T<M>: <what>`. Never push, tag remotely, create a release, set repository secrets or run `release.yml` without the owner's explicit approval.

## Before you finish a session

1. `scripts/check.sh` exits 0.
2. Run the task's done-when check. For a UI task this includes the shot and the side-by-side read. Paste the output into `docs/STATUS.md`.
3. Update `docs/STATUS.md`: what works, what is half done, the exact next task id, decisions made, pinned versions, open differences from the frame.
4. Commit on the phase branch: `P<N>-T<M>: <what>`.

## Environment (this Mac, confirmed 2026-10-24)

- macOS 27.0 (26A428), Apple M3 Pro, arm64. Xcode at `/Applications/Xcode.app`. The only build and release host.
- Rust: rustup in `~/.cargo/bin` (`. "$HOME/.cargo/env"` in non-login shells), rustc 1.99.1 (pinned by `rust-toolchain.toml`), dx 0.7.10, cargo-nextest 0.9.149, cargo-insta 1.49.0.
- Docker: Colima 0.10.3, profile `default` running (aarch64, vz, 2 CPU, 8 GiB, 60 GiB, runtime docker, rosetta false, virtiofs). Socket `~/.colima/default/docker.sock`; docker context `colima` is current. Docker client 29.8.0 at `/opt/homebrew/bin/docker`, server 29.5.2 (API 1.54), Compose 5.5.1.
- The engine also runs the owner's own containers (`server-db-1` on host port 5432, `ferry-pg` on 55432, `ferry-mysql` on 53306). Do not touch them. Berth fixtures use host ports 56000–56999.
- Harnesses: `claude` (Claude Code 2.1.309) and `muse` (Muse Code 1.4.12) in `~/.local/bin`. A Finder-launched app gets `PATH=/usr/bin:/bin:/usr/sbin:/sbin`; `SearchPath` adds the login shell's PATH. Live harness tests spend the owner's subscription: run them only with `BERTH_HARNESS_SMOKE=1` and only in tasks that change an adapter.
- Node 24.20.0 and pnpm 12.9.1 (through fnm) for `scripts/build-web.sh` only. pnpm 12 needs `pnpm approve-builds esbuild` once and refuses packages published less than 24 h ago. Python 3.14.7 (`python3 -I` for scripts). `sqlite3` on PATH is the Android platform-tools copy (works). `gh` is logged in.
- Seeing the app: `scripts/shot.sh <name> [--scene "<frame>"]` (osascript window bounds + `screencapture -x -R`, 2x PNG plus a `-1x` copy); this session's terminal has Screen Recording and Accessibility permission. The computer-use screenshot tool is the alternative. `BERTH_MOCK=1` runs on mock data; `BERTH_SCENE="<frame>"` opens a frame's state; `BERTH_DATA_DIR=$(mktemp -d)` keeps runs off the real data.
- Design: `design/berth.figcraft` (page "Screens", 34 frames such as "Containers / selected / 1440"; page "Components"). PNGs of every frame in `design/shots/figcraft/screens/` ("Containers _ selected _ 1440.png"). Export a fresh frame or component with `figcraft-cli export design/berth.figcraft -o <out.png> --page Screens --node "<frame name>"`; list frames with `figcraft-cli info design/berth.figcraft`. IBM Plex Sans is not installed system-wide, so exports draw it in a fallback face.
- Data and logs: `~/Library/Application Support/dev.berth.Berth/berth.db`, `~/Library/Logs/Berth/berth.log`. Keychain items: `security find-generic-password -s dev.berth`.
- Terminal apps installed: Terminal.app only. Editor: VS Code (`code` in `/usr/local/bin`).
- Signing: "Developer ID Application: Emmanuel Badu Sarpong (RVPL25L2BR)" is in the keychain and the notarytool profile `ferry-notary` answers (2026-10-24). Use them only in P10-T5 with the owner's go-ahead (Q4).
