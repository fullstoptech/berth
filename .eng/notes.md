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
