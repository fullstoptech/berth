# Research log

Date: 2026-10-24

## Products referenced (patterns only, never layout, branding or copy)

| Product | Take | Never copy | How it was viewed |
|---|---|---|---|
| OrbStack | Menu bar item that starts/stops Compose projects as a group or per container; speed as the brand | Its VM-replacing model (Berth manages Colima, it does not replace it) | Docs: https://docs.orbstack.dev/menu-bar (search result, 2026-10-24); release note v2.2.3 fixing Compose projects splitting into several groups (https://releasebot.io/updates/orbstack) |
| Docker Desktop | Containers grouped by Compose project; extensions as a plugin precedent | Heavy single-window dashboard, marketing surfaces inside the app | From prior knowledge; not re-viewed this session |
| Portainer | Remote environments as a first-class switcher | Web-admin density and navigation depth | From prior knowledge; not re-viewed this session |
| Lazydocker | Keyboard-first, logs and stats beside the list | TUI constraints | From prior knowledge; not re-viewed this session |
| Zed | IDE shell: sidebar, main, inspector, bottom panel with tabs; command palette | Editor-specific chrome | From prior knowledge |
| Coolify | API v1 at `<instance>/api/v1` with bearer token; groups: Servers, Teams, Projects, Private Keys, Deployments, Applications, Databases, Services | n/a (plugin reads the API) | https://coolify.io/docs/api-reference/introduction (search result, 2026-10-24); community note that v4.2.0 moved state-changing endpoints to POST (verify against the official OpenAPI spec) |

## Not done this round
- No screenshots saved under `design/research/refs/`. Mobbin covers web and iOS apps, not desktop developer tools, so it was not searched. Reference screenshots of OrbStack, Docker Desktop and Zed are captured at the screens stage.

## Facts checked on this machine (2026-10-24)
- Colima 0.10.3; profile `default` running, aarch64, 2 CPU, 8 GiB, 60 GiB, runtime docker.
- Socket: `~/.colima/default/docker.sock` (also `containerd.sock`); Docker context `colima` points at it.
- `muse` at `~/.local/bin/muse` has `muse exec` (headless) and `muse serve` (MSP session host over stdio).

## Round 2 (2026-10-24)
- Coolify OpenAPI spec downloaded from https://raw.githubusercontent.com/coollabsio/coolify/v4.5.0/openapi.json (latest release v4.5.0, published 2026-10-23T10:41Z per the GitHub releases API). 212 paths; identical to `main` on 2026-10-24. Base URL `https://app.coolify.io/api/v1` (self-hosted: own host); bearer token. Endpoints used by the plugin are listed in brief 6.11. Coolify also exposes its own MCP server (`POST /mcp/enable`).
- Harness CLIs on this machine: Claude Code (`claude`, supports `-p --input-format stream-json --output-format stream-json --include-partial-messages --mcp-config --strict-mcp-config`); Muse Code 1.4.12 (`muse exec --json --workspace`, `muse serve` MSP session host over stdio, `muse schema` exports the wire schema).

## Round 3: screen references (2026-10-24)

All files are in `design/research/refs/`, each with a `.json` sidecar (source URL, sha256) and listed in `index.json`. Mobbin images are web apps; the desktop references come from official docs and project repos. Take the pattern only. Never copy layout, branding or copy.

| Name | File | Source | Date viewed | What to take for Berth | What never to copy |
|---|---|---|---|---|---|
| Vercel build logs | mobbin-vercel-build-logs.jpg | https://mobbin.com/screens/50a79ff4-37c0-4a9b-8df3-29599652a9fb | 2026-10-24 | Whole-row tint for warning (amber) and error (red) lines; error and warning counts in the log header; "Find in logs ⌘F" right in the header | Card-in-page layout and the product's sidebar |
| Replit logs with failed row | mobbin-replit-logs-errors.jpg | https://mobbin.com/screens/463065cc-fddc-437c-bc42-fe5c6b8994eb | 2026-10-24 | Dense columns (time, source, message); the failing line gets a solid red row; "Errors only" toggle next to search; agent chat docked beside the logs | Raw JSON shown unformatted; their workspace chrome |
| Railway deploy logs drawer | mobbin-railway-deploy-logs.jpg | https://mobbin.com/screens/8092f7e0-8441-4011-8843-6c2c71a06b65 | 2026-10-24 | Search with up/down match navigation; one log line expanded into attributes; log types as tabs (Build / Deploy / HTTP) | Canvas of service nodes; their dark palette |
| Cloudflare observability logs | mobbin-cloudflare-observability-logs.jpg | https://mobbin.com/screens/db64bfde-eac6-4786-8c29-62b98eb1a149 | 2026-10-24 | Live button plus time-range chips (15m / 1h / 24h) in one toolbar; type chip per line; left colour bar per row | Histogram above the logs (too heavy for a bottom panel) |
| Render services grouped by environment | mobbin-render-services-grouped.jpg | https://mobbin.com/screens/3e106d4a-c875-45d5-be51-387a519355f8 | 2026-10-24 | Group header with count, then a table per group: our project grouping; status as a small labelled badge; ⋯ at the row end | Spacious row height and the big section titles |
| Snowflake warehouses table | mobbin-snowflake-warehouses-table.jpg | https://mobbin.com/screens/257e62fc-b0dd-4ea9-ae24-9a909a7656dd | 2026-10-24 | Filter dropdowns that show their value ("Status All"), result count, Columns menu and refresh in one row; Started / Suspended pills | Their blue accent and icon rail |
| Braintrust table with inspector | mobbin-braintrust-table-inspector.jpg | https://mobbin.com/screens/9af2e557-0b59-4720-b45b-4819f119a812 | 2026-10-24 | Dense numbered rows with a right inspector that has prev/next row arrows and collapsible sections; the selected row stays highlighted | Monospace JSON in every cell |
| Replit command palette | mobbin-replit-command-palette.jpg | https://mobbin.com/screens/9bfad190-392c-4ad0-ba1b-6475cd1bd0d8 | 2026-10-24 | Verbs a dev tool needs (Stop, Restart, Open in editor, Copy SSH command) with a one-line description and shortcut; nested items with a chevron | Very long single list without grouping |
| Vapi command palette | mobbin-vapi-command-palette.jpg | https://mobbin.com/screens/593d7acd-2e16-4365-bcd6-02ce52f48f3b | 2026-10-24 | Sections Actions / Recent / All; breadcrumb hint after each item ("— Build"); footer with key hints and result count | Their dark theme colours |
| Cursor agent with diff | mobbin-cursor-agent-diff.jpg | https://mobbin.com/screens/fa7df34d-7288-4c26-a0e9-95a84921e34e | 2026-10-24 | Agent reply that lists commands run with a check; "N files changed" card with +/- counts that opens a per-file diff column; "New"/"Generated" tags per file | Their web agent layout, PR flow and branding |
| Emergent tool-call rows | mobbin-emergent-tool-call-rows.jpg | https://mobbin.com/screens/e123876d-56ef-4731-aa60-d8e6a4c05ec4 | 2026-10-24 | Each tool call is one grey row: status icon, tool name and truncated arguments in mono, chevron to expand | Credits banners and promotional cards |
| Lindy task steps | mobbin-lindy-tool-steps.jpg | https://mobbin.com/screens/9f4affd5-f387-4149-860e-95c83f9bbba5 | 2026-10-24 | A run of steps as a checklist: done steps checked, the current one "Processing…" with a spinner, each step expandable | Joke loading lines; large type |
| Mintlify chat with split diff | mobbin-mintlify-chat-diff.jpg | https://mobbin.com/screens/75a194a1-ad5f-44e0-b8eb-6f78bd5598ee | 2026-10-24 | Chat ends with a "1 file change +213" card; split diff with "N unmodified lines" folds; red/green line tint plus gutter mark | Their file tree and publish flow |
| ClickUp bulk bar | mobbin-clickup-bulk-bar.jpg | https://mobbin.com/screens/e9639493-e0a6-46c9-93d1-d3189cbdc3c7 | 2026-10-24 | Floating bar at the bottom: "3 selected ×" then labelled actions, delete as an icon in danger colour at the end | Its menu content and colour-coded statuses |
| LangSmith bulk bar | mobbin-langsmith-bulk-bar.jpg | https://mobbin.com/screens/6fbcce8e-09c0-47df-b701-da468e82af09 | 2026-10-24 | "1 run selected" with labelled buttons, disabled ones greyed (not hidden); status icon and latency chip (green/red) in dense rows | Their colours and nav |
| Snowflake service detail | mobbin-snowflake-service-detail.jpg | https://mobbin.com/screens/23f7bfb8-e8e1-4086-9a4b-45f6805f375f | 2026-10-24 | Detail tabs Overview / Logs / Events / Metrics / Spec; container status table with Restarts and Last exit code; key-value grid in three columns | Page-width cards with big padding |
| Render env vars | mobbin-render-env-vars.jpg | https://mobbin.com/screens/fa035917-7e53-4000-a224-7c62da231318 | 2026-10-24 | Key / masked value rows with a delete per row; "Add from .env" next to "Add variable" | Form-page layout |
| Railway raw env editor | mobbin-railway-env-raw-editor.jpg | https://mobbin.com/screens/6d289cb0-e4f3-4961-86c4-eb4bbb2c7d52 | 2026-10-24 | Raw editor with ENV / JSON switch and "Copy ENV" for bulk edits of a service's env | Modal dialog over a canvas |
| OpenAI empty list + detail | mobbin-openai-empty-list-detail.jpg | https://mobbin.com/screens/2154a273-3987-43c0-8c15-2989d2cb9f39 | 2026-10-24 | Empty list with one line of reason and two buttons; the detail pane says "Select an item to view details" instead of disappearing | Deprecation banner, brand mascot icon |
| Docker Desktop containers | web-docker-desktop-containers.webp | https://raw.githubusercontent.com/docker/docs/main/content/get-started/docker-concepts/running-containers/images/running-postgres-containers.webp (docs.docker.com) | 2026-10-24 | Port as a link with external icon; Actions column of stop / ⋯ / delete icons; engine status and RAM/CPU/disk in a footer bar | Usage summary above the table; heavy blue header |
| Docker Desktop container rows | web-docker-desktop-container-rows.webp | https://raw.githubusercontent.com/docker/docs/main/content/get-started/docker-concepts/running-containers/images/multi-container-apps.webp | 2026-10-24 | Name over short ID with copy button; "Only show running" toggle; sticky Actions column | Two-line rows at ~52px (too tall for Berth's 28px) |
| Podman Desktop containers | web-podman-desktop-containers.webp | https://raw.githubusercontent.com/podman-desktop/podman-desktop/main/website/static/img/features/manage-containers.webp | 2026-10-24 | Collapsible group rows (pod with N containers) with group-level actions; state word under the name (RUNNING / EXITED); bulk actions appear beside search with "On 2 selected items"; detail tabs Summary / Logs / Inspect / Kube / Terminal | Purple branding; tall rows |
| Portainer containers list | web-portainer-containers-list.png | https://raw.githubusercontent.com/portainer/portainer-docs/2.45/lts/.gitbook/assets/2.20-containers-list.png (docs.portainer.io) | 2026-10-24 | Toolbar Start / Stop / Kill / Restart / Pause / Resume / Remove shown disabled until rows are selected; state pills incl. "unhealthy" and "healthy"; per-row quick actions (logs, inspect, stats, console) | Web-admin look, ownership column, pagination |
| OrbStack containers | web-orbstack-containers-hero.png | https://orbstack.dev/img/hero.png | 2026-10-24 | Compose project as a selected group row with its own stop/delete; services nested with image under name and a status dot on the icon; detail as a segmented control Info / Stats / Logs / Terminal / Files | The two-pane native layout as a whole, its colours and icons |
| Zed agent panel | web-zed-agent-panel.webp | https://zed.dev/img/home/og.webp | 2026-10-24 | Agent panel beside the editor; tool call shown as a one-line action ("Read file path (lines 105-125)") with an icon; "Restore Checkpoint"; context chips inline in the prompt | Editor chrome, type and brand |
| Dozzle log viewer (GIF) | web-dozzle-demo.jpg (GIF bytes; the fetch script names non-png/webp as .jpg), frames web-dozzle-demo-frame60-logfmt-errors.png, web-dozzle-demo-frame120-stacktrace.png | https://raw.githubusercontent.com/amir20/dozzle/HEAD/.github/demo.gif | 2026-10-24 | Level dot per line (green info, amber warn); error lines get a dark red row tint plus a red left bar; a stack trace stays one grouped block under its error; row ⋯ appears on hover; "Live" indicator and CPU/MEM in the log header; services listed under their project in the sidebar | Its sidebar tree look, mascot and colours |
| Lazydocker (viewed, not saved) | none | https://github.com/jesseduffield/lazydocker (README demo3.gif) | 2026-10-24 | Status as a coloured word with exit code in place: "exited (1)" red, "running" green; key hints in a footer | TUI boxes |

Not found: an OrbStack logs view screenshot (only the hero image is public), a Docker Desktop logs view image in the docs repo (the logs images there are terminal output), and a Portainer log view (the docs show only the log settings form).

### Patterns that recur
- Status is a short word or pill with colour, plus a dot on the icon; the colour never carries the meaning alone (Portainer, Snowflake, Podman, OrbStack, Lazydocker). Exit code sits next to the state ("exited (1)").
- Row actions live at the right end: 2–3 icons (stop/start, delete) plus ⋯. Desktop tools (Docker Desktop, Podman, OrbStack) show them on every row; web tools hide behind ⋯. Delete sits last and in danger colour.
- Bulk actions show up next to the search or as a floating bottom bar with the count ("3 selected ×"); actions that do not apply are disabled, not hidden (Portainer, LangSmith).
- Compose projects (or pods) are collapsible group rows with their own actions; services are indented beneath (OrbStack, Podman, Render's environment groups).
- Row height: desktop container tables run 44–52px; the denser references (Braintrust, Snowflake, Dozzle logs) sit near 28–32px, which supports Berth's 28px.
- Log errors: whole-row tint (red for error, amber for warn) plus a left bar or gutter mark; level dot or word per line; stack traces kept as one block; error/warning counts and an "Errors only" filter in the header (Vercel, Replit, Dozzle, Cloudflare).
- Log toolbar: search with match up/down, Live/Follow toggle, time range chips, all on one row (Railway, Cloudflare, Dozzle).
- Detail panes use tabs in the same order: Overview/Info, Logs, Terminal, Stats, Inspect/Files (Podman, OrbStack, Snowflake, Railway).
- AI tool calls are one-line rows (icon, tool name, mono args truncated, chevron), with a check when done and a spinner on the running one; file changes end the turn as a "N files changed +x −y" card that opens a diff (Emergent, Lindy, Cursor, Mintlify, Zed).
- Command palettes group results (Actions / Recent / Navigation), show the shortcut right-aligned and key hints in a footer (Vapi, Replit).

### Gaps
- No reference for an approval prompt on a tool call (the tool, its arguments, Approve / Deny). The coding-agent search returned finished tool calls only.
- No good multi-container merged log view with a colour prefix per container (Dozzle shows one container at a time; the Docker docs image is terminal output).
- No bottom panel with tabs (Logs / Terminal / Problems / Assistant) in a desktop app; Zed shows the agent as a side panel, not a bottom tab. Use VS Code or Zed docs at the screens stage.
- No Problems list (severity, title, subject, Apply fix) and no "Explain" action on a log line.
- No menu bar extra and no first-run checks screen.
- No Coolify-style deployment list with live build log in a desktop shell; Vercel and Railway cover the log part only.
