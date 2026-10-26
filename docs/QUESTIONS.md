# Questions for the owner and the architect

Each open entry has a default already taken, so work never waits. The owner answers by editing the entry or telling the architect; the architect moves it to "Answered" and updates PLAN, TASKS or an ADR if needed.

## Open

- **Q9 Project status rule.** P1-T7's `status()` makes any Unhealthy service Failing (its Done-when), but the frames call `analytics` Partial (3 of 4 running) while `ingest` is Unhealthy; the mock world stores the brief's Partial, so live grouping and mock scenes can disagree. Options: (a) only Dead/Restarting fail a project (Unhealthy stays Partial; Problems already flag it per container); (b) keep the rule and treat the frames as wrong. Architect's pick: (a), to apply at P2 with the STATUS note. Owner to confirm.
- **Q1 Product name (brief §9).** Default: "Berth", bundle id `dev.berth.app`, binary `berth`, Keychain service `dev.berth` (D64). Found 2026-10-24: the GitHub repository `fullstoptech/berth` exists; the Homebrew cask name `berth` is free; on crates.io `berth` (an MCP server runtime) and `berth-registry` are taken by another project, which does not matter for unpublished crates but is a brand overlap. Renaming later is one task (identifiers, Keychain service, bundle id); do it before P10-T1 at the latest.
- **Q2 Licence (brief §9).** Default: MIT OR Apache-2.0 (D63). Alternative: GPL-3.0 if forks must stay open. Needed by P10-T1.
- **Q3 Icons.** Default: the hand-drawn Lucide-style icons exported from `design/berth.figcraft` (D10, P0-T3). Alternative: download the real Lucide SVGs (MIT, about 1,500 icons) and keep the ones the design uses. `.eng/notes.md` records this as waiting for your OK to download.
- **Q4 Signing and notarization.** Default: P10-T5 uses "Developer ID Application: Emmanuel Badu Sarpong (RVPL25L2BR)" and the existing notarytool profile `ferry-notary` (both answer on this Mac, 2026-10-24), only after you say go. Alternative: a separate profile for Berth.
- **Q5 App icon.** The FigCraft file has no app icon frame (34 screen frames, checked 2026-10-24). Default: P10-T1 drafts three directions with the design skill and you pick one; until then the window uses the Dioxus default icon.
- **Q6 Restarting Colima during development.** Edit resources and Start/Stop engine restart your `default` profile, which stops `server-db-1` and the Ferry fixtures for about 20 s. Default: P4-T6 tests the sheet without applying; the real restart happens once at the P4 acceptance check with you present. Alternative: a throwaway profile `berthfx` created for tests (`colima start -p berthfx`, about 2 GB of disk).
- **Q7 SSH host for remote-engine tests (P8-T1).** Default: no host; the SSH integration test reports skipped and only the unreachable state is checked by hand. Give a `user@host` with Docker to run it.
- **Q10 Harness TUI in a Berth terminal tab (P7-T6, AD38).** Default taken: "Open in Terminal" launches the harness TUI in Terminal.app (D42 `.command` file) with the bridge tools attached, because an interactive host process in Berth's own xterm tab needs a PTY and PLAN §5 has no PTY crate; muse's button is disabled ("cannot take Berth's tools into a terminal yet") because no verified way attaches the `berth` MCP server to interactive `muse` (its session config serves `muse serve`). Options: (a) keep Terminal.app; (b) add a PTY crate to §5 (architect picks + owner OK) and build `Kind::Host`; (c) find muse's interactive MCP config path and enable it. Architect to decide at Phase 7 close or P10 audit.
- **Q8 Coolify instance (P9).** Default: wiremock only, built from the v4.5.0 spec. Give an instance URL and a read-only token to try the plugin against a real server.

## Answered (decisions already made in the brief's conversation, 2026-10-24)

- "Container" meant Portainer: developer-relevant Portainer features are in (brief §1.1); RBAC, edge, Swarm, Kubernetes are out.
- Audience: for Emmanuel, released as open source; no accounts in v1, data keeps `actor` and owner fields (brief §1.2).
- Folders are never scanned in the background; projects come from running containers and folders the user opens or pins (brief §9; D36).
- The AI is interactive: multi-turn harness sessions with Berth's Docker tools over MCP, log reading, watch mode (brief §6.12; ADR 0005, 0006).
- Harnesses only propose file changes; Berth writes after Review changes (D70).
- Menu bar item: yes (brief §6.14; ADR 0010).
- Actions are visible buttons on rows, headers and the bulk bar (brief §7.2).
- Stack: Rust, Dioxus 0.7 desktop webview, bollard (ADR 0001, 0003); plugins out of process (ADR 0008).
- Build host: this Mac only. "I'm running the whole thing here on the Mac" (owner, 2026-10-24); the earlier Linux-build plan was dropped.
