# Software prompt: turn a product brief into a build-ready docs folder

Paste this into Claude Code (Opus) at the root of a new project that has a product brief. Fill the four inputs at the top. It writes the same planning set Ferry used (`~/Dev/ferry/docs`), so a cheaper model can build the product task by task without making architectural decisions.

---

## Inputs

- **Product brief:** `<path, e.g. design/product-brief.md>` (required)
- **Design artefacts:** `<FigCraft file, tokens.json, screens spec, component spec, mock data; or "none">`
- **Reference docs folder:** `<e.g. ~/Dev/ferry/docs; or "none">`: copy its structure, tone and level of detail, never its product content.
- **Build hosts:** `<e.g. "this Mac only" or "Linux build, Mac release">`

## Prompt

You are the architect for this product. Turn the product brief into a docs folder that lets a less capable model (the workhorse) build the whole product one task per session, without making architectural decisions. Opus is the architect and reviewer; Sonnet is the workhorse.

**Read first.** Read the product brief, every design artefact listed, and `.eng/stack.md` / `.eng/notes.md` if present. If a reference docs folder is given, read its `PLAN.md`, `TASKS.md`, `GOAL.md`, `KICKOFF_PROMPT.md`, `STATUS.md` (top part only), `QUESTIONS.md`, two ADRs and the root `CLAUDE.md`, and match their shape.

**Probe the machine before deciding anything.** Record each fact with today's date as "confirmed on this machine": OS and architecture, installed toolchains and versions, the runtimes the product depends on (for example `colima version`, `docker context ls`, socket paths), CLIs the product integrates with (`--version`, `--help` for the flags you rely on), free ports for fixtures. Never write a version, flag, path or API name you have not checked; when you cannot check one, mark it "unverified" and give it a task that verifies it first.

**Verify every dependency.** For each library in the approved list, confirm the name and current major version from the registry (`cargo search`, `npm view`, docs). For each interface that calls a library, check the real type and function names in its docs.

**Write these files** (skip any that do not apply and say why; for example a host handoff runbook exists only when building and releasing happen on different machines):

1. `docs/PLAN.md`. The contract. Sections, in this order:
   1. *Refined prompts*: the product brief condensed to one quoted paragraph, plus the execution brief (this prompt's purpose).
   2. *Scope*: a v1 table by area, and a "Later" list that no v1 task touches.
   3. *Operating model*: the architect and workhorse roles table; the five escalation conditions (interface change, unlisted dependency, done-when failing after two honest attempts, a CLAUDE.md rule would break, task contradicts code); the seven-step review gate run at each phase close; session size.
   4. *Decision register*: numbered decisions (D1…) in tables, each with choice and reason, covering framework, platform targets, runtime, async, cancellation, errors, logging, IDs, time, serialization, local storage, secrets, config paths, state management, testing, lints, branches and commits, versioning, how sessions see the running app, and every product-specific decision the brief left open. Every open question in the brief becomes a decision here, or stays in QUESTIONS.md with the default taken.
   5. *Approved dependencies*: crate or package, minimum version, where it is used; and a "not approved" line.
   6. *Architecture and interfaces*: the workspace tree, strict dependency direction, then per crate or module the real public types and traits in code (derives, field names, function signatures). These are what the workhorse implements against.
   7. *UI specification*: shell layout as an ASCII sketch with sizes, one row per component (behaviour, states), keyboard map, bridge messages to any embedded web code, token names. When a design exists, point to the frames and say the UI is built to them from the first UI task.
   8. *Tooling the sessions can run*: build, test, lint, run, see the app (screenshot script), fixtures, integration tests, logs. Plus the fixture data every acceptance check uses.
   9. *Phases*: a table with phase number, name, an acceptance check the architect runs by hand, sessions, and token estimates (low / expected / high), plus the cost split between the two models.
   10. *Session protocol*: start and end of every session, reset after drift, parallel worktrees.
   11. *Design handoff*: where the design lives and how UI tasks use it.
   12. *Packaging and release*.
   13. *Risks*: each with its mitigation.
   14. *Day one checklist*: exact install commands and the first three steps.
2. `docs/TASKS.md`. Phases as `## Phase N — Name (branch phase/N-name, tag v0.N)`, each with a `Learned: —` line for the architect. Tasks as `**P<N>-T<M> Title.**`, naming the files they touch and the PLAN sections they implement, sized for one session, ending with `Done when:` and a command plus its expected result, never an opinion. UI tasks name the design frame they match. Phases that can run in parallel say so.
3. `docs/GOAL.md`. The refined build prompt, a numbered definition of done, and a one-line `/goal` command.
4. `docs/KICKOFF_PROMPT.md`. The workhorse prompt for P0-T1 (same shape for every task, changing only the id) and the architect prompt for phase open and close.
5. `docs/STATUS.md`. Initial state: "plan only, nothing built", current task P0-T1, empty "What works", "Half done" and "Decisions made" sections, and the confirmed machine facts.
6. `docs/QUESTIONS.md`. Open entries only for questions the owner must answer (with the default taken so work never waits), plus an "Answered" section for decisions already made in the brief's conversation.
7. `docs/adr/000N-<slug>.md`. One ADR per foundational decision (framework, each external integration, secrets, sealed web bundles, design-first UI, anything with a real alternative). Each has Status, Context, Decision (citing the D numbers), and Consequences.
8. **When the build host and the release host differ** (for example "Linux build, Mac release"), also write:
   - `docs/adr/000N-<build-host>-dev-environment.md`: how the build host differs from the release host (paths, ports, display, missing tools) and the substitutes used.
   - `docs/HANDOFF-<RELEASE-HOST>.md` (for example `HANDOFF-MAC.md`): the ordered runbook of everything that can only be done on the release host, with exact commands and a done-when check per step. It covers setup, the build and test run there, the acceptance checks redone there, platform-only features (keychain, native dialogs, menu bar, OS permissions, platform shortcuts), the design audit on the real renderer, packaging, signing, notarization, the release and a clean-install test.
   - `docs/<RELEASE-HOST>_KICKOFF_PROMPT.md`: the paste-ready prompt that starts the release-host session.
   - In TASKS.md, every task that has a part only the release host can do keeps its normal `Done when:` and adds a `Linux check:` (or `<build host> check:`) line with what to run on the build host, plus `(Mac: see HANDOFF-MAC.md)` for the deferred part. Platform-only code sits behind `cfg` gates with a build-host fallback, so the build host compiles and tests everything else.
   - CLAUDE.md environment facts get one section per host. GOAL.md gets a definition of done for the build host and a separate one, with its own `/goal` line, for the release-host session.
   Ask the owner how the work splits when the brief does not say. A common split: the build host only writes and tests code (compile, lint, unit, render and integration tests); everything that needs the release platform (running and seeing the app, the design audit, platform features, packaging, release) goes to the handoff. In that split, a UI task's `Done when:` is the release-host check against the design frame, and its build-host check is the compile and render tests.
   Facts about a host you cannot probe from here come from the reference project when it used the same host, marked "from <project>, re-verify in the first session".
9. `CLAUDE.md` at the repository root. Agent instructions: what to read at session start, the roles, the escalation conditions, the non-negotiable rules (secrets, nothing runs without a click, cancellation, path resolution, lints, tests that fail when the code breaks, no refactors outside the task, UI matches the frame), the end-of-session checklist, and environment facts per host.

**Rules for the writing.** Plain, short sentences. Tables for registers and lists. Real code for interfaces. Every number carries its source or is marked as a target. Every `§` reference must resolve. Cross-check before finishing:

- every brief feature maps to at least one task (write a coverage table at the end of TASKS.md: brief section → task ids);
- every crate a task uses is in PLAN §5;
- every type a task names is in PLAN §6;
- every task has a `Done when:` command;
- every phase has an acceptance check;
- nothing from the reference project's content leaked in (names, paths, fixtures).

**Finish** with a short report: files written with line counts, the decisions you made that the brief did not, the facts you could not verify, and the coverage table's gaps (should be none).
