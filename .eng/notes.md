# Notes

## 2026-10-24
- Direction (Emmanuel's words, condensed): "I don't want to use a normal Docker desktop. I want to use Colima with a Docker client"; containers known "based on the folders I'm using"; Coolify as the first plugin, "not built in"; AI that detects "the Claude harness or the Muse harness"; "very dense UI"; "i prefer dioxus".
- Checkpoint: archetype dense-pro (borrowing the show-the-effect-before-commitment rule); platforms macOS first, Linux second; stack Rust + Dioxus 0.7 desktop + bollard; target this repo, FigCraft file design/figcraft/berth.figcraft next; defaults taken listed in design/product-brief.md section 9.
- Stopped after the brief: Emmanuel asked for the brief and questions before screens.

## 2026-10-24 (round 2)
- Emmanuel: "also add container portaine functionalities in it ... most of the functionalities that will help is for developer. It's for me, but it will be open source. I think I don't need any accounts currently, but maybe in the future I can use accounts."
- Decided: "container" meant Portainer. Developer-relevant Portainer features added (brief 1.1); ops-fleet features (RBAC, edge, Swarm, Kubernetes) left out. Open source, no accounts in v1; data keeps `actor` and owner fields so accounts can be added later (brief 1.2).

## 2026-10-24 (round 3)
- Emmanuel: menu bar yes; "don't scan folders"; the AI "should be interactive", create Dockerfiles and Compose files, "get errors from containers", "read console logs ... see the error and use it to fix it"; auto-detect Claude or muse; "make it very, very performant"; use Coolify's current docs.
- Decided: AI = harness session driven by Berth with Docker tools over a local MCP server; reads are free, changes need a click, destructive actions need the name typed; file edits only as proposed changes (brief 6.12). Logs is a top-level section (6.13). Menu bar (6.14). Performance budget (1.3). Coolify plugin built on OpenAPI v4.5.0 (6.11).
- Emmanuel (round 3, later): "I need actually UI buttons for stop, start, and all those things like restarts or deletes for the various containers." Recorded as brief 7.2: visible buttons on rows, detail headers and a bulk bar, with confirm-on-delete and disabled-with-reason rules.

## 2026-10-24 (design run)
- Emmanuel: "using /eng:design ... figcraft-app live design the ui using mobbin mcp and also doing ui research online."
- figcraft-app MCP was disconnected; FigCraft.app launched with --control 7979 and driven live through design/figcraft/live.py (MCP over stdio to `figcraft-cli mcp --attach 7979`).
- System: IBM Plex Sans + JetBrains Mono (chosen over Schibsted Grotesk, Hanken Grotesk, SF Pro, IBM Plex Mono); signal-blue accent H252; near-pure grey neutrals; status is the only colour. Art-director round 1 applied (dark dots, saturated fills, lifted dark ladder, 5 series hues, log line tints). See design/design-system.md.
- Research: 26 references in design/research/refs (Mobbin + web), log in design/research.md round 3.
- Screens round 1 (17 frames) critiqued by eng:art-director and eng:user in parallel. Top persona quit risks: unclear order of AI fix steps (Approve vs Write files and restart), dangerous commands beside routine ones, self-contradicting status (No problems while loading/stopped), missing confirm screens (Clean up, engine starting, Coolify redeploy), bare icons. Art-director: logs focal point lost to blue selection, ragged baselines, blue overweight in dark, Engines dashboard fake sparklines, assistant panel led by harness picker. Round 2 (22 items + 3 new frames) sent to the builder.
- Lint: plugin lint assumes 44px touch targets; project wrapper design/figcraft/lint_desktop.py applies size.target.min 24px (WCAG 2.2 2.5.8) and waives ink-disabled contrast (WCAG 1.4.3). Controls sm 24 / md 28.
- [open] Icons are Lucide-style hand-drawn paths; swapping in real Lucide SVGs (MIT) needs Emmanuel's OK to download.
- Emmanuel (2026-10-24): "i want to be able to drag a folder into the app and also access terminal for each container and i need a ui for finding images online." Added brief §6.15 (drop folder overlay + Detected sheet), §6.16 (terminal per container: tabs, shell/user picker, split, pop out, open in Terminal app, no-shell state), §6.17 (Find images: Docker Hub + registries, publisher badges, arm64 support, tags per arch, pull/run/add to project, rate-limit state). Screens 23–29 queued after round 3.
- Emmanuel: "aside the drag and drop i want to be able to browse the file system". Interpreted as an in-app folder browser for opening projects (brief §6.18; frames 32–34), keeping the no-background-scan rule: markers only for the folder on screen. Container and volume file browsing already existed.
- Final screens batch (round 3 + 7 requested frames + persona trust fixes): 31 frames, lint_desktop 0 FAIL, check_bounds 0. Open decision: Uptime column hidden while the inspector is open (still in the inspector header).
- Final (2026-10-24): 34 screens, 64 components + icons; lint_desktop 0 FAIL, check_bounds 0. Art-director: "ready to hand to the build". Generated compose fixed per persona (password into .env, container hostnames, develop.watch).
- Emmanuel (2026-10-24): build Berth on Linux first; a Mac handoff (docs/HANDOFF-MAC.md) takes care of the Mac-only work later, like Ferry. docs/SOFTWARE_PROMPT.md gained item 8 (build host differs from release host → Linux dev ADR, HANDOFF-<HOST>.md, <HOST>_KICKOFF_PROMPT.md, `Linux check:` lines, per-host CLAUDE.md, two /goal lines). Linux host assumed to be Ferry's Ubuntu 22.04 server; its facts come from Ferry's docs, marked re-verify.
- Split clarified: Linux sessions only write and test Rust code (check.sh, unit, SSR, integration); everything Mac related, including running and seeing the app and the design audit, happens on the Mac via HANDOFF-MAC.md. No Xvfb tooling for Berth.
- 2026-10-24, later: Emmanuel reversed the split: "I'm running the whole thing here on the Mac, so forget the Linux specific stuff." Build host = this Mac only; no HANDOFF-MAC, no Linux ADR. SOFTWARE_PROMPT keeps the different-hosts item as a conditional for other projects.

## 2026-10-25 (spacing)
- Emmanuel: "the general spacing seems tight". Audit at 1x: table rows 28 (Containers, Activity, palette), Projects rows 32, Recent activity rows 24, page gutter 16, header block 12-13 top / 6-8 bottom, button gaps 4, section gaps 24 above Needs attention and 20 above Recent activity; Containers showed 26 lines at 1440x900.
- Decided (D11, option A plus tweaks): `size.row.md` 28 -> 32 (compact stays 24); page gutter 16 -> 20 from a 1200 px-wide window up (new `layout.gutter` token, `--gutter`; `--space-12` untouched); Needs attention and Recent activity get `space-32` above them (was 24 and 20); header block 16 top, 20 bottom, 8 to the table; button gaps in the Projects row actions and the bulk bar 4 -> 8; Recent activity rows 24 -> 28. Unchanged: status bar 24, title bar 38, sidebar rows 28, menu rows 28, cell padding 8, column gap 12, dialog padding 24.
- Numbers (1440x900, 100 %): Containers lines 26 -> 22 (+ a cut-off 23rd) with the inspector docked; Projects rows 32 -> 36 (`--projects-row` = row + 4, left as is); Activity rows 28 -> 32.
- Side effects handled: the container-query em tiers for Containers moved by the wider gutter (63 -> 63.5, 55 -> 55.5, 68 -> 68.5 em); the Projects buttons track grew 12 px (3 gaps x 4) so Services at 1024 px and 150 % has 88 px (was 100). Containers row buttons keep gap 0: 8 would need 40 px more in a table with no slack at 1024.

## 2026-10-25 (spacing fix-up, D12/D13)
- Art-director review of the D11 commit plus advisor decisions D12 and D13, applied as seven `Design:` commits.
- Bug first: `.vt-row` and `.table-header-cell` lacked `box-sizing: border-box`, so the 1 px border made 32 px rows 33 px while VirtualTable assumed 32 (scroll drift of 1 px per row). Fixed, with a test that ties the stylesheet row to `DEFAULT_ROW_HEIGHT_PX` and `row_height_px`.
- One 12 px gap from the last header control to the column headers on Projects, Containers and Activity (was 42, 16, 32); all three column-header rows are 28 px (`size.control.md`).
- Activity's first column, headers, rows and gap banner sit on the page gutter, under the title.
- Projects: `size.row.lg` = 36 (compact 28) replaces `--projects-row`; 1 px divider under each row; button gap 8 from 1200 px wide, 4 below (same breakpoint as `--gutter`); the Services floor at 1024 px and 150 % is back to 96 with gap 4. Needs attention is one bordered list (32 px rows, 1 px dividers, nothing below), so each section sits 32 px below what precedes it.
- Containers row buttons keep gap 0 (decided).
- Inspector: one 12 px left edge for header, tabs, key/value list and failure card; Stop, Restart, Pause and Kill on one line, Delete on its own line below (button padding eases from 6 to 4 px as the text grows to 150 % so the four still fit 360 px); the tab strip shows 5 tabs up to 100 %, 4 up to 130 %, 3 at 150 %, the rest under the existing ⋯ menu.
- Numbers (1440x900, 100 %): Containers 22 lines visible plus a cut-off 23rd with the inspector docked; Projects Recent activity shows 8 rows, the 8th about 8 px short of the status bar (the page is 32 px taller than the window and scrolls).
