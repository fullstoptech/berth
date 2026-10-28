# P10-T4 design audit, part 2: P5-P9 frames and no-frame screens vs fresh shots (2026-10-28)

Scope (owner, AD67): must-fix only: broken, unreadable, unreachable, clipped or overlapping, wrong state, misleading copy. Spacing, sizes, colours that do not hurt use, and the fallback face (IBM Plex is not installed system-wide) are not findings. Shots: `scripts/shot.sh <name> --scene "<frame>"` on `phase/10-ship` at `c1580fa` (BERTH_MOCK=1), 2x PNG plus `-1x` copy in `target/shots/` (git-ignored); frames in `design/shots/figcraft/screens/`. Known accepted deviations (Coolify failed row Logs button, Warning: callout outline, In progress ring-dot, Branch track 168 px) and the three items another worker is fixing (registry login loopback host / duplicate error, Export save-panel default folder, Coolify Logs replacing the inspector) are not re-reported.

**Frame count: 12, not 15.** `design/shots/figcraft/screens/` holds 34 PNGs; P10-T3 covered 22 of them (Containers 7, Palette 1, Projects 3, Container terminal 2, Logs 2, Clean up 2, Engines 2, Images find 3). The remaining 12 are the P5-P9 frames: Menu bar failing 320 and dark (P6), New project detected (P5), Open folder browser and no access (P5), Project assistant (P7), Project files (P8), Projects drop folder (P5), Review changes default and new project (P5), Coolify deployments and confirm redeploy (P9). TASKS' 15 is a count made before the frames were exported (PLAN section 12 lists the same 12); no P5-P9 frame is missing a PNG.

## Summary

| Screen / frame | Verdict | Note |
|---|---|---|
| Menu bar / failing / 320 | covered by test (native menu) | not shot; `menubar_failing_frame_has_the_frames_order_and_text` |
| Menu bar / failing / 320 / dark | covered by test (native menu) | same model, native menu recolours |
| New project / detected / 1440 | matches | |
| Open folder / browser / 1440 | matches | |
| Open folder / browser / no access / 1440 | matches | |
| Project / assistant / 1440 | matches | only the intended "Open in Terminal" ghost button differs (P7-T6) |
| Project / files / 1440 | matches | |
| Projects / drop folder / 1440 | matches | |
| Review changes / default / 1440 | matches | |
| Review changes / new project / 1440 | matches | |
| Coolify / deployments / 1440 | matches | accepted deviations only; re-shot after the fixes, no regression |
| Coolify / confirm redeploy / 1440 | matches | accepted deviations only |
| Image detail (Images / selected, 1440 + 1024, run, pulling, confirm delete) | matches | dark also read |
| Build image sheet (folder, Dockerfile, running, failed, done, pushing) | matches | **Dockerfile editor readable in dark** |
| Save as image (inline bar: idle, running, failed, invalid, saved) | matches | |
| Volumes (list, selected, 1024, Files browser, helper download, create, confirm delete) | matches | dark shots |
| Networks (default, selected, create) | matches | light and dark |
| Templates (list, light, no custom, delete confirm) | matches | |
| Activity (live, paused) | matches | |
| Registries | matches | login sheet fixed: loopback http, hint line, error once (shot `p10t4-registry-login`; error-once covered by test) |
| Settings (light, dark) | matches | |
| First run / default stopped | matches | |
| First run / colima missing | fixed | title bar "No engine", status bar "No engine found · Finish setup", no spinner (shot `p10t4-noengine`) |
| Plugins (empty, installed, enabled, error) + Enable / Configure sheets | matches | |
| From Git sheet (form, cloning, clone failed, invalid compose, wrong address) | matches | light and dark |
| New project sheet (chooser, editor), Detected sheet | matches | |

Open must-fix: **0**. The one must-fix found (First run / colima missing) is fixed and re-shot, see "Fixes and re-shoot". Other items closed by the fixes: Coolify Logs replacing the inspector (Back control, row stays selected), Export save panel default folder (covered by test, native panel). Accepted deviations, by name: Coolify failed row "Logs" as a bordered button (the action-cell convention); "Warning:" callout outline (pixel only); "In progress" ring-dot (consistency with the other status badges); Menu bar frames are "covered by test (native menu)".

## Must-fix (found by the audit; closed below)

1. **First run / colima missing: the chrome says "Checking..." for ever.** Shot `target/shots/n30-firstrun-missing-1x.png` (dark: `d09-firstrun-dark-1x.png`). The Set up Berth card has finished its check and says Colima "not found", but the title bar shows a spinning "Checking..." chip, the status bar says "Looking for engines..." plus the same spinner, and the sidebar shows "0" counts as if still loading. With no engine discovered `connection()` returns `Connecting` (`crates/berth-app/src/ui/status_bar.rs:56-58`, label at `:74` and `:138-141`), which is also the state a real launch is in before the first answer, so a Mac without Colima never leaves it. Misleading: it reads as "still working" next to a result that says "install it". Fix: when the first-run check has completed and no engine exists, give `Connection` a `NoEngine` variant (chip "No engine", status "No engine found", no spinner), or end `Checking...` once setup has reported.

## Fixes and re-shoot (c13fdb4 and the NoEngine commit)

Shots: `scripts/shot.sh <name> --scene "<scene>"` (BERTH_MOCK=1) on `phase/10-ship`, in `target/shots/` (git-ignored).

- **No engine instead of an endless check (must-fix, fixed).** `connection()` takes a third argument, `discovered`; `AppState.engines_discovered` is set by the engines applier on `EnginesDiscovered` (an empty list included). Empty list and not discovered: `Connecting` (title bar "Checking..." with spinner, status bar "Looking for engines..."). Empty list and discovered: new `Connection::NoEngine`: title-bar chip "No engine" (warning triangle, muted, no spinner), status bar "No engine found · Finish setup", Problems tab empty state "No engine". One or more engines: unchanged. Shot `p10t4-noengine-1x.png` ("First run / colima missing / 1440") beside the earlier `n30` shot: the Set up Berth card is unchanged; top right reads "No engine", bottom left "No engine found · Finish setup", bottom right "No engine"; no spinner anywhere. Scene artefact, not a product bug: the engine switcher still says "colima:default" because the scene empties `engines` but keeps the mock scope; a real launch with no engine is scoped to All. Verdict: fixed.
- **Registry login (fixed).** Shot `p10t4-registry-login-1x.png` (new dev scene "Dev / registry login", the Add registry sheet over the list). The URL field now has the hint "A bare localhost or 127.0.0.1 address uses plain http." under it; the placeholder still shows https. The refused-login error renders once, under the Password field only (the second `InlineMessage` is gone); a scene cannot reach the refused state without a registry service, so that part is covered by `registry_login_tests.rs`. Verdict: fixed.
- **Coolify deployment log with Back (fixed).** Shot `p10t4-coolify-log-back-1x.png` (new dev scene "Coolify / log with Back / 1440": the failed deployment's log opened from a card of its application pane). The pane shows a "Back" control with a chevron above "Deployment log"; the landing row stays selected in the table. The row header is not repeated over the log. Verdict: fixed. Back/selection logic covered by `plugin_view_tests.rs`.
- **Coolify / deployments / 1440 re-shot** (`p10t4-coolify-deployments-1x.png`) beside the frame PNG: layout, Redeploy.../Logs on the failed row, the recorded log in the pane are unchanged. Differences are the accepted ones (Logs as bordered button, In progress ring-dot) plus the fallback face; log times read 16:30 where the frame has 14:30 because scenes format in local time (not a must-fix). Verdict: matches, no regression.
- **Export panel**: native save panel; its default (Downloads) is covered by the `downloads_dir` unit test and the build-image sheet test, not shot.

## Per item

### P5-P9 frames (shots t4-01 to t4-10, each read beside its frame)
- **New project / detected**: `t4-01-newproj-detected-1x.png`. Found list, stack, services needed, "Berth does not change compose.yaml", Start project: all present and readable.
- **Open folder / browser** and **no access**: `t4-02-openfolder-browser-1x.png`, `t4-03-openfolder-noaccess-1x.png`. Column browser, path, the system-settings pointer and Retry are all there.
- **Project / assistant**: `t4-04-project-assistant-1x.png`. Same layout; the extra ghost button left of Watch is the AD38 Open in Terminal.
- **Project / files**: `t4-05-project-files-1x.png`. Tree, preview and the proposed-change card match.
- **Projects / drop folder**: `t4-06-projects-drop-1x.png`. Overlay text and drop target match.
- **Review changes / default** and **new project**: `t4-07-review-default-1x.png`, `t4-08-review-newproj-1x.png`. Diffs, checks line, Write files and start / Cancel all present.
- **Coolify / deployments** and **confirm redeploy**: `t4-09-coolify-deployments-1x.png`, `t4-10-coolify-confirm-1x.png`. Only the accepted deviations differ.
- **Menu bar / failing / 320 (+dark)**: not shot. The tray menu is a native `muda`/NSMenu (PLAN D38, D80: "the 320 px frames are the content guide, not a pixel target") and is not a window the shot script can capture. Its content is pinned by `menubar_failing_frame_has_the_frames_order_and_text` in `crates/berth-app/src/services/menubar_tests.rs:212` (summary "1 failing · 1 unhealthy · 1 engine unreachable", Engines default/work/hetzner-1 with Stop.../Start/Retry, Projects shop-api with nested shop-api-db-1 and shop-api-worker-1, analytics, berth-site, Open Berth, Quit Berth). STATUS (Phase 6 acceptance) records the owner's live check of the item in both appearances. Difference from the frame, not a must-fix: project rows also offer "Open in Berth".

### No-frame screens, read against the framed screens' conventions
- **Image detail** (`n01`, `n02` at 1024, `n03` run, `n04` pulling, `n05` confirm delete, dark `d08`): inspector header, action row (Run, Pull latest, Push..., Tag, Export...), Overview/Layers/Containers tabs, Inspector toggle at 1024 all consistent with Containers. The Image column start-truncates with the same ellipsis convention as the Containers frames.
- **Build image sheet** (`n06`..`n11`, dark `d02`, `d01`): Folder/Dockerfile switch, tags, args, platform, build log with the failing step marked, Open Problems / Edit and rebuild, Build another / Push next, push progress with Cancel push. All readable and reachable.
- **Dockerfile editor in dark theme: readable.** Evidence: scene shot `d01-dockerfile-dark-1x.png` (the scene deliberately does not start CodeMirror, so the editor box is an empty sunken panel there) and a live run of the real app (`BERTH_THEME=dark`, Images, Build image..., Write a Dockerfile; a 2x crop is at `/private/tmp/claude-501/-Users-emmanuel-Dev-app-ideas-docker-manger/b4ebf0b2-f2c7-49af-a1ba-65faa23f241c/scratchpad/live2d.png`). Starter text `FROM alpine:3` / `CMD ["echo", "hello"]` shows keywords in pale blue, strings green, the number amber, light ink on a near-black sunken surface, grey line numbers, a visible active-line band. Not seen live: typed text (my synthetic key events did not reach CodeMirror) and a selection; by `web-src/src/editor.ts:121-155` every editor colour is a theme variable (comments and gutter use `--color-ink-muted`, `#898d91` in dark, on the sunken surface), so they follow the theme and comments are readable.
- **Save as image** (`p08` idle, `p09` running, `p10` failed, `p11` invalid, `p12` saved): inline bar in the inspector; error text and disabled Save as image are clear.
- **Volumes** (`n12` list, `n13` selected, `n14` 1024 with Inspector toggle, `n15` Files browser, `n16` helper download card, `n17` Create volume, `n18` typed-name delete confirm): all readable; Delete... is disabled for an in-use volume next to the "In use by 1" badge; the confirm names the volume and the size.
- **Networks** (`n19`, `n20`, `n21`): Create network validation message ("Host bits are set. Did you mean 172.31.250.0/24?") readable, Create network disabled while it shows; built-in networks show "Built-in" in place of a delete.
- **Templates** (`n22` dark, `n23` light, `n24` empty custom, `n25` delete confirm, dark `d06`): fine; the empty Custom group says where to save one.
- **Activity** (`n26` live, `n27` paused, dark `d07`): paused banner and Resume feed, the gap row and Failed outcomes readable.
- **Registries** (`n28`, dark `d04`): the "Add registry..." button is disabled in the dev scene only because it runs with no registry service; the empty and list states read well. Row actions (Log out, Remove, Log in...) are bare text where Coolify's action cells use bordered buttons: noted, not a must-fix.
- **Settings** (`n29` light, `d03` dark): groups, theme switch with the selected value, density, inspector side, engines and assistants rows legible in both themes; the Assistants card runs past the 900 px window and scrolls with the page (`shell.css:42`).
- **First run** (`n30`, `n31`, dark `d09`): see the must-fix for `colima missing`. `default stopped` is right: Colima row "0.10.3 · stopped" with a warning icon, Start default, Check again, Continue, title bar "Engine stopped".
- **Plugins** (`p01` empty, `p02` installed, `p03` enabled, `p07` error, dark `d05`) and sheets (`p04` Enable Coolify, `p05` Configure new, `p06` Configure saved): permissions list, "Needs setup: Coolify address", the honest "Berth shows these but does not block other addresses in this version" note, token field with "Saved ... Leave empty to keep it" and Remove saved value, error card with last output and Try again. All readable.
- **From Git** (`p13` form, `p14` cloning, `p15` clone failed, `p16` invalid compose, `p17` wrong address in dark), **New project chooser** (`p18`), **editor** (`p19`), **Detected** (`p20`): every state has a way forward (Back, Cancel, Edit the address, Open folder, Close) and plain copy.

## Not shot
- Menu bar frames (native menu, see above).
- Export save panel: native NSSavePanel, covered by the `downloads_dir` test in `crates/berth-sys/src/paths.rs` and the Export default-folder test in `build_image_tests.rs`.
- Light-theme live Dockerfile editor: the scene does not start CodeMirror and only the dark live run was done; the same variable mapping applies.
- Settings below the fold (Assistants rows after Muse Code) and the lower Networks create list: scrolling regions, not captured.

## Method notes
- Scenes used: every name in the "Shots" lists above is a scene in `crates/berth-app/src/scenes/*.rs`; dark variants via `BERTH_THEME=dark`, 1024 via `BERTH_WINDOW=1024x700`.
- A berth process (pid 97267) was already running before this audit; it was not touched. The live editor run was a separate instance with a temp `BERTH_DATA_DIR`, stopped afterwards.
- No code, CSS or docs other than this file were edited; nothing was committed.
