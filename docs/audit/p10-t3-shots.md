# P10-T3 design audit, part 1: P2-P4 frames vs fresh shots (2026-10-28)

Scope (owner, must-fix only): each shot is `scripts/shot.sh <name> --scene "<frame>"` on `phase/10-ship` at `2da5a90` (BERTH_MOCK=1), read beside the frame PNG in `design/shots/figcraft/screens/`. Only broken, unreadable, unreachable, clipped/overlapping, wrong-state or wrong/misleading-copy differences are listed. Pure spacing, size, colour, fixture-data and fallback-font differences are skipped, except the one repeating pattern under "Skipped on purpose".

**Frame count: 22, not 19.** TASKS P10-T3 says 19; the P2-P4 task texts name 22 frames that exist as PNGs: P2 11 (Containers bulk / confirm delete / engine stopped / loading / selected 1024 / selected 1440 / selected 1440 light; Palette open; Projects default / default dark / empty), P3 4 (Container terminal, terminal no shell, Logs errors, Logs paused), P4 7 (Clean up sheet / done; Engines dashboard / starting; Images find / pulling / rate limited). The 19 in TASKS is probably 22 minus the three theme/size variants (1024, light, dark); all 22 were shot.

Shots live in `target/shots/` (git-ignored). Two retakes: `a03` was taken at the default 1440 window (the scene does not resize; `BERTH_WINDOW=1024x700` is needed), so `b03` replaces it; `a17` missed the 5 s toast because the machine was at load ~300, so `b17` replaces it.

## Summary

| Frame | Verdict | Must-fix |
|---|---|---|
| Containers / selected / 1440 | no must-fix | 0 |
| Containers / selected / 1440 / light | no must-fix | 0 |
| Containers / selected / 1024 | no must-fix | 0 |
| Containers / bulk / 1440 | no must-fix | 0 |
| Containers / confirm delete / 1440 | no must-fix | 0 |
| Containers / engine stopped / 1440 | no must-fix | 0 |
| Containers / loading / 1440 | no must-fix | 0 |
| Palette / open / 1440 | no must-fix | 0 |
| Projects / default / 1440 | 2 must-fix | 2 |
| Projects / default / 1440 / dark | 2 must-fix (same as default) | 2 |
| Projects / empty / 1440 | no must-fix | 0 |
| Container / terminal / 1440 | no must-fix | 0 |
| Container / terminal / no shell / 1440 | 1 must-fix | 1 |
| Logs / errors / 1440 | no must-fix | 0 |
| Logs / paused / 1440 | no must-fix | 0 |
| Clean up / sheet / 1440 | no must-fix | 0 |
| Clean up / done / 1440 | no must-fix | 0 |
| Engines / dashboard / 1440 | no must-fix | 0 |
| Engines / starting / 1440 | no must-fix | 0 |
| Images / find / 1440 | no must-fix | 0 |
| Images / find / pulling / 1440 | no must-fix | 0 |
| Images / find / rate limited / 1440 | no must-fix | 0 |

Must-fix items (3 distinct, 5 frame-hits):

1. **Projects Branch track clips the branch name** (`feat/che…`; the row wraps in some tiers). Projects default + dark. Known as AD58 item 6; the projects-row worker owns it. Fix: widen the track in `screens/table_tiers.rs` / `projects_tiers.css`, regenerate tests.
2. **"Set up project…" is shown disabled/greyed on the `docker-manger` row** (the frame draws it live; the row menu entry also does nothing, `ui/project_row.rs:841`). Projects default + dark. Fix: build it or hide it (projects-row worker).
3. **"Open a debug shell…" is a permanently disabled button** on Terminal / no shell (`ui/terminal.rs:780`, `disabled: true`, tooltip "Debug shells are not built yet") and the note under it ("Starts a temporary busybox container…") describes a feature that does not exist. No task schedules it. Fix: hide the button and the note until AD28 helper-container shells exist (or schedule the task).

## Skipped on purpose (repeats on most frames, not must-fix)

- Table rows are 32 px (token `--size-row`, design-system.md says 32) where the frames draw 28 px; page header and filter chip sit 4-8 px lower; page side inset 20 px vs 16 px.
- Sidebar shows Plugins and Settings where the frames show a collapsed COOLIFY group (accepted, STATUS AD55).
- Table column-header band uses the page colour; the frame uses the sunken colour.
- Container order inside a project is alphabetical; the frames order by hand.
- Frame fixtures differ (Unassigned group, activity lines, times). Logs times show local time (16:02) where the frames show 14:02.
- In shots a13-a16 the traffic lights are grey: the window was not frontmost during capture, not an app difference.
- The primary button blue reads 50,115,203 in the capture vs 0,117,210 in the frame; a capture colour-profile effect.

## Containers / selected / 1440
- Shot: `target/shots/a01-containers-selected-1x.png` (2x: `a01-containers-selected.png`)
- Frame: `design/shots/figcraft/screens/Containers _ selected _ 1440.png`
- Verdict: no must-fix. Inspector Delete… wraps to a second line and Pause is disabled (the container is restarting); "Fix with Claude Code" is greyed because a scene runs no assistant; Networks gained Disconnect / Connect to network after P4. None hurts use.

## Containers / selected / 1440 / light
- Shot: `target/shots/a02-containers-selected-light-1x.png`
- Frame: `design/shots/figcraft/screens/Containers _ selected _ 1440 _ light.png`
- Verdict: no must-fix. Same notes as the dark frame; light tokens read correctly (selection tint, red exit code, primary button).

## Containers / selected / 1024
- Shot: `target/shots/b03-containers-1024-1x.png` (1024x700, `BERTH_WINDOW=1024x700`)
- Frame: `design/shots/figcraft/screens/Containers _ selected _ 1024.png`
- Verdict: no must-fix. Inspector hidden with the Inspector button in the header, Uptime dropped, names truncate like the frame.

## Containers / bulk / 1440
- Shot: `target/shots/a04-containers-bulk-1x.png`
- Frame: `design/shots/figcraft/screens/Containers _ bulk _ 1440.png`
- Verdict: no must-fix. Four selected, Start 0 disabled, Stop 4 / Restart 4 / Pause 3 / Delete 4… / Clear all present; the frame's blank status on selected rows is a frame defect (the app shows Restarting).

## Containers / confirm delete / 1440
- Shot: `target/shots/a05-containers-confirm-delete-1x.png`
- Frame: `design/shots/figcraft/screens/Containers _ confirm delete _ 1440.png`
- Verdict: no must-fix. Note (not must-fix, copy is shorter, not wrong): the app says "Its writable layer is deleted." and has no "Removes 12 MB · Keeps 1.2 GB" line; the mock container carries no layer size. Worth a look when the EffectSummary is next touched.

## Containers / engine stopped / 1440
- Shot: `target/shots/a06-containers-engine-stopped-1x.png`
- Frame: `design/shots/figcraft/screens/Containers _ engine stopped _ 1440.png`
- Verdict: no must-fix. Banner, empty state, Start engine default, and the command line text match.

## Containers / loading / 1440
- Shot: `target/shots/a07-containers-loading-1x.png`
- Frame: `design/shots/figcraft/screens/Containers _ loading _ 1440.png`
- Verdict: no must-fix. Eight skeleton rows, "Checking…" top and bottom.

## Palette / open / 1440
- Shot: `target/shots/a08-palette-open-1x.png`
- Frame: `design/shots/figcraft/screens/Palette _ open _ 1440.png`
- Verdict: no must-fix. The first result lacks the right-aligned ⌘R hint the frame shows (should-fix, not asked for here); groups, copy, footer keys match.

## Projects / default / 1440
- Shot: `target/shots/a09-projects-default-1x.png`
- Frame: `design/shots/figcraft/screens/Projects _ default _ 1440.png`
- Verdict: 2 must-fix (the projects-row worker is editing these areas, so re-shoot after their merge).
  1. Branch track clips: `feat/che…` on the shop-api row (`screens/table_tiers.rs`, `projects_tiers.css`; AD58 item 6). Proposed fix: widen the Branch track so the longest mock branch fits, or let the cell show the full name on hover and shrink Path instead.
  2. `docker-manger` row shows "Set up project…" greyed, and its menu item does nothing (`ui/project_row.rs:841-846`). The frame shows it as a live button. Proposed fix: build it, or hide until built.
- Also seen (not must-fix): Path is proportional where the frame draws it monospace; Needs attention is one bordered list where the frame draws separate cards; the Needs-attention/problem row has no left accent bar.

## Projects / default / 1440 / dark
- Shot: `target/shots/a10-projects-default-dark-1x.png`
- Frame: `design/shots/figcraft/screens/Projects _ default _ 1440 _ dark.png`
- Verdict: 2 must-fix, same two as the default frame (Branch clip, greyed Set up project…). Dark tokens read correctly (problem row tint, warn/error activity bars).

## Projects / empty / 1440
- Shot: `target/shots/a11-projects-empty-1x.png`
- Frame: `design/shots/figcraft/screens/Projects _ empty _ 1440.png`
- Verdict: no must-fix. The frame's own text overlap ("…writes one for you" under the button) is a frame defect; the app renders it cleanly. App reads "Muse Code" where the frame says "muse" (app is right per CLAUDE.md).

## Container / terminal / 1440
- Shot: `target/shots/a12-container-terminal-1x.png`
- Frame: `design/shots/figcraft/screens/Container _ terminal _ 1440.png`
- Verdict: no must-fix. Not must-fix: no block cursor after the last prompt, working dir is an editable field, terminal line height 16 vs 18 (already known: "needs a bundle rebuild"), Logs/Terminal tabs have no per-tab ✕ (one panel ✕ on the far right instead).

## Container / terminal / no shell / 1440
- Shot: `target/shots/a13-container-terminal-noshell-1x.png`
- Frame: `design/shots/figcraft/screens/Container _ terminal _ no shell _ 1440.png`
- Verdict: 1 must-fix. "Open a debug shell…" is disabled with the tooltip "Debug shells are not built yet" (`ui/terminal.rs:780`), and the note below it describes the unbuilt feature. Frame: enabled. Proposed fix: hide the button and note until the feature exists (or schedule it). Files works.

## Logs / errors / 1440
- Shot: `target/shots/a14-logs-errors-1x.png`
- Frame: `design/shots/figcraft/screens/Logs _ errors _ 1440.png`
- Verdict: no must-fix. Chips, level filters, 2 lines selected, search 4 of 4, highlighted ECONNREFUSED, Next error F8 all present; "Ask Claude Code" is greyed in a scene.

## Logs / paused / 1440
- Shot: `target/shots/a15-logs-paused-1x.png`
- Frame: `design/shots/figcraft/screens/Logs _ paused _ 1440.png`
- Verdict: no must-fix. "142 new lines" pill replaces Ask/Copy row correctly; Follow is off.

## Clean up / sheet / 1440
- Shot: `target/shots/a16-cleanup-sheet-1x.png`
- Frame: `design/shots/figcraft/screens/Clean up _ sheet _ 1440.png`
- Verdict: no must-fix. Checked rows, sizes, explanatory notes, button "Remove 13 items (5.7 GB)" and the unchecked volumes row all match; the frame's overlap on "project3 containers" is a frame defect.

## Clean up / done / 1440
- Shot: `target/shots/b17-cleanup-done-1x.png` (retake; `a17-cleanup-done-1x.png` missed the toast because of load)
- Frame: `design/shots/figcraft/screens/Clean up _ done _ 1440.png`
- Verdict: no must-fix. Disk 16.8 of 60 GB, 0 B reclaimable, Clean up… greyed, toast "Removed 13 items · 5.7 GB freed" shown. Note: the toast sits 7 px into the status bar (its lower edge is at y 884; the frame floats it above), close to the "3 problems" chip but not covering text; the toast is light where the frame's is dark.

## Engines / dashboard / 1440
- Shot: `target/shots/a18-engines-dashboard-1x.png`
- Frame: `design/shots/figcraft/screens/Engines _ dashboard _ 1440.png`
- Verdict: no must-fix. The app adds Restart engine… (P8); Clean up… is primary with 5.7 GB reclaimable as the frame shows.

## Engines / starting / 1440
- Shot: `target/shots/a19-engines-starting-1x.png`
- Frame: `design/shots/figcraft/screens/Engines _ starting _ 1440.png`
- Verdict: no must-fix. Progress 40%, step 3 of 5, 8 s of about 20 s, the log lines and the note all match. Status bar says "Starting…" where the frame has "Starting… 8 s" (copy detail).

## Images / find / 1440
- Shot: `target/shots/a20-images-find-1x.png`
- Frame: `design/shots/figcraft/screens/Images _ find _ 1440.png`
- Verdict: no must-fix.

## Images / find / pulling / 1440
- Shot: `target/shots/a21-images-find-pulling-1x.png`
- Frame: `design/shots/figcraft/screens/Images _ find _ pulling _ 1440.png`
- Verdict: no must-fix. Row pull bar, tab "1 pulling", layer list and Cancel pull match.

## Images / find / rate limited / 1440
- Shot: `target/shots/a22-images-find-rate-limited-1x.png`
- Frame: `design/shots/figcraft/screens/Images _ find _ rate limited _ 1440.png`
- Verdict: no must-fix. Banner, Keychain line, "5 results · cached 6 min ago" and "Pulls may also be limited" match.
