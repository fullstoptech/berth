# Berth — handoff back to the Mac (2026-10-28)

Written on the Linux box at the end of the Linux period, for an agent (or
person) opening this repo cold on the owner's Mac. It mirrors
`docs/HANDOFF-LINUX.md`, which sent the work the other way.

## Task

Phases 8 and 9 were built and closed on Linux (AD47, AD53). Everything that
needs the Mac was deferred, not dropped: the hand acceptances, every UI shot,
three Colima-CLI live tests, the Keychain, the tags. Do those first, in the
order below, then start **Phase 10** (Mac-only). One task per session and the
CLAUDE.md rules still apply; its "Environment (this Mac…)" section is valid
again.

## Status after the Mac run of 2026-10-28

This file was corrected and its checklist re-marked after the first Mac
acceptance run (branch `phase/9-acceptance`; evidence and AD54-AD60 in
`docs/STATUS.md`, "Mac acceptance" under "What works"). Items 1, 2 and 5 are
done, item 3 is half done (terminal legs only), items 4, 6, 7 and 8 are still
owed, and Phase 10 (item 9) has not started. Three instructions in the first
version were wrong and are fixed below: the bare git fixture does not exist
until `scripts/fx-git.sh` runs; the copy under `$HOME` is needed because the
test pushes a third commit into it, not because of where the repo lives; and
`fake-coolify` needs cargo's `--` separator before `--version`. The three
Colima-CLI tests live in `berth-engine`, and there are 28 scenes, not ~27.

## Read first, in this order

1. `CLAUDE.md` — the rules, including the Mac environment section and the
   shot workflow, which now apply in full again.
2. `docs/STATUS.md` — "Current state", then "Decisions made" AD44–AD53, then
   the `shot pending Mac return` bullets under "What works" (P8-T3, P8-T4,
   P8-T5, P9-T2, P9-T5) with their difference lists. "Machine facts (Linux
   box)" is history; the Mac table above it is current.
3. `docs/TASKS.md` — the Phase 9 and Phase 10 "Learned" paragraphs, then the
   Phase 10 tasks.
4. `docs/PLAN.md` — §3 (review gate), §9 rows 8 and 9, §12 for P10.
5. `docs/QUESTIONS.md` — Q1, Q2, Q4, Q5 need the owner for Phase 10; Q6
   (Colima restart) and Q7 (SSH host) and Q8 (a real Coolify) are optional
   extras for the acceptances below.
6. `docs/HANDOFF-LINUX.md` — "Deferred to the Mac" is the origin of the list
   below.

## Current state (written 2026-10-28, Linux box)

- Phase 8 (P8-T3 rest, P8-T4, P8-T5) and Phase 9 (P9-T1..T5) are done and
  closed on Linux, and both are merged into `origin/main` through GitHub
  PRs (owner-approved 2026-10-28): PR #1 `6b6152d` (Phase 8, 0.8.0) and
  PR #2 `9de9ace` (Phase 9, 0.9.0, includes this handoff). CI
  (`scripts/check.sh` on macos-14) passed on both PRs: 2472 and 2704
  passed, 70 skipped. Phase 9 close commit `ac5421f`. Workspace version
  `0.9.0`.
- Last `scripts/check.sh` on Linux: exit 0, **2695 passed, 70 skipped**. The
  Mac count will differ (some tests are platform-gated); only exit 0 matters.
- Live ITs that passed on Linux against the native engine, with
  `BERTH_KEYRING=memory`: `registry_it`, `build_it`, `git_it`,
  `save_image_it`. `ssh_it` skipped (no `BERTH_SSH_TEST`).
- **Everything is pushed** (`main`, `phase/8-parity`, `phase/9-plugins`).
  The tags `v0.2`–`v0.7` are local to the Mac; `v0.8` and `v0.9` do not
  exist anywhere yet (item 8 below).
- Local branch `phase/9-plugins-crates` on Linux (`247f230`) is redundant:
  its commits were cherry-picked onto `phase/9-plugins`. Ignore it.

## Step 0 — sync the Mac with `origin`

*Done 2026-10-28:* local `main` = `origin/main` = `0df42c9` (PR #3), nothing to
fast-forward; `phase/9-plugins` is merged and not local, so acceptance ran on
`phase/9-acceptance` off `main`. The text below is the original instruction.

The Linux work is on GitHub (PRs #1 and #2, merged 2026-10-28). On the Mac:

1. `git fetch origin`, then fast-forward each local branch:
   `git checkout main && git merge --ff-only origin/main` (local `main`
   should be `35257b9`), and the same for `phase/8-parity` (local should be
   `1be9372`). `phase/9-plugins` is new: `git checkout phase/9-plugins`
   tracks it. If a `--ff-only` refuses, the Mac moved on since the first push:
   stop and compare (`git log main..origin/main`, `origin/main..main`).
2. Work from `main` (`9de9ace` or later). Phase 10 gets its own branch
   `phase/10-ship` from it.
3. The untracked `.audit/` in the repo root is the owner's. Leave it.
4. The sibling worktrees `../berth-p8t1`, `../berth-p8t5`, `../berth-p9t1`,
   `../berth-p9t2`, `../berth-p9t3` (branches `work/p8-t1-remote`,
   `work/p8-t5-git`, `work/p9-t1-protocol`, `work/p9-t2-host`,
   `work/p9-t3-coolify`) are **superseded**. What was used is on `main`:
   `p8-t5-git`, `p9-t1`, `p9-t2`, `p9-t3` (their head commits `8f8cdd0`,
   `a1f51a1`, `43c79ff`, `79e13dc` are on `main` with identical patches per
   `git cherry main origin/work/…`; the protocol-types and Coolify-client
   base commits `d2c6e64` and `b0fc712` landed rewritten by the Linux crate
   worker, so `git cherry` still shows them as `+`). `p8-t1-remote` (`7f3f3e1`)
   was never used: P8-T1 landed as `7a64c58`. Before removing any worktree run
   `git -C <dir> status` and keep anything uncommitted; never delete the
   `origin/work/*` branches (that is a push).
5. Run fixtures and live tests from this primary checkout only. Compose
   records the project folder; recreating `berthfx` from another checkout
   splits it in two (see Gotchas).

## Owed on the Mac, in order

State legend: `[x]` done with evidence in STATUS, `[ ]` still owed. A line
starting "State" says what the 2026-10-28 run actually did.

Record each result in `docs/STATUS.md` (command and output pasted) as you go.
Fix failures forward as `P8-HC: <fix>` or `P9-HC: <fix>` commits on
`phase/9-plugins`, the P2-HC pattern; do not move or create tags until all of
1–7 pass.

- [x] **1. Gate on the Mac.** *State: `scripts/check.sh` exit 0, 2705 passed,
      70 skipped (2704 / 70 at the first green; the first run failed on a
      Mac-only timezone assertion, fixed as P9-HC). Full `scripts/it.sh`: 2768
      passed, 6 skipped, run before the UI fixes and not repeated. Cleanup
      checked: no fixture-labelled container left and `docker images | grep
      berthfx` empty after `fx-down.sh` (two test-built `berthfx-gen-*` images
      were removed by hand). The owner's `server-db-1`, `ferry-pg` and
      `ferry-mysql` were not on the engine at session start, so none was
      touched.* `. "$HOME/.cargo/env"`, then
      `scripts/check.sh` exits 0, then the full `scripts/it.sh` (macOS engine
      `colima:default`, fixtures up, retries `fx-up.sh` itself). Many
      berth-app live tests hard-code `colima:default`; they were written on
      the Mac and should pass here unchanged. Afterwards
      `scripts/fx-down.sh` and `docker ps --filter label=dev.berth.fixture=1`
      show nothing left; `docker images | grep berthfx` is empty; the owner's
      `server-db-1`, `ferry-pg`, `ferry-mysql` are untouched.
- [x] **2. The three Colima-CLI tests that fail on Linux only (AD44)** must
      pass here (*State: all three pass 1/1; all three live in
      `berth-engine`, not `berth-app`; AD44 is inert on the Mac: the profile
      line printed `profile=default vm_type=Some(Vz) rosetta=Some(false)
      socket=Some("/Users/emmanuel/.colima/default/docker.sock")`*): `colima_it_default_profile`,
      `compose_it_a_failed_build_leaves_no_stopped_container`,
      `compose_it_restart_stop_start_config_validate`. Run each by name,
      all three with `-p berth-engine`, for example
      `scripts/it.sh -p berth-engine colima_it_default_profile`.
      Also check that AD44's discovery fallback (commit `9871bc2`, in
      `crates/berth-engine/src/discovery.rs`: a *running* profile whose
      `colima status` gave no socket falls back to
      `~/.colima/<profile>/docker.sock`, then the `colima` docker context's
      socket) does not change Mac behaviour: `colima_it_default_profile`
      prints the profile line; the socket must be
      `…/.colima/default/docker.sock`, as in the Phase 1 acceptance
      (`vm_type=Some(Vz) rosetta=Some(false)`). If `colima status` answers,
      the fallback never runs.
- [ ] **3. Keychain and PLAN §9 row 8 by hand.** *State: PARTIAL. Done: the
      keyring test passed against the real login keychain and
      `security find-generic-password -s dev.berth -a test/roundtrip` exits 44
      afterwards; `scripts/fx-git.sh` built the bare repo (`d544a3d`, 2 commits)
      and its working copy is at `/Users/emmanuel/berth-git-fixture/berthfx-app.git`;
      `scripts/fx-up.sh --registry` is up on 127.0.0.1:56500 (401
      unauthenticated, 200 authenticated; it also starts the base `berthfx`
      stack). NOT done: every step that needs a click in the app (registry
      login and the Keychain-versus-`berth.db` check, Build / Push / Export /
      Import, From Git clone and deploy, third commit, Check, Pull and
      redeploy, scratch project down). Blocked: `cmux-cua` reports
      `screen_recording: false` and its daemon says Computer Use onboarding is
      unfinished; unattended pixel-clicking is not used (AD60). The fixtures
      and the git copy were left up for this. The four native file panels
      (build context, Export, Import, "Clone into") are checked as "panel
      opens" only (AD58).*
      - `BERTH_KEYRING_TEST=1 cargo nextest run -p berth-store --run-ignored
        only keyring` passes, and
        `security find-generic-password -s dev.berth -a test/roundtrip`
        exits 44 afterwards.
      - Registry: `scripts/fx-up.sh --registry` (host port 56500, fixture in
        `fixtures/compose/berthfx-registry/`). In the real app (no
        `BERTH_KEYRING=memory`; `BERTH_DATA_DIR=$(mktemp -d)` keeps it off
        your data) add the registry login: the password must show in
        `security find-generic-password -s dev.berth` and **not** in the
        `berth.db` of that data dir (`sqlite3 <dir>/berth.db .dump | grep -c
        <password>` prints 0). Then Build image (fixture
        `fixtures/folders/dockerfile-only`), tag, Push, Export, Import, as
        in `build_it`.
      - Deploy `fixtures/git/berthfx-app.git`: run `scripts/fx-git.sh` first;
        the bare repo is gitignored and does **not** exist until it runs. Then
        **copy it** (for example to `$HOME/berth-git-fixture/berthfx-app.git`)
        and work only on the copy, because this step pushes a third commit
        into the repository it clones from and that must not touch
        `fixtures/git/`. (The copy is not needed to get under `$HOME`: this
        checkout already lives under `/Users/emmanuel`. Scratch folders that
        are mounted into containers must still be under `$HOME`, the Colima VM
        shares only the home directory and `/tmp` mounts empty, AD45 item 8.)
        Clone the copy in New project → From Git (a `file://` or plain path),
        Deploy, push a third commit to the copy from another clone, Check in the Git panel
        shows one incoming commit, Pull and redeploy runs `pull --ff-only`
        then `up --build`. Bring the scratch project down with
        `compose down -v`; its name must start `berthfx-` (CLAUDE.md Docker
        safety).
      - SSH engine only if the owner names a host:
        `BERTH_SSH_TEST=<user@host> BERTH_IT=1 cargo nextest run -p
        berth-engine --run-ignored only ssh_it` (Q7). Otherwise it reports
        skipped and only the unreachable state is checked by hand.
      - Delete the Keychain items your run created; list what is there with
        `security find-generic-password -s dev.berth` and tell the owner what
        remains.
- [ ] **4. PLAN §9 row 9 by hand.** *State: NOT RUN. Blocked for the same
      reason as the GUI leg of item 3 (Computer Use onboarding unfinished,
      no screen recording). The two Coolify scenes were shot and read (item
      5), but nothing was run against `fake-coolify` in the live app.* In one
      terminal,
      `cargo run -p berth-coolify --example fake-coolify` (port 18080, any
      bearer token accepted, no header gives 401). In another, build the
      plugin next to the app (`cargo build -p berth-coolify`; the app puts
      the app binary's directory on the child's `PATH`) and run the app with
      `BERTH_PLUGIN_DIRS=$PWD/plugins/coolify` (a development override only)
      and no `BERTH_KEYRING=memory`. Check, with the owner watching where it
      matters: Plugins → Enable… shows the permissions (Network hosts
      "shown, not enforced", Secrets `coolify/token`, MCP); Configure takes
      `http://127.0.0.1:18080` and a token, the token lands in the Keychain
      (account `plugin/coolify/coolify/token` by AD48's `plugin/<id>/<key>`;
      confirm with `security find-generic-password -s dev.berth`) and **not**
      in `berth.db`; the Coolify section renders resources and the
      deployments table (12 rows); Deploy's confirm names the application,
      server and branch (App landing, Server hetzner-1, Branch main); the
      failed deployment's log opens scrolled to the failing step (the
      `Caddyfile:14: unknown directive` line); the fake with a newer major, started as
      `cargo run -p berth-coolify --example fake-coolify -- --version 5.0.0`
      (cargo needs the `--` before the example's own flags), shows the
      newer-major warning banner. A real instance only if the
      owner supplies one (Q8).
- [x] **5. Every `shot pending Mac return`.** *State: DONE. All 28 scenes
      captured and read (not ~27: P8-T3 6, P8-T4 6, P8-T5 7, P9-T2 7, P9-T5 2),
      plus 17 extra risk shots and four of the Dockerfile editor typed into by
      hand. 41 findings: 2 fixed by the shot worker, 13 already known, 19 not
      defects, 7 ruled by the architect. Ten UI fixes were made and seven
      deviations accepted (AD55-AD58; the deviations are named audit items
      under TASKS P10-T3/P10-T4). Shots are in `target/shots/` (ignored by
      git).* For each scene:
      `scripts/shot.sh <name> --scene "<frame>"`, read
      `target/shots/<name>.png` beside the frame PNG in
      `design/shots/figcraft/screens/` where one exists, list each difference
      in STATUS as fixed or as a question (CLAUDE.md "UI matches the frame").
      Only the two Coolify scenes have a frame PNG; the rest are dev scenes
      with no frame, so read them beside the nearest sibling (Images find,
      Create volume sheet, New project sheet) and the layout risks noted in
      STATUS.
      - P8-T3: `Images / build / 1440`, `Images / build dockerfile / 1440`,
        `Images / build running / 1440`, `Images / build failed / 1440`,
        `Images / build done / 1440`, `Images / build pushing / 1440`. Risks
        from STATUS: Images controls row width at the narrowest window; the
        inspector header buttons wrapping at 320 px and bottom dock; the
        640 px sheet; the build log's `column-reverse` scroll in WKWebView
        (scrolling up must not snap back while lines arrive); the CodeMirror
        Dockerfile editor is not rendered in SSR, so type in it once by hand.
      - P8-T4 (`scenes/save_image.rs`, `scenes/templates.rs`):
        `Containers / save as image / 1440`,
        `Containers / save as image / failed / 1440`,
        `Containers / save as image / invalid / 1440`,
        `Containers / save as image / running / 1440`,
        `Containers / save as image / saved / 1440`, and
        `Templates / delete confirm / 1440`. Risks: the Custom template row's
        actions cell clipping at large text sizes; the delete confirm uses a
        local scrim, so global Esc does not see it.
      - P8-T5 (`scenes/from_git.rs`): `New project / from git / 1440`,
        `New project / from git / clone failed / 1440`,
        `New project / from git / cloning / 1440`,
        `New project / from git / invalid compose / 1440`,
        `New project / from git / wrong address / 1440`,
        `Project / git / 1440`, `Projects / git / 1440`. Risk: From Git is the
        third stage of the New project sheet, 640 px wide (`new_project.css:9`;
        the 520 px in earlier notes was wrong).
      - P9-T2 (`scenes/plugins.rs`; each `Dev / plugins / <variant>`, no
        `/ 1440` suffix; seven scenes): `empty`, `installed`, `enable sheet`,
        `configure new`, `configure saved`, `enabled`, `error`. Risks: card
        header row (name, version, state, two buttons) at the narrowest
        window; the crash log capped at 160 px.
      - P9-T5 (these two have frames): `Coolify / deployments / 1440` and
        `Coolify / confirm redeploy / 1440`; example commands
        `scripts/shot.sh coolify-deployments --scene "Coolify / deployments / 1440"`
        and
        `scripts/shot.sh coolify-confirm --scene "Coolify / confirm redeploy / 1440"`.
        Known **open** differences (STATUS "Differences from …"; the
        architect accepted the Plugins sidebar item): *deployments:* no
        "Failed · still serving c0ffee1" line in the pane (not in Coolify's
        deployment list); the pane's button reads "Redeploy…" where the frame
        says "Redeploy c2a7e94…"; "Fix with Claude Code" is greyed in a scene
        because a scene runs no assistant service (live it needs a harness, a
        linked folder and the loaded log: check it on the Mac against the fake
        Coolify); the Message column has a heading and widths follow text
        length; fonts (IBM Plex Sans is not installed here either, so type
        metrics differ from the frame exports); the Server chip's × only hides
        it. *confirm redeploy:* warning text reads "This commit failed
        (Caddyfile:14: …). Redeploying it will likely fail again." where the
        frame has "failed 4 min ago at step 3 (…)"; the note says "Keeps
        serving the current version" where the frame names c0ffee1; Fix with
        is greyed in a scene. These need data the Coolify API lacks; close
        them as deviations accepted by the architect or ask.
- [ ] **6. Batched owner checks** *(State: NOT RUN, by design: each of these
      stops containers on the owner's engine, which CLAUDE.md puts behind the
      owner's presence.)* (STATUS, fix-forward): real first-run
      "Start default" (stops the running profile's containers; the owner must
      be watching); true cold start after `sudo purge`; the menu-bar icon in
      light and dark and whether macOS hides the extra; the Colima restart
      path, Q6 (ignored test
      `engines_it_restart_default_profile_owner_present_only`, gated on
      `BERTH_COLIMA_RESTART=1`; it restarts the `default` profile and stops
      the owner's containers for about 20 s, so only with the owner present).
      The `~/Documents` permission prompt for ⌘O needs the bundled app and is
      a P10 check. (No VoiceOver check: the owner decided there is no voice
      in the app.)
- [ ] **7. Performance spot-check.** *State: DEFERRED, no number. Three runs
      were attempted on 2026-10-28; the first started at load average 7.32 and
      was stopped at 59.77 while still compiling release (rc=143), the other
      two were killed mid-compile (the second reported `rc=101`). There is no
      results table; do not quote anything from the partial log.*
      `scripts/perf.sh --app` on a quiet
      machine, fixtures per its header; report ranges, not one number. The
      last Mac numbers (Phase 6): cold start to a populated Projects table
      580 ms median (budget 600), idle CPU 0.10 %, footprint 97 MB.
- [ ] **8. Tags, locally.** *State: NOT DONE; `git tag --list 'v*'` lists
      `v0.2` to `v0.7` only. Items 3 (GUI leg), 4, 6 and 7 have not passed.*
      Only after 1–7 pass:
      `git tag v0.8 6b6152d` and `git tag v0.9 9de9ace` (the PR merge
      commits; check `git rev-parse v0.7` is the Phase 7 merge `35257b9` and
      follow the same pattern). If acceptance needed fix commits and you want
      the tags to include them, ask the owner where they point. Never push a
      tag.
- [ ] **9. Then Phase 10** (below). *State: not started. Next task id
      P10-T1, after items 3 (GUI leg), 4, 6, 7 and 8.*

## Decisions that bind

Numbered ADn in `docs/STATUS.md` "Decisions made"; PLAN owns D1–D84.

- **AD44** discovery fallback for a running Colima profile without a status
  socket; harmless on the Mac, verified in step 2.
- **AD45** `berth-store` `git_sources` (migration `0002`); git addresses
  normalised to `url::Url`; SSH key only via `core.sshCommand`; scratch
  folders mounted into containers live under `$HOME`.
- **AD46** Save as image is an inline bar under the inspector header and
  Delete template a local confirm, because PLAN §6 had no `Overlay` variant.
  Moving both onto real overlays is a wish, not scheduled.
- **AD47 / AD53** each phase closes without its tag; tags wait for the Mac
  acceptances (step 8).
- **AD48** `Overlay::PluginEnable(String)` and `PluginConfigure(String)`;
  `Store::plugin_config` JSON `{ enabled, granted, config }`; secrets only in
  the `SecretStore` under `plugin/<id>/<key>`; plugin dirs are
  `<data dir>/plugins` plus the app binary's directory.
- **AD49** crate-side plugin decisions (a view failure is a `Message` view,
  not an RPC error; `mcp_enabled` gates `host/mcp.servers`; project badges
  come from `GET /applications`).
- **AD50** Disable clears grants; Enable grants all or none; empty secret
  field keeps the stored value; `BERTH_PLUGIN_DIRS` is a development
  override; `BERTH_KEYRING=memory` makes each `secret_store()` call a
  separate store (irrelevant on the Keychain).
- **AD51** version warning is a `Banner` from the hidden `status` view; **no
  env reveal in v1** (envs stay masked, no reveal path); the plugin log is a
  compact `LogLine` list, not `LogViewer`; badges asked every 60 s only for
  projects with a git remote.
- **AD52** `Overlay::PluginAction(PendingAction)` (PLAN §6.11); conventions
  on existing fields (an `open`-only action is opened by Berth; a
  `repository` cell links a local project by git remote; a `Warning:` confirm
  line is the amber box); Fix with on a plugin log never attaches `.env`.
- **AD54-AD60** (Mac acceptance, 2026-10-28): branch `phase/9-acceptance`
  and the P9-HC timezone test fix (a rendered-time test never hard-codes a
  wall clock); the ten UI fixes; the seven accepted deviations, two of them
  shell-wide and deferred to P10-T3/T4 (Inspector toggle on every screen with
  an inspector; `.bottom-panel button` flattening); `secret-scan.sh` stays
  limited to shipped code; no unattended pixel-clicking, tags wait for items
  1-7.
- **Bundling (Phase 10):** `berth-coolify` must sit next to the app binary
  in the bundle (`Berth.app/Contents/MacOS/{berth, berth-coolify,
  berth-plugin.json}`; `scripts/release.sh` copies it). `BERTH_PLUGIN_DIRS`
  is dev-only.

Wishes noted in STATUS, not done (outside their tasks' files): fold the
duplicated `editor_script` in `sheets/new_project.rs` and
`sheets/build_image.rs` into one helper; a `\r`-aware line reader in
`berth-sys` for finer clone progress (AD45); move the Save-as-image bar and
Delete-template confirm onto overlays (AD46); a palette "Go to Plugins"
action and Activity entries for plugin enable/disable (AD50); a protocol
filter facet for the "Server: hetzner-1" chip, `Selection::Plugin` for
docking plugin panes in the inspector, plugin Activity entries (AD51/AD52).
Q9 was moved to Answered in `docs/QUESTIONS.md` on 2026-10-28 (pick (a)).

## Constraints (unchanged, from CLAUDE.md)

- Secrets only through `SecretStore`; the fixture password only in
  `fixtures/` and `scripts/`; never log a `BuildSpec` or `GenerationPlan`.
  Run `scripts/secret-scan.sh` before committing acceptance output into
  STATUS (it must not hold a token, Keychain value or `.env` value).
- Nothing changes without a click; project files go through Review changes.
- Docker safety: touch only `dev.berth.fixture=1` resources (`berthfx…`),
  `berth-helper-…` containers a test created, and `berthfx-*` Compose projects
  from fixture copies on ports 56000–56999. Never touch `server-db-1`,
  `ferry-pg`, `ferry-mysql`. Never start, stop, restart or resize a Colima
  profile unless the task says so and the owner is present.
- Git: branch `phase/N-name`, commits `P<N>-T<M>: <what>`. Never push, tag
  remotely, release, set repository secrets or run `release.yml` without the
  owner's explicit approval.
- End of session: `scripts/check.sh` exit 0, the task's done-when pasted into
  STATUS, STATUS updated with the exact next task id, commit.

## Linux-only conventions to drop, and the ones to keep

- Drop: `BERTH_KEYRING=memory` (use the Keychain; Keychain-touching tests run
  for real), "shot pending Mac return" and SSR-as-shot-substitute
  (a shot is now the evidence; `scripts/ssr.sh` stays a quick look), the
  Linux `platform/other.rs` stubs (macOS uses `platform/macos.rs`).
- Keep: new live tests pick the first running engine, never a hard-coded
  `colima:default`; scratch folders mounted into containers live under
  `$HOME`; wall-clock assertions only in `scripts/perf.sh`.

## Gotchas

- Live tests share the four `berthfx` containers: serial (`live-engine`
  group), each starts with `settle_fixture()`. Create fixtures from the
  primary checkout. If a worktree recreated `berthfx`, run
  `docker compose -p berthfx -f fixtures/compose/berthfx/compose.yaml up -d
  --force-recreate` from the primary checkout.
- `it.sh` retries `fx-up.sh` up to 5 times; one failed first attempt is
  normal.
- `check.sh` after a long idle can be slow on the first build: sccache is the
  rustc wrapper on the Mac (`~/.cargo/config.toml`); that config belongs to the
  Mac only.
- `detect` tests use a 400 ms version timeout and can starve under full-suite
  load; rerun before calling it a failure.
- Perf and wall-clock numbers are load-sensitive; quiet machine, ranges.
- If `cargo` rewrites `Cargo.lock` on the Mac, read the diff before committing
  it; the Linux period needed no regeneration beyond `8c2c3a9`.

## Then Phase 10

Mac-only: `phase/10-ship`, tag `v1.0.0`. Read TASKS "Phase 10" ("Learned"
first). Tasks in order: **P10-T1** bundle configuration (needs **Q1** product
name, **Q2** licence and **Q5** app icon from the owner; Q3 icons has a
default), **P10-T2** release script and unsigned DMG (copies `berth-coolify`
into the bundle), **P10-T3** and **P10-T4** design audit (34 frames plus the
no-frame screens; the Linux-period screens listed in step 5 are part of it),
**P10-T5** signed and notarized build (needs **Q4**; only with the owner's
go-ahead), **P10-T6** clean-install test and `1.0.0`. Push, release and cask
publication only with the owner's approval.
