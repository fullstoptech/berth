# P10-T3 design audit, part 1: P2-P4 frames vs fresh shots (2026-10-28)

Scope (owner, must-fix only): each shot is `scripts/shot.sh <name> --scene "<frame>"` on `phase/10-ship` at `2da5a90` (BERTH_MOCK=1), read beside the frame PNG in `design/shots/figcraft/screens/`. Only broken, unreadable, unreachable, clipped/overlapping, wrong-state or wrong/misleading-copy differences are listed. Pure spacing, size, colour, fixture-data and fallback-font differences are skipped, except the one repeating pattern under "Skipped on purpose".

**Frame count: 22, not 19.** TASKS P10-T3 says 19; the P2-P4 task texts name 22 frames that exist as PNGs: P2 11 (Containers bulk / confirm delete / engine stopped / loading / selected 1024 / selected 1440 / selected 1440 light; Palette open; Projects default / default dark / empty), P3 4 (Container terminal, terminal no shell, Logs errors, Logs paused), P4 7 (Clean up sheet / done; Engines dashboard / starting; Images find / pulling / rate limited). The 19 in TASKS is probably 22 minus the three theme/size variants (1024, light, dark); all 22 were shot.

Shots live in `target/shots/` (git-ignored). Two retakes: `a03` was taken at the default 1440 window (the scene does not resize; `BERTH_WINDOW=1024x700` is needed), so `b03` replaces it; `a17` missed the 5 s toast because the machine was at load ~300, so `b17` replaces it.

## Summary (final verdicts after the re-shoot, 2026-10-28, HEAD 4a2c0de)

Verdict key: matches = no must-fix, nothing changed since the first pass; fixed = the first-pass must-fix is closed; deviation = differs from the frame on purpose. The two regressions the re-shoot found are fixed (section "Regressions fixed" at the end). 22 frames, 22 verdicts; no open must-fix.

| Frame | Final verdict | Note |
|---|---|---|
| Containers / selected / 1440 | matches | not re-shot; unchanged areas |
| Containers / selected / 1440 / light | matches | not re-shot |
| Containers / selected / 1024 | fixed | Inspector toggle present, overlay reachable (c08, c12); was already in the frame, now also on the other three screens |
| Containers / bulk / 1440 | matches | not re-shot |
| Containers / confirm delete / 1440 | matches | not re-shot; copy note only |
| Containers / engine stopped / 1440 | matches | re-shot c05; Start engine default, banner, command line unchanged |
| Containers / loading / 1440 | matches | not re-shot |
| Palette / open / 1440 | matches | not re-shot |
| Projects / default / 1440 | deviation | Set up project... live (fixed); Branch track is 168 px where the frame draws 98 px (deviation: branch names were unreadable at 98); Restart, Stop/Start, the list and ... buttons all visible (d01) |
| Projects / default / 1440 / dark | deviation | same as light (d02) |
| Projects / empty / 1440 | matches | re-shot c04; unchanged |
| Container / terminal / 1440 | matches | bottom-panel tab strip and session tabs have padding, Terminal has the active underline (d03) |
| Container / terminal / no shell / 1440 | fixed | debug-shell button and note gone; tab strip has padding and underline (d04) |
| Logs / errors / 1440 | matches | re-shot d05; full-page screen, no bottom panel, unaffected |
| Logs / paused / 1440 | matches | not re-shot; same screen |
| Clean up / sheet / 1440 | matches | not re-shot |
| Clean up / done / 1440 | matches | not re-shot |
| Engines / dashboard / 1440 | matches | not re-shot |
| Engines / starting / 1440 | matches | not re-shot |
| Images / find / 1440 | matches | not re-shot |
| Images / find / pulling / 1440 | matches | not re-shot |
| Images / find / rate limited / 1440 | matches | not re-shot |

Original must-fix items (3 distinct, 5 frame-hits; all closed, see "Re-shoot after fixes"):

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


## Re-shoot after fixes (2026-10-28, HEAD 4a2c0de, BERTH_MOCK=1)

Shots in `target/shots/c01`..`c15` (`-1x.png` copies). 1024 shots used `BERTH_WINDOW=1024x700`. The bottom-docked Images shot (c13) was taken by seeding `inspector_dock: Bottom` and theme Dark into a scratch `berth.db` (no Images scene docks at the bottom), launching `Images / selected / 1440` against it and capturing with `BERTH_SHOT_ATTACH=1`. No code was changed.

| Item | Result | Evidence |
|---|---|---|
| Original 1: Branch clip | closed | c01/c02: `feat/checkout-v2` shows in full, row stays one line, nothing else in the row clips, Path/Engine/Services unchanged. Deviates from the frame on purpose (168 px, no ellipsis). |
| Original 2: greyed "Set up project..." | closed | c01/c02: docker-manger row shows a live bordered "Set up project..." button. Click behaviour (opens the Detected sheet) is covered by tests, not exercised in a shot. |
| Original 3: debug-shell button and note | closed | c03: only "Files" remains under "This image has no shell"; text reads "...Browse its files instead." |
| Inspector toggle below 1200 px | closed | c08 Containers, c09 Images, c10 Volumes, c11 Networks: a bordered "Inspector" button, same look and the last item of each controls row (on Images it sits on the buttons row because the header wraps at 1024). c12: overlay opens with the full inspector. |
| Inspector docked at the bottom, buttons | closed | c06/c07 Containers (scenes "inspector bottom" and "inspector bottom / alone"): Stop, Restart, Pause (disabled), Kill and Delete... have border and fill. c13 Images: Run, Pull latest, Push..., Tag, Export... and the header Import image.../Build image.../Pull image are bordered or filled. |
| Starting state (Start default) | not shot | no scene reaches it and clicking "Start engine" would run `colima start`; covered by `projects_tests.rs` (`projects_state_starting_engine_shows_the_starting_copy_and_a_disabled_button`, `..._click_on_start_engine_shows_starting_before_the_engine_reports_it`). |
| Projects / empty / 1440 | no regression | c04: empty block, Open folder, drop zone and first-run checks as in a11. |
| Containers / engine stopped / 1440 | no regression | c05: banner, empty state, "Start engine default", command line as in a06. |

### NEW must-fix 1: bottom-panel tabs and sub-tabs lost their styling (regression from a337bab)

Seen in c03, c06, c07, c13, c14 (every screen with the bottom panel open). `.bottom-panel button:not(.btn)` (shell.css:62) has specificity 0,2,1, which beats `.tab` rules such as `.bottom-head .tab` (shell.css:365), `.cd-tabs .tab` (container_detail.css:134) and `.im-tabs .tab` (images.css:52) at 0,2,0. Before the change the reset was 0,1,1 and lost to them. Effect: the Logs / Terminal / Problems / Assistant strip has no padding and no active-tab underline (compare a13, where Terminal is underlined), the terminal session tabs (`api-1 · bash x`, `+`) are crammed together, and the docked inspector's Overview / Logs / Stats... strip is bare text with no active marker. Still clickable, but the selected tab cannot be told apart. Proposed fix: write the reset as `.bottom-panel :where(button:not(.btn))` (specificity 0,1,0) or add `.tab`, `.tab-close` and `.icon-btn` to the `:not()` list.

### NEW must-fix 2: Projects row menu clipped off the right edge at 1440 (regression from e172166)

Seen in c01 and c02. On every row that has Restart plus Stop/Start project, the row-menu buttons (the list icon and the ... button) are pushed past the window edge: the list icon is cut in half and the ... button is not visible, so "Remove from Berth..." and the other row actions cannot be reached at 1440. The docker-manger row (one button) still shows both. Cause: the Branch track grew 98 -> 168 px (`--projects-cols` in projects.css:12) while the last track is `1fr`; the fixed tracks leave less than the actions need and the row overflows (a09 had them at x 1370 and 1398). Proposed fix: take the 70 px from tracks that have slack (Path 138, Services min/max 140-218, Ports 138) or give the actions track a `minmax(var(--projects-actions-w), 1fr)`, and regenerate the tier CSS and tests. Also check 1280 and 1100 widths.

## Regressions fixed (2026-10-28)

Both fixes are in `phase/10-ship` (commit "P10-T3: keep Projects row actions on screen; bottom-panel reset yields to tabs"). Shots `d01`..`d06` (`target/shots/`, `-1x.png` copies), BERTH_MOCK=1.

### 1. Projects row buttons clipped at 1440: closed

Cause, found by solving the grid by hand: Services is `minmax(140, 218)` and a grid shares free space out to such a track before a plain `1fr` track gets any. At 1440 (main 1220 px) the `1fr` buttons track got 155 px of the 228 px it needs; the tier maths had assumed Services stayed at its minimum. At the old 98 px Branch there were 70 px more to give, which hid it.

Fix: the last track of the full list is now `minmax(calc(160px * var(--text-scale, 1) + var(--projects-actions-w)), 1fr)` (`projects.css`), so Services cannot take the buttons' room; `table_tiers/projects.rs` counts the buttons in the minimums instead of as a reserve (the generated `projects_tiers.css` is byte-identical; thresholds did not move). Branch stays 168 px: 172 px is the most the 1440 frame can hold before the full list drops to the next tier (zero slack), 168 leaves 4 px and still holds `feat/checkout-flow`; longer names ellipsise. Projects has no inspector, so there is no second width to check. The tiers at their own minimum widths are covered by the new test (every tier, both views, every text size).

Evidence d01 / d02: Restart, Stop project / Start project, the list icon and the ... button all visible on every row; docker-manger shows Set up project... plus both icons.

### 2. Bottom-panel tabs lost padding and underline: closed

Fix: the reset in `shell.css` is `:where(.bottom-panel button:not(.btn))` and `:where(.bottom-panel button:not(.btn):focus-visible)`, specificity 0,0,0, so any component class (`.tab`, `.tab-close`, `.icon-btn`, `.bottom-head .tab`) wins in any order; bare buttons in the panel chrome still get the reset and `.btn` keeps its look. Nothing else relied on the old specificity (shots below).

Evidence: d03 Terminal tab underlined, Logs / Problems / Assistant padded, session tabs spaced with their close buttons; d04 same strip; d06 (Containers / inspector bottom) tab strip, Overview underline in the docked inspector, and Stop, Restart, Pause, Kill, Delete... still bordered and filled like buttons.

### Tests

- `projects_row_buttons_keep_their_whole_track_in_every_tier_down_to_its_minimum_width`, `projects_at_1440_show_the_branch_and_all_four_row_buttons`, `projects_table_wide_tracks_grow_with_the_text` (a small grid solver in the test resolves the tracks). Break-check: with the last track back to `1fr` they fail ("the buttons get 155 px of 228").
- `shell_bottom_panel_reset_has_zero_specificity_so_tabs_keep_their_rules`, `shell_specificity_helper_reads_where_not_and_classes`. Break-check: with the old selector the first fails ("`.bottom-panel button:not(.btn)` must not outrank a component class").
- `scripts/check.sh`: exit 0, 2745 tests run, 2745 passed, 70 skipped.
