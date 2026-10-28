# Berth — handoff to the Linux environment (2026-10-28)

Prepared on the Mac so development can continue on Linux and later come
back to this Mac for the Mac-only work. Written for an agent (or person)
opening this repo cold on Linux.

## Task

Continue **Phase 8** on branch `phase/8-parity`, starting mid-task at
**P8-T3**. Work one task per session, in order, exactly as on the Mac.
The Mac-only parts (shots against frames, tray/menu, Colima, signing)
are deferred, not dropped — see "Deferred to the Mac" below.

## Read first, in this order

1. `CLAUDE.md` — the rules. Its "Environment (this Mac…)" section and
   the shot workflow are Mac-specific; this document overrides those
   parts while on Linux. Everything else still binds.
2. `docs/STATUS.md`, "Current state" — where the work stands. It is the
   resume mechanism: update it at the end of every session.
3. `docs/TASKS.md` — the Phase 8 header ("Learned" first), then the
   P8-T3 entry including its **Architect notes (AD43)**.
4. `docs/PLAN.md` — only the sections the current task cites (§6
   interfaces are implemented exactly; changing one is an escalation,
   see CLAUDE.md "Roles").
5. `docs/QUESTIONS.md` — open questions, each with a default already
   taken. Nothing there blocks Phase 8.

## Current state (verified on the Mac, 2026-10-28)

- Phases 0–7 closed; workspace version `0.7.0`, tags `v0.0`–`v0.7`
  local only. **Nothing has ever been pushed.**
- Phase 8: P8-T1 remote engines (`7a64c58`), P8-T2 registries
  (`ed7444f`) done. P8-T3 is part done, committed as a checkpoint:
  `a7a559c P8-T3: AD43 build contracts, step parser, context tar
  (checkpoint)`.
- Gate at the checkpoint: `scripts/check.sh` exit 0 —
  **2337 passed, 67 skipped** (macOS, this Mac).
- The working tree is clean except `.audit/`, the owner's untracked
  notes. They are not needed by the code and were left uncommitted on
  purpose.

### P8-T3: done in the checkpoint

- AD43 decided and recorded (PLAN D52/§6.2/§6.5, TASKS P8-T3 notes,
  STATUS "Decisions made"): builds go through bollard's engine API,
  not the `docker build` CLI. On the Mac there is no buildx and no
  `--progress` flag; on Linux the CLI may exist — **do not switch back**:
  the API path is the decision and works on local/SSH/TCP engines.
- `berth-core`: `BuildContext`, `BuildSpec`, `BuildEvent` (in
  `image.rs`, re-exported). Never log a `BuildSpec` (build args can be
  secrets).
- `berth-engine`: `EngineClient::build_image` (default = unsupported),
  one `RegistryAuth` for pull and push (`push` by reference),
  `build_parse.rs` (step/failure parser, legacy + BuildKit plain
  forms), `build_context.rs` (folder tar honouring `.dockerignore`,
  Dockerfile always included, inline-Dockerfile tar). Tests:
  `cargo nextest run -p berth-engine build` → 11 passed.
- `tag_image` already exists in both engines (since P4-T1); the trait
  comment saying otherwise is stale history, the code is the truth.

### P8-T3: still to do (the next session's work, in order)

1. `BollardEngine`: `build_image` (tar via `build_context`, buffer
   `BuildInfo.stream` text into lines, parse with `build_parse`, extra
   tags via `tag_image` after success, `docker_for(CallKind::Build)`),
   `push` (`PushImageOptionsBuilder`, map to `PullProgress` like
   `pull`), `export_image` (`CallKind::Export`), `import_image`
   (collect the load stream, return the "Loaded image: …" names),
   `commit` (`commit_container`, pause true). Builders in bollard
   0.21.1 take `::new()`; `bollard::body_full(Bytes)` is the body
   helper already used for upload.
2. `MockEngine`: the same five (build replays step events and adds the
   image, push replays layer lines, export/import round-trip a
   `manifest.json` tar, commit snapshots a container's name/image).
   Follow the existing `pull`/`tag_image` mock patterns.
3. `ImageStore` (`crates/berth-app/src/services/images.rs`):
   `BuildState` and `PushState` beside `PullState`; pending keys
   `image.build` / `image.push` on `Subject::Image`; credentials via
   `RegistriesService::credentials_for` exactly as `pull` does
   (line ~673). Activity entries for both.
4. `sheets/build_image.rs` + `sheets/mod.rs` entry (alphabetical) +
   the `Overlay::BuildImage` arm in `routes.rs::overlay` (today it
   falls through to the placeholder). Form → building (live log,
   failing step highlighted, Problems-tab link, Cancel) → done (size
   of the first tag after refresh, Push next) / failed. Entry points:
   "Build image…" and "Import image…" in the Images controls row,
   "Push…" and "Export…" in the image detail header. Export writes
   via an `rfd` save panel (a user-chosen export is not a Review
   changes write, AD15); Import reads via `rfd` open panel.
5. Assistant `images.build` in `services/tools.rs` (today a stub
   error): same `BuildSpec`, context locked inside the project folder
   like `files.read`. Tool args are in
   `berth-assistant/src/catalog.rs` (`BuildImage`).
6. `build_it` (the done-when): `crates/berth-app/tests/build_it.rs`
   following `registry_it.rs` (brings `berthfx-registry` up/down
   itself, fixture password read from `fixtures/`) plus the engine
   rig of `images_it.rs`. It builds `fixtures/folders/dockerfile-only`
   as `127.0.0.1:56500/berthfx/app:1`, pushes, exports, imports back,
   and removes the images it made. **Write it engine-agnostic** (first
   running engine) — see the live-test caveat below.
7. Done-when: `cargo nextest run -p berth-engine -p berth-app build`,
   `scripts/it.sh -p berth-app build_it`, a mutation check (break the
   parser or the first-tag logic, see the test fail, restore), STATUS
   update, commit `P8-T3: …`. The shot is deferred (below).

Then, in order: P8-T4 (Save as image — `commit` lands with P8-T3 —
and custom templates), P8-T5 (Project from Git), and Phase 9
(P9-T1…T4 are platform-free; P9-T5's logic and SSR scenes too).

## Linux environment setup

- **Rust:** rustup; `rust-toolchain.toml` pins 1.99.1 with rustfmt and
  clippy and installs on first use. Also `cargo install cargo-nextest
  cargo-insta`. `dx` (Dioxus CLI) is only needed for bundling (Phase
  10, Mac) — skip it.
- **Do not copy the Mac's `~/.cargo/config.toml`** (it points the
  rustc wrapper at `/opt/homebrew/bin/sccache`). Use your own sccache
  or none.
- **Library crates** (everything except `berth-app`) are plain Rust
  and should build first try: start with
  `cargo nextest run -p berth-core -p berth-engine -p berth-rules -p berth-logs -p berth-store -p berth-registry -p berth-assistant`.
- **`berth-app`** compiles to a plain window off macOS:
  `platform/other.rs` stubs the menu, tray, traffic lights and
  Terminal app. Building it needs the usual wry/webkit system
  libraries (on Debian/Ubuntu typically `pkg-config`,
  `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `librsvg2-dev`,
  `libayatana-appindicator3-dev`). **Verify first, don't trust this
  list:** run `cargo check -p berth-app` and install what
  `pkg-config` reports missing. Record the working package list in
  STATUS.md under "Machine facts" for the Linux box.
- **Docker:** native `dockerd` + the compose plugin; no Colima.
  Discovery reads docker contexts and `DOCKER_HOST`, so the default
  context (`unix:///var/run/docker.sock`) is the engine. Fixtures:
  `scripts/fx-up.sh` / `fx-down.sh` / `it.sh` are plain compose and
  should work unchanged; host ports 56000–56999 as on the Mac.
- **Secrets:** there is no macOS Keychain. The workspace pins
  `keyring` with the `apple-native` feature, which has no Linux
  backend. Run the app and any Keychain-touching test with
  **`BERTH_KEYRING=memory`** (registry logins then last one run).
  Do not add a Linux keyring feature on your own: that changes PLAN
  §5, which is an escalation (write it in `docs/QUESTIONS.md` with
  two options and your pick, per CLAUDE.md). Skip
  `BERTH_KEYRING_TEST=1` here; it is a Mac check.
- Useful env as on the Mac: `BERTH_MOCK=1` (mock world, no engine),
  `BERTH_DATA_DIR=$(mktemp -d)` (keep runs off real data),
  `BERTH_IT=1` (set by `it.sh`), `BERTH_THEME=light|dark`,
  `BERTH_SCENE="<frame>"` (opens a frame's state), `scripts/ssr.sh
  "<frame>"` (renders a scene to HTML without a window — the Linux
  substitute for a shot's first look).
- Node/pnpm are needed only for `scripts/build-web.sh` (terminal and
  editor bundles, already committed under `assets/web/`). Don't touch
  unless a task changes `web-src/`.

## First session on Linux — acceptance checks

Do these before starting P8-T3's remainder, and record the results in
`docs/STATUS.md`:

- [ ] `scripts/check.sh` exits 0. Fix only Linux compile issues in
      `platform/other.rs` or `cfg` gates; anything touching PLAN
      §5/§6 is a QUESTIONS entry, not a silent change.
- [ ] `scripts/fx-up.sh` brings `berthfx` up and
      `docker ps --filter label=dev.berth.fixture=1` shows the four
      containers; `scripts/fx-down.sh` leaves none.
- [ ] Run `scripts/it.sh -p berth-engine` and note what passes.
      **Known caveat:** most `berth-app` live tests hard-code the
      engine id `colima:default` (e.g. `tests/common/mod.rs`
      `settle_fixture`, the `images_it` rig) and will panic with
      "colima:default running" on Linux. Do not rewrite them all:
      list the affected tests in STATUS as a Linux-port question for
      the architect. New live tests (starting with `build_it`) must
      pick the first running engine instead of a hard-coded id, so
      they pass on both systems.
- [ ] `BERTH_MOCK=1 BERTH_KEYRING=memory cargo run -p berth-app`
      opens the window on mock data. Fonts and chrome will differ
      from the design frames; that is expected and is not judged here.

## Deferred to the Mac (do not attempt on Linux)

- `scripts/shot.sh` and every shot-vs-frame comparison (it needs
  `osascript`, `screencapture`, `sips`, a Swift helper). For UI
  tasks, the scene + SSR tests + `ssr.sh` are the evidence on Linux;
  write "shot pending Mac return" in STATUS instead of the usual
  side-by-side, and list layout differences you can see from the
  HTML. The 34 frame PNGs in `design/shots/figcraft/screens/` are
  still the spec — read them before UI work.
- Menu bar item, native menu, Dock icon, traffic lights
  (`platform/macos.rs`); "Open in Terminal app", "Reveal in Finder",
  the `open …systempreferences` path, port links via `open`.
- Colima anything: profiles, Edit resources, engine start/stop. On
  Linux those engines simply don't exist; the code must keep
  compiling and degrading as it does when the binary is absent.
- `scripts/perf.sh --app` numbers (Mac process tooling; the stage-1
  nextest perf profile runs, but numbers aren't comparable).
- Phase 10 entirely (bundle, icons, signing, notarization, DMG).
- Live harness smoke (`BERTH_HARNESS_SMOKE=1`): the four paid runs of
  AD37 are spent; model-catalog queries are free but record any use.
- The batched owner checks listed in STATUS (real first-run "Start
  default", menu-bar look, VoiceOver-adjacent items, Q6 Colima
  restart).

## Decisions already taken that bind this work

- **AD43** (above) — engine-API builds; `.dockerignore` subset
  matcher; first tag in the request, rest tagged after; the sheet's
  Problems link opens the Problems tab and the `BuildFailed` rule
  stays project/compose-scoped. AD43 was workhorse-decided: the
  advisor subagents failed outright twice on 2026-10-28 (the AD38
  precedent — decide, record an ADn, keep moving).
- Decisions are numbered **ADn** in `docs/STATUS.md` "Decisions made"
  (AD1–AD43 used; PLAN owns D1–D84 — never reuse Dn). House practice
  (owner, 2026-10-25): when a decision is needed, consult an advisor
  subagent and adopt its recommendation instead of blocking on the
  owner; record it as the next ADn. Owner-in-person items (push,
  release, Colima restarts, hand checks) still wait for the owner.
- Speed practices: max 3 parallel workers; gap-review each task
  against PLAN §6 before dispatch; screens register themselves
  (scenes/routes/providers/actions/confirm/table-tiers — insert
  alphabetically).

## Constraints (must preserve, from CLAUDE.md)

- Secrets only through `SecretStore`; fixture password only in
  `fixtures/` and `scripts/`; env values masked everywhere except
  the one-click UI reveal; never log a `BuildSpec` or a
  `GenerationPlan`.
- Nothing changes without a click; file writes to a project go
  through Review changes; destructive confirms render from one
  `EffectSummary`.
- Every stream/subprocess takes a `CancellationToken`,
  `kill_on_drop(true)`.
- Sockets only from `berth_engine::discovery`, binaries only from
  `berth_sys::SearchPath`, data paths only from `berth_sys::paths`.
- The UI never calls an engine; no work over 4 ms on the UI path.
  Text inputs uncontrolled. Only `assets/css/tokens.css` variables
  (generated — never hand-edit; regenerate with
  `python3 -I scripts/gen-tokens.py design/tokens.json`). No
  Tailwind, no hex colours in component CSS.
- `cargo clippy --workspace --all-targets -- -D warnings`;
  `unwrap`/`expect` denied in library crates; tests must fail when
  the code breaks (mutation-check before committing a test).
- Docker safety: tests and scripts touch only `dev.berth.fixture=1`
  resources (`berthfx…`), Berth's own `dev.berth.helper=1`
  containers the test created, and `berthfx-*` Compose projects from
  temp fixture copies on ports 56000–56999, removed with
  `compose down -v`. The Linux engine may be "yours", but keep the
  discipline — the same tests run again on the owner's Mac.
- No refactors outside the task's files; note wishes in STATUS.
- Git: stay on `phase/8-parity` until the phase closes; commits
  `P<N>-T<M>: <what>`. **Never push, tag remotely, or release
  without the owner's explicit approval** (nothing in this repo has
  ever been pushed; moving the work between machines is the owner's
  transfer to arrange).
- End of session: `scripts/check.sh` exit 0, run the task's
  done-when, update STATUS, commit. If a §6 type must change, a §5
  crate is needed, a done-when fails twice honestly, a rule would
  break, or the task contradicts the code: stop, write the entry in
  `docs/QUESTIONS.md` (task id, blocker, two options, your pick),
  note it in STATUS, commit, end the session.

## Gotchas carried over

- Live tests share the four `berthfx` containers: serial
  (`live-engine` nextest group), each starts with `settle_fixture()`,
  and fixtures must be created from the tree under test (Compose
  records the folder; recreating from another checkout splits the
  project in two).
- `it.sh` retries `fx-up.sh` up to 5 times (a paused `web` fails one
  health probe) — a single failed first attempt is normal.
- On the Mac, one earlier `check.sh` run for this checkpoint was
  killed mid-compile by a session end; the rerun is the 2337-pass
  result quoted above. Trust reruns, not stale logs.
- Wall-clock/perf assertions are load-sensitive; run perf on a quiet
  machine and report ranges.

## Coming back to the Mac

1. Same branch resumes; no merge needed if history stayed linear.
2. Re-run `scripts/check.sh`, the full `scripts/it.sh`, and take
   every "shot pending Mac return" deferred during the Linux period,
   reading each beside its frame PNG and closing the differences in
   STATUS.
3. Spot-check `scripts/perf.sh --app`, the Keychain round trip
   (`BERTH_KEYRING_TEST=1`), and the batched owner checks.
4. Then continue with whatever task STATUS names next, and finally
   Phase 10 (Mac-only).

## Transfer checklist (Mac side, done 2026-10-28)

- [x] P8-T3 checkpoint committed (`a7a559c`); tree clean except
      `.audit/` (owner's untracked notes — travel separately, if at
      all; the code does not need them).
- [x] `scripts/check.sh` exit 0 quoted above (includes the secret
      scan; no credentials in the repo).
- [x] Keychain items and `~/Library/Application Support/dev.berth.Berth/berth.db`
      stay on the Mac; on Linux start from a temp `BERTH_DATA_DIR`
      and redo registry logins (with `BERTH_KEYRING=memory` they are
      per-run anyway).
- [x] `target/` does not travel (2 GB+ of build output; first Linux
      build is a full one — budget the time).

