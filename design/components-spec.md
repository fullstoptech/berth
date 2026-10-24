# Berth component spec (for the Components page)

Every component reads role tokens only (`design/tokens.json`), uses auto layout, and is named after the code component it becomes in `crates/app/src/ui/`. Props are unions, never boolean soup. Sizes: controls sm 22 / md 26 / lg 32, rows 28, icons 14 / 16, radius sm 3 / md 5 / lg 8 / xl 12. Icon set: **Lucide**, 1.5px stroke, 14 or 16px, one icon per meaning everywhere.

Status vocabulary (the domain's own state names; same words in badges, filters and copy):
`Running` · `Healthy` · `Starting` · `Restarting` · `Unhealthy` · `Paused` · `Exited (0)` · `Exited (137)` (any non-zero code) · `Created` · `Removing` · `Dead`.
Tone map: Running/Healthy → success; Starting/Restarting/Unhealthy/Paused → warning; Exited non-zero/Dead → danger; Exited (0) → stopped (neutral); Created → info; Removing → stopped.

| Component (code name) | For / not for | Variants and props | States |
|---|---|---|---|
| `Button` | Commands with a label (verb + object) / not navigation | `variant: primary | secondary | ghost | danger`, `size: sm | md`, optional leading icon, optional `Kbd` | default, hover, pressed, focus-visible, disabled (tooltip with reason), loading (spinner replaces icon, label kept: "Stopping…") |
| `IconButton` | Row and toolbar commands / not the primary action | `icon`, `tone: neutral | danger`, `size: sm (22) | md (26)`, required accessible name, tooltip "Restart · ⌘R" | default, hover, pressed, focus-visible, disabled, loading |
| `Kbd` | Shortcut hint / not a button | glyphs ⌘ ⇧ ⌃ ⌥ ⌫ ↵ as icons, letters in label style | — |
| `StatusDot` | State inside dense rows / never alone without text nearby | `state` (vocabulary above); Restarting and Starting show a 2-segment ring, Paused shows a hollow dot so state survives greyscale | each state |
| `StatusBadge` | State as text in detail headers and filters / not interactive | `state`; subtle background + text + dot | each state, light and dark |
| `Badge` | Counts and tags / not status | `kind: count | tag | arch` (arch "amd64" uses warning tone) | — |
| `TextField` | Free input / not choosing | label above, input, help or error below, optional prefix icon, optional suffix (`Kbd`) | default, hover, focus, filled, error, disabled |
| `SearchField` | Filtering a list / not global search (use `PaletteTrigger`) | search icon, placeholder "Filter containers", clear button, match count "12 of 48" | default, focus, filled, no matches |
| `Select` | One of many / not 2–4 options (use `SegmentedControl`) | label, trigger, chevron | closed, open (menu), focus, disabled |
| `SegmentedControl` | 2–4 peer views or modes / not navigation | items with optional count | default, selected, hover, focus |
| `Checkbox` | Row selection, options | square, radius sm | unchecked, checked, mixed, hover, focus, disabled |
| `Switch` | Immediate on/off (Follow, Wrap, Timestamps) / not options needing Save | — | off, on, focus, disabled |
| `Tabs` | Peer views of one object (inspector, bottom panel) / not steps | `variant: underline | panel` (panel tabs are the bottom panel's, with close × and count) | default, hover, selected, focus |
| `SidebarItem` | Navigation destination / not actions | icon, label, optional count, optional status dot (projects), indent level | default, hover, selected, focus |
| `TitleBar` | Window chrome / one global action only | traffic lights, `EngineSwitcher`, `PaletteTrigger`, Problems count | default, engine stopped (switcher shows Stopped) |
| `EngineSwitcher` | Pick the engine or "All engines" | engine name, kind icon (Colima / SSH), StatusDot, chevron | default, open menu, stopped, unreachable |
| `PaletteTrigger` | Opens the command palette | search icon, "Search or run a command…", `Kbd` ⌘K | default, hover, focus |
| `TableHeaderCell` | Column header | label, sort arrow, right-aligned variant for numbers | default, hover, sorted asc, sorted desc |
| `ContainerRow` | One container in a table | Checkbox, StatusDot, name (mono for generated names), project, image:tag (mono), ports (mono, link style), CPU % and MEM (tabular, right-aligned, with micro bar), uptime, **row actions**: Start/Stop toggle, Restart, Logs, Terminal, Delete, More | default, hover (actions visible), selected (actions always visible), action loading ("Stopping…"), error (inline message under the row), disabled action (tooltip) |
| `ProjectRow` | One project (folder) | folder icon, name, path (muted mono, middle-truncated), engine, git branch, service dots (one per service), ports summary, CPU/MEM, **actions**: Start project/Stop project (labelled), Restart, Logs, More | default, hover, selected, partial, failing, not created (pinned) |
| `GroupHeaderRow` | Group rows by project or engine in a table | chevron, name, counts "3 running · 1 exited", group actions | expanded, collapsed |
| `BulkBar` | Actions on 2+ selected rows | "4 selected", Start 2, Stop 2, Restart 4, Pause 4, Delete 4, Clear | appears when 2+ selected; partial applicability counts |
| `InspectorHeader` | Container or project detail header | name, StatusBadge, image, ID (mono, copy), labelled buttons Start/Stop, Restart, Pause, Kill (danger), Delete (danger), More | per state: running, exited, restarting loop |
| `KeyValueList` | Details of one record / not comparison | label (muted) + value (mono where technical), copy on hover | default, long value (wraps), empty value ("Not set") |
| `MetricTile` | One live figure with sparkline | label, value (tabular), unit, sparkline (5 min), limit | default, loading, unavailable ("—" with reason) |
| `UsageBar` | Disk or memory share by type | segments (images, containers, volumes, build cache), legend with sizes | default, nearly full (warning) |
| `LogLine` | One line in a log view | gutter marker, timestamp (muted mono), container name (series colour), stream marker for stderr, message (mono) | stdout, stderr, error-marked (danger-subtle background + gutter mark), warn, JSON collapsed (summary + chevron), stack trace folded ("12 more lines"), search match highlight, selected (for "Ask about these lines") |
| `LogToolbar` | Controls for a log view | Follow switch, SearchField with "3 of 41", level chips (Error 7, Warn 12, Info, Debug), stream select, time range select, Next error (F8), Wrap, Timestamps, Copy, Save | default, paused ("142 new lines" pill) |
| `Banner` | Persistent condition of a page or engine | `tone: info | success | warning | danger`, icon, message, one action | each tone; dismissible or not |
| `InlineMessage` | Result of a row action / not page-level | icon + text, under the row | danger, warning |
| `ProblemItem` | One detected problem | severity icon, title, subject, age, explanation, evidence lines (mono), fix buttons ("Map Postgres to port 5433", "Fix with Claude Code") | default, expanded, fix running, fix failed |
| `Toast` | Confirmation of the user's own non-routine action | message, optional Undo/View | entering, visible |
| `EmptyState` | First run, no results / not an apology | icon, title, one-line payoff, one primary action | first run, filtered empty, engine stopped |
| `SkeletonRow` | Table loading after 300 ms | shapes matching ContainerRow | — |
| `ProgressBar` | Determinate work (pull, build, clean up) | label, value, bytes "312 MB of 1.2 GB" | determinate, indeterminate |
| `Tooltip` | Name + shortcut of an icon button; reason for disabled | text, `Kbd` | — |
| `Menu` | More actions, context menu | items with icon, label, Kbd; separators; danger item | default, item hover, disabled item with reason |
| `ConfirmDialog` | Destructive confirm | title "Delete container api-db-1?", effect list (what goes, what stays, sizes), checkbox option, buttons Cancel + "Stop and delete" (danger) | default, typed-name variant for volumes and prune |
| `DiffView` | Review changes per file | file header (path, +12 −3), hunk lines: added, removed, context, line numbers (mono) | default, conflict |
| `ChatTurn` | Assistant conversation | user turn (right-aligned text block), assistant turn (markdown text), streaming caret | default, streaming |
| `ToolCallRow` | A harness tool call inside a turn | tool icon, sentence "Read 200 lines of api-1 logs", risk tag (Read / Change / Destructive), duration, chevron | running, done, expanded (args + result), failed |
| `ApprovalCard` | Change or destructive tool waiting for the user | what will happen in one sentence, command (mono), buttons Approve + Deny; destructive asks to type the name | waiting, approved, denied |
| `ProposedChangeCard` | A proposed file change | files list with +/−, "Review changes" button | pending, applied, discarded |
| `ContextChip` | Attached context | icon, label "api-1 · 24 lines", remove × | default, hover |
| `HarnessPicker` | Choose Claude Code or muse | name, version, path on hover | detected, not found |
| `PaletteItem` | One result in the command palette | icon, label (verb + object or record name), scope (muted), Kbd | default, active |
| `MenuBarMenu` | The macOS menu bar dropdown | header state, engines with Start/Stop, projects with Start/Stop, failing containers with Show logs, Open Berth, Quit | all good, something failing |

Accessibility annotations to add on each component: role, accessible name pattern (e.g. IconButton "Restart api-db-1"), keyboard (Tab into row actions on the selected row; Space toggles selection; ⌫ Delete), focus ring 2px `focus` outside.
