# Berth: product brief

Date: 2026-10-24 · Source: idea from Emmanuel (voice brainstorm, this session) · Working name: **Berth** (placeholder; a berth is where a ship docks)

This brief is written for an AI that will build the UI. Section 10 lists the build rules. Section 9 lists the open questions and the default taken for each, so work never waits on an answer.

---

## 1. The idea, shaped

- **Problem (one sentence):** Developers who run Docker through Colima on macOS manage containers through the terminal, because Docker Desktop is heavy and licensed, and no light GUI understands their projects, their remote hosts, or their errors.
- **Who has it, and why now:** Solo developers and small teams on Apple Silicon who already left Docker Desktop for Colima. They now type `docker ps`, `docker compose up`, `docker logs -f` and `docker exec -it` all day. AI coding harnesses (Claude Code, muse) are now installed on most of these machines, so a Docker tool can hand them real context (logs, compose files, the codebase) instead of asking the user to copy and paste.
- **What they use today:** the `docker` and `docker compose` CLIs; Lazydocker (terminal UI); OrbStack (fast, polished, but replaces Colima instead of managing it); Portainer (web, built for remote hosts, slow for local work); Docker Desktop (the thing they left); the Coolify web dashboard for deployed apps.
- **The one idea that makes Berth different:** a container is known by **the folder it came from**, not by its ID. Every list starts from projects (folders), and a folder with no Docker setup gets one generated.
- **What it combines:** Docker Desktop's local workflow (projects, logs, terminal, images, cleanup) with Portainer's management depth (many engines, deploy from Git, templates, registries, edit-and-recreate, resource limits, an activity log), in one native-feeling desktop app. Section 1.1 lists the Portainer features Berth takes and the ones it leaves out.
- **Distribution:** built for Emmanuel first, released as **open source**. No accounts in v1. The data model leaves room for accounts later (section 1.2).
- **MVP slice (Milestone 1 and 2):**
  1. Connect to every Colima profile on the machine; start, stop and resize them.
  2. Projects view: folders grouped from Compose labels, with up, down, restart, rebuild.
  3. Containers, Images, Volumes, Networks: dense tables with search, filter, sort and bulk actions.
  4. Container detail: logs (live, searchable), terminal (exec), stats, inspect, env, mounts, files.
  5. Command palette (⌘K) that covers the 20 Docker commands the user types most.
  6. Global search across every resource on every engine.
  7. Open a folder: detect the Compose file or Dockerfile; if there is none, generate one from **rules** (no AI) and show the diff before writing.
  8. Problems panel: known Docker errors detected by rules, each with a plain-language reason and a fix button.
  9. A **Logs** section that streams one container, a project, or any set of containers merged, with error lines marked (6.13).
  10. A **menu bar** item with engines, projects and failing containers (6.14).
  11. Portainer-depth container management, local engines first: edit and recreate a container, change its CPU and memory limits live, pause, kill, rename, duplicate; create volumes and networks with options; connect and disconnect containers from networks; the activity log.
- **Later milestones:**
  - M3 AI: detect installed harnesses (Claude Code, muse), send them context, receive proposed edits as diffs.
  - M4 Remote, Portainer parity and plugins: remote engines over SSH or TLS with a dashboard per engine; projects deployed from a Git repo (with "pull and redeploy" and optional polling); templates; registries (Docker Hub, GHCR, private) with credentials in the Keychain; image build, tag, push, import and export; the plugin API and the first plugin (Coolify).
  - M5 Visual Compose builder (drag services, draw `depends_on`, edit ports and volumes on a canvas).
- **Cut from the MVP, on purpose:** Kubernetes (Colima can run k3s, but it is a second product); Docker Swarm (services, secrets, configs); Windows. AI is cut from M1 and M2 so the rule-based path works on its own first.
- **Non-goals:** Berth does not ship a container runtime or a VM; Colima does that. It does not replace Coolify; the Coolify plugin reads and triggers, and Coolify stays the source of truth. It is not a team or cloud product in v1: no accounts, no sync, no telemetry by default.

### 1.1 Portainer features: taken, adapted, left out

Taken because a developer uses them weekly; left out because they serve an operations team, not one developer.

| Portainer feature | In Berth | Milestone |
|---|---|---|
| Environments (many Docker endpoints, one switcher) | **Engines**, with Colima profiles first-class and remote hosts beside them; lists can show one engine or all merged | M1 local, M4 remote |
| Environment dashboard (counts, engine info, disk) | **Engine dashboard** | M1 |
| Stacks: deploy from web editor | **Project from editor**: paste or write a Compose file; Berth saves it to a folder you pick, so it stays a folder-first project | M2 |
| Stacks: deploy from Git repo, pull and redeploy, auto-update by polling or webhook | **Project from Git**: clone to a local folder (or deploy remotely to an engine), **Pull and redeploy**, optional polling every N minutes. No webhook server in v1 | M4 |
| App templates and custom templates | **Templates**: one-click services (Postgres, Redis, MySQL, MongoDB, MinIO, Mailpit, RabbitMQ, Meilisearch…) plus your own saved templates; they share the format with Recipes | M2 built-ins, M4 custom |
| Container: logs, console, attach, inspect, stats | Container detail tabs (Logs, Terminal, Attach, Inspect, Stats) | M1 |
| Container: duplicate / edit, recreate, pull latest and recreate | **Edit and recreate** sheet showing a diff of the container's settings before recreating; **Duplicate**; **Recreate with latest image** | M1 |
| Container: runtime and resources (live `docker update`) | **Limits** in the inspector: CPU, memory, restart policy, applied without recreate | M1 |
| Container: pause, resume, kill, rename | Row and palette actions | M1 |
| Container: commit to image | **Save as image** | M4 |
| Container: quick actions in the list | Hover actions on each row: logs, terminal, restart, stop, open port | M1 |
| Images: build (editor, upload, URL), tag, push, import and export tar | **Build image** sheet (from a project folder or a Dockerfile in the editor), **Tag**, **Push**, **Export** and **Import** | M4 (build from folder in M2) |
| Registries with credentials, browse tags | **Registries** screen; credentials in the macOS Keychain; tag browser when pulling | M4 |
| Volumes: create with driver options, browse files | **Create volume** sheet; **Volume browser** with upload and download | M1 create, M2 browse |
| Networks: create with subnet and gateway, connect and disconnect containers | **Create network** sheet; **Connect** / **Disconnect** in the network and container details | M1 |
| Events and activity logs | **Activity**: a live feed of engine events (`docker events`) plus a log of every action taken in Berth, filterable by engine, project and type | M1 |
| Users, teams, roles (RBAC), LDAP/OAuth | Left out of v1; see 1.2 | Later |
| Edge agents, edge groups, fleet updates | Left out: operations-fleet feature | — |
| Kubernetes, Swarm, Nomad, Azure ACI | Left out: second products | — |
| Webhooks to redeploy a service | Left out of v1; Git polling covers the developer case | Later |

### 1.2 Open source, and accounts later

- **Licence:** default MIT OR Apache-2.0, the same dual licence as Rust and Dioxus (question in section 9).
- **Open-source consequences for the UI:** no telemetry, or opt-in only and shown in Settings; every external call (Docker Hub search, registry, Coolify, AI harness) named in Settings; plugin authors get a documented manifest and protocol; distribution through GitHub releases and a Homebrew cask, with in-app "Update available" from GitHub releases.
- **Room for accounts later, without building them now:** every action in the Activity log records an `actor` (today always `Actor::Local`); every engine and setting has an owner field that is today always the local profile; credentials live in the Keychain behind one `SecretStore` trait. When accounts arrive (sync settings and engines across machines, shared remote engines with roles), they extend these fields instead of reshaping the data. No sign-in screen, avatar or "Upgrade" placeholder ships in v1.
- **Success measures (targets, not measurements yet):**
  - Coverage: 18 of the user's top 20 Docker terminal commands can be done in Berth with no terminal. Measured by listing the commands from `~/.zsh_history` and checking each one.
  - Speed: every budget in section 1.3 met on an M1 MacBook Air. Measured with Instruments and the app's own timing log.
  - Time to running: an unfamiliar repo with no Dockerfile goes from "Open folder" to running containers in under 3 minutes, using rules only. Measured on 5 sample repos (Node, Python, Go, Rust, PHP).
  - Error help: for the 15 error kinds in section 6.4, the Problems panel names the cause and offers a fix in at least 12. Measured by reproducing each error.
- **Risks and unknowns:**
  - Dioxus desktop renders through the system webview (WKWebView on macOS), so 500-row tables need virtual scrolling and logs need a capped buffer, or the "fast" promise breaks.
  - Apple Silicon and amd64 images: `exec format error` is the most common Colima failure, and the fix depends on whether the profile has Rosetta enabled.
  - Writing files into the user's repo (generated Dockerfiles, AI edits) must always go through a reviewed diff. A silent write breaks trust for good.
  - Plugin security: a plugin that holds a Coolify API token can deploy to production. Plugins need declared permissions.
  - Remote engines over SSH add latency; every view needs a loading and a stale state.


### 1.3 Performance budget ("very, very performant")

These are targets for the release build on an M1 MacBook Air with 8 GB of memory, the slowest Apple Silicon Mac the app supports. They are not measurements yet. Each one is checked in CI or with Instruments before a release.

| Measure | Budget | How it is met |
|---|---|---|
| Cold start to a populated Projects view | < 600 ms | Window and shell render first, from the last snapshot cached in SQLite; live engine data replaces it as it arrives |
| Click to visible response | < 50 ms | Optimistic state ("Stopping…") at once; the engine call runs off the UI thread |
| Containers table, 1,000 rows | 120 fps scroll on ProMotion | Virtual list (only visible rows in the DOM); rows are plain elements, no per-row components with their own effects |
| Log streaming | 10k lines/s sustained without dropped frames | Rust side batches lines and sends one update per animation frame; ring buffer per view; search runs in Rust, not in the webview |
| Live stats | Only visible rows subscribe | One `stats` stream per visible container, closed on scroll-out and when the window is hidden |
| Idle memory (window open, 50 containers) | < 150 MB | One webview; no polling when nothing changes; images and icons as an SVG sprite |
| Idle CPU (window closed, menu bar only) | < 0.5 % | Event-driven: subscribes to `docker events` instead of polling; stats streams closed |
| Engine updates | Event-driven | `docker events` drives list updates; full refresh only on reconnect |
| App size | < 30 MB download | Release profile with LTO and `strip`; no bundled browser (system WKWebView) |

Rules for the build: the UI never calls the engine directly, only through an async core that owns all I/O; no work on the main thread that can take over 4 ms; every list over 100 rows is virtual; the webview never holds more than one screen of log lines in the DOM.

---

## 2. Archetype

- **Primary:** dense-pro.
- **Borrowed slots:** from fintech-trust, the "show the full effect before commitment" rule: every destructive or file-writing action (prune, remove, generate, apply AI fix, Coolify redeploy) shows exactly what will change, and the same numbers appear on the confirm and on the result.
- **Why:** The user is a developer who scans and acts on records all day on a desktop. Trust breaks when a container's state is stale, a log line is missing, or the app is slower than typing the command. That is the dense-pro trust job.
- **What this means for the UI:** 13px UI body; monospace for IDs, ports, image tags, logs and paths; tabular figures for CPU, memory, size and uptime; neutral greys with one accent for action and selection; status colours (running, stopped, restarting, unhealthy, exited with error) that carry meaning on their own; radii of 4 to 6; motion under 150 ms; keyboard first.
- **Bands key for the scorer:** `dense-pro`.

---

## 3. Platforms and target

- **Platforms:** macOS (Apple Silicon) first, because Colima on macOS is the user's setup. Linux second; Colima runs there too and Dioxus desktop supports it.
- **Stack:** Rust. UI in **Dioxus 0.7, desktop renderer** (webview). Docker Engine API through the `bollard` crate against each engine's socket. Colima controlled through its CLI. Full detail in `.eng/stack.md`.
- **Window sizes:** primary 1440x900; must work at 1024x700 (inspector collapses); minimum 900x600.
- **Target canvas:** this repo. Screens are designed in FigCraft next (`design/figcraft/berth.figcraft`), then built.

---

## 4. Roles and jobs

One role for now: **the developer** (the person whose Mac this is). A "viewer" role arrives only if remote engines are shared, which is not planned.

| Role | Jobs (verb + object) |
|---|---|
| Developer | Start and stop a Colima profile; change its CPU, memory and disk |
| | See every project (folder) and whether it is up |
| | Bring a project up or down, restart one service, rebuild one image |
| | Open a folder and get it running, with a generated Compose file if it has none |
| | Follow, search and filter the logs of one container or a whole project |
| | Open a shell in a container |
| | Find any container, image, volume or network by name, tag, port, label or folder |
| | Free disk space: see what is unused and remove it, with the size shown first |
| | Pull an image; search Docker Hub |
| | Browse a volume's files; back up or restore a volume |
| | Understand why a container failed and apply a fix |
| | Ask an AI harness to write or repair a Compose file, with the change shown as a diff |
| | Edit a container's settings and recreate it; change its CPU and memory limits live |
| | Add a ready-made service (Postgres, Redis…) to a project from a template |
| | Deploy a project from a Git repo and redeploy it after a push |
| | Log in to a registry; build, tag and push an image |
| | Create a network and connect containers to it |
| | See what happened and when: engine events and every action taken in Berth |
| | Connect a remote engine and manage it like a local one |
| | See Coolify servers, apps and deployments; redeploy and read deploy logs (plugin) |

---

## 5. Entities and API shapes

These are the types the UI shows. Mock data for the design and for UI tests uses them, so swapping mocks for the real engine is a data-layer change, not a UI rewrite. All IDs are newtypes, never bare strings.

```rust
// ---------- Engines ----------
pub struct EngineId(String);            // stable, e.g. "colima:default", "ssh:prod-1"

pub enum EngineKind {
    Colima { profile: String },          // ~/.colima/<profile>/docker.sock
    Remote { transport: RemoteTransport },
    Socket { path: PathBuf },            // any other local socket
}
pub enum RemoteTransport { Ssh { host: String, user: String }, Tcp { url: Url, tls: bool } }

pub enum EngineState { Running, Stopped, Starting, Stopping, Unreachable { reason: String } }

pub struct Engine {
    id: EngineId,
    name: String,                        // "default", "work", "prod-1"
    kind: EngineKind,
    state: EngineState,
    docker_version: Option<String>,
    resources: Option<VmResources>,      // Colima only
    counts: ResourceCounts,              // containers running/total, images, volumes
}
pub struct VmResources {
    cpus: u8, memory: ByteSize, disk: ByteSize,
    arch: Arch,                          // Aarch64 | X86_64
    vm_type: VmType,                     // Vz | Qemu
    rosetta: bool,
    runtime: Runtime,                    // Docker | Containerd | Incus
}

// ---------- Projects (the folder-first idea) ----------
pub struct ProjectId(String);            // hash of canonical folder path + engine

pub struct Project {
    id: ProjectId,
    name: String,                        // compose project name, else folder name
    folder: PathBuf,                     // from label com.docker.compose.project.working_dir
    compose_files: Vec<PathBuf>,         // from label com.docker.compose.project.config_files
    engine: EngineId,
    source: ProjectSource,
    services: Vec<ServiceSummary>,
    git: Option<GitInfo>,                // branch, dirty flag
    status: ProjectStatus,               // derived from services
}
pub enum ProjectSource {
    Compose,                             // found through compose labels
    Pinned,                              // user added the folder; may have no containers yet
    Git { repo: Url, branch: String, last_commit: Option<String>, poll: Option<Duration> }, // cloned and deployed
    RemoteOnly,                          // compose project on a remote engine whose folder is not on this Mac
    Inferred { from_bind_mount: PathBuf }, // plain `docker run` with a bind mount into a known folder
    Loose,                               // no folder known; shown under "Unassigned"
}
pub enum ProjectStatus { AllRunning, Partial { running: u16, total: u16 }, Stopped, Failing, NotCreated }

pub struct ServiceSummary {
    name: String, image: String, replicas: u16,
    state: ContainerState, ports: Vec<PortMapping>, health: Option<Health>,
}

// ---------- Containers ----------
pub struct ContainerId(String);          // full ID; UI shows the first 12 chars in mono

pub struct Container {
    id: ContainerId,
    name: String,
    project: Option<ProjectId>,
    service: Option<String>,
    image: ImageRef,
    state: ContainerState,
    health: Option<Health>,              // Starting | Healthy | Unhealthy
    exit_code: Option<i32>,
    created: DateTime<Utc>,
    started: Option<DateTime<Utc>>,
    ports: Vec<PortMapping>,
    mounts: Vec<Mount>,
    networks: Vec<String>,
    labels: BTreeMap<String, String>,
    stats: Option<LiveStats>,            // only while visible
    platform: Option<Arch>,              // to flag amd64 under emulation
}
pub enum ContainerState { Created, Running, Paused, Restarting, Removing, Exited, Dead }
pub struct PortMapping { host_ip: Option<IpAddr>, host_port: Option<u16>, container_port: u16, proto: Proto }
pub struct LiveStats { cpu_pct: f32, mem_used: ByteSize, mem_limit: ByteSize, net_rx: ByteSize, net_tx: ByteSize }

// ---------- Images, volumes, networks ----------
pub struct Image {
    id: String, repo_tags: Vec<String>, size: ByteSize, created: DateTime<Utc>,
    arch: Arch, used_by: Vec<ContainerId>, dangling: bool,
}
pub struct Volume {
    name: String, driver: String, size: Option<ByteSize>, // size needs `system df`, may be slow
    mountpoint: String, used_by: Vec<ContainerId>, project: Option<ProjectId>, created: DateTime<Utc>,
}
pub struct Network {
    id: String, name: String, driver: String, subnet: Option<String>,
    containers: Vec<ContainerId>, project: Option<ProjectId>, built_in: bool,
}

// ---------- Container editing (Portainer "duplicate / edit") ----------
pub struct ContainerSpec {               // what the Edit and recreate sheet edits
    name: String, image: ImageRef, command: Option<Vec<String>>, entrypoint: Option<Vec<String>>,
    env: Vec<(String, String)>, ports: Vec<PortMapping>, mounts: Vec<Mount>, networks: Vec<String>,
    labels: BTreeMap<String, String>, restart: RestartPolicy, limits: Limits,
}
pub struct Limits { cpus: Option<f32>, memory: Option<ByteSize>, memory_swap: Option<ByteSize> } // live via `docker update`
pub enum RestartPolicy { No, Always, UnlessStopped, OnFailure { max_retries: u16 } }

// ---------- Templates, registries, activity ----------
pub struct Template {
    id: String, name: String,            // "Postgres 17"
    kind: TemplateKind,                  // BuiltIn | Custom
    service: String,                     // compose service YAML fragment
    inputs: Vec<TemplateInput>,          // port, password (generated), volume name, version
    category: String,                    // Database | Cache | Queue | Mail | Storage | Search
}
pub struct Registry {
    id: String, name: String, url: Url,  // docker.io, ghcr.io, private
    username: Option<String>,            // password or token lives in the Keychain, never in this struct
    status: RegistryStatus,              // LoggedIn | LoggedOut | Error{reason}
}
pub struct ActivityEntry {
    ts: DateTime<Utc>, engine: EngineId,
    actor: Actor,                        // Local today; room for accounts later
    origin: ActivityOrigin,              // Engine (docker events) | Berth (an action taken in the app) | Plugin(String)
    action: String,                      // "container.restart", "image.pull", "project.redeploy"
    subject: Subject, outcome: Outcome,  // Ok | Failed{reason}
}
pub enum Actor { Local }                 // later: User(UserId)

// ---------- Logs and terminal ----------
pub struct LogLine { container: ContainerId, ts: DateTime<Utc>, stream: Stream, text: String } // Stream: Stdout | Stderr
pub struct ExecSession { id: String, container: ContainerId, shell: String, cols: u16, rows: u16 }

// ---------- Problems and fixes ----------
pub struct Problem {
    id: ProblemId,
    kind: ProblemKind,                   // see 6.4
    severity: Severity,                  // Error | Warning | Info
    subject: Subject,                    // Container | Project | Engine | Image
    title: String,                       // "Port 5432 is already in use"
    explanation: String,                 // one or two plain sentences
    evidence: Vec<String>,               // the log lines or inspect fields it was detected from
    fixes: Vec<Fix>,
}
pub struct Fix {
    label: String,                       // verb + object: "Map Postgres to port 5433"
    source: FixSource,                   // Rule | Harness(HarnessId)
    effect: FixEffect,
}
pub enum FixEffect {
    EditFiles(Vec<FileDiff>),            // always shown as a diff before applying
    RunAction(EngineAction),             // e.g. remove container, restart engine with Rosetta
    Explain,                             // no automatic fix; text only
}

// ---------- Generation ----------
pub struct Detection {                   // result of scanning a folder
    folder: PathBuf,
    found: Vec<FoundFile>,               // compose.yaml, Dockerfile, .env, .dockerignore
    stack: Vec<StackGuess>,              // Node{pm, framework}, Python{tool}, Go, Rust, Php, Ruby, Java, Static
    services_needed: Vec<ServiceGuess>,  // Postgres, Redis, MySQL, Mongo... from deps and .env keys
    confidence: Confidence,
}
pub struct GenerationPlan { files: Vec<FileDiff>, source: FixSource, notes: Vec<String> }

// ---------- AI harnesses ----------
pub struct Harness {
    id: HarnessId,                       // "claude-code", "muse"
    name: String,
    binary: PathBuf,                     // found on PATH, ~/.local/bin, /opt/homebrew/bin
    version: Option<String>,             // from `--version`
    transport: HarnessTransport,
    available: bool,
}
pub enum HarnessTransport {
    // Multi-turn, streamed both ways: claude -p --input-format stream-json --output-format stream-json
    //   --include-partial-messages --mcp-config <berth-mcp.json> --strict-mcp-config
    ClaudeStreamJson,
    // Multi-turn session host over stdio: `muse serve` (MSP; schema from `muse schema`).
    // One-shot fallback: `muse exec --json --workspace <folder>`
    MuseMsp,
}
pub struct AssistantSession {
    id: String, harness: HarnessId, project: Option<ProjectId>,
    context: Vec<ContextItem>,           // what the user attached or Berth attached for them
    turns: Vec<Turn>,                    // user text, assistant text, tool calls, proposed changes
    state: SessionState,                 // Idle | Thinking | WaitingForApproval(ToolCall) | Error{stderr}
}
pub enum ContextItem {
    Logs { container: ContainerId, lines: Range<usize> },  // a selection, or "last 200 lines"
    Problem(ProblemId), ComposeFile(PathBuf), Dockerfile(PathBuf), Inspect(ContainerId), FolderTree(PathBuf),
}
pub struct ToolCall {                    // a Berth MCP tool the harness called
    tool: String,                        // "docker.logs", "compose.up", "files.propose_edit"
    args: serde_json::Value,
    risk: Risk,                          // Read (runs at once) | Change (needs approval) | Destructive (approval + typed name)
}

// ---------- Plugins ----------
pub struct Plugin {
    id: String, name: String, version: String,
    permissions: Vec<Permission>,        // Network{hosts}, ReadEngines, RunEngineActions, Secrets{keys}
    contributes: Vec<Contribution>,      // SidebarSection, ResourceView, Action, ProblemSource, PaletteCommands
    state: PluginState,                  // Enabled | Disabled | Error{reason}
}
// Coolify plugin entities. Source: Coolify OpenAPI spec at tag v4.5.0 (released 2026-10-23),
// base URL <instance>/api/v1, bearer token from Coolify "Keys & Tokens → API tokens". Field names as in the spec.
pub struct CoolifyServer { uuid: String, name: String, ip: String, user: String, port: u16, proxy_type: Option<String>, unreachable_count: u32 }
pub struct CoolifyProject { uuid: String, name: String, description: Option<String>, environments: Vec<CoolifyEnvironment> }
pub struct CoolifyEnvironment { id: u64, name: String }            // "production", "staging"
pub enum CoolifyResource {                                           // GET /resources
    Application(CoolifyApp), Service(CoolifyService), Database(CoolifyDatabase),
}
pub struct CoolifyApp {
    uuid: String, name: String, fqdn: Option<String>, status: String,   // e.g. "running:healthy", "exited"
    git_repository: Option<String>, git_branch: Option<String>, git_commit_sha: Option<String>,
    build_pack: String,                                                 // nixpacks | dockerfile | dockercompose | static
    limits_cpus: Option<String>, limits_memory: Option<String>,
}
pub struct CoolifyService { uuid: String, name: String, service_type: Option<String>, server_id: u64 }
pub struct CoolifyDatabase { uuid: String, name: String, kind: String, status: String } // postgresql, redis, mysql…
pub struct CoolifyDeployment {                                       // ApplicationDeploymentQueue
    deployment_uuid: String, application_name: String, server_name: String,
    status: String,                                                  // queued | in_progress | finished | failed | cancelled
    commit: Option<String>, commit_message: Option<String>,
    is_webhook: bool, is_api: bool, rollback: bool,
    created_at: DateTime<Utc>, updated_at: DateTime<Utc>, logs: Option<String>,
}
```

---

## 6. Flows

### 6.1 First run
1. Berth checks for `colima`, `docker`, and each `~/.colima/<profile>/docker.sock`.
2. If Colima is missing: a one-screen setup with the `brew install colima docker docker-compose` command and a Copy button. Berth never runs installers itself.
3. If a profile is stopped: "Start default (2 CPU, 8 GB)" with an edit link.
4. Berth scans running containers for Compose labels and builds the Projects list.
5. Berth detects AI harnesses on `PATH` and in `~/.local/bin` and shows them in Settings as found or not found. No prompt; it stays quiet until the user needs it.

### 6.2 Open a folder (the core flow)
1. User drags a folder onto the window, presses ⌘O, or runs "Open folder" from the palette.
2. Berth scans it (Detection): Compose file, Dockerfile, `.env`, stack, needed services.
3. **Compose found:** the project appears with its services as "not created"; primary action **Start project** (`compose up -d`).
4. **Dockerfile only:** Berth offers **Create compose.yaml** with one service built from the Dockerfile, plus any services it detected (Postgres from `DATABASE_URL`, Redis from a Redis client in the dependencies).
5. **Nothing found:** Berth shows what it detected ("Node 22 with pnpm, Next.js; uses Postgres") and offers:
   - **Generate from recipe** (rules, instant, offline);
   - **Generate with Claude Code** or **with muse** (only if detected; M3).
6. Either path ends on the same **Review changes** screen: a file tree of new and changed files, a diff per file, and a summary ("Adds 3 files. Writes nothing outside this folder."). Primary action **Write files and start**.
7. The project starts; the bottom panel opens on combined logs; problems, if any, appear in the Problems panel.

### 6.3 Something failed
1. A container exits non-zero, goes unhealthy, or a `compose up` fails.
2. The row turns to the error status; a badge appears on the Problems tab with the count.
3. The Problems panel shows the title, the plain explanation, and the log lines it came from.
4. Fixes are listed as buttons. A file fix opens Review changes; an action fix ("Restart Colima with Rosetta") shows exactly what will happen and how long it takes.
5. If no rule matches, or the user wants a second opinion: **Fix with Claude Code** (or muse, whichever was detected) opens the Assistant with the problem, the log lines it came from, the inspect output, the Compose file and the folder tree already attached. The harness can read more logs itself through Berth's tools (6.12), proposes a change, and the change lands on the same Review changes screen. After writing, Berth restarts the service and watches the logs; if the same error comes back, the Assistant says so and keeps going.

### 6.4 Problem kinds detected by rules (M2)
Port already allocated · Image not found or pull access denied · `exec format error` (amd64 image on arm64) · Out of memory, exit 137 · Container exits immediately, exit 1 with no logs · Unhealthy health check · Bind mount path does not exist · Permission denied on a bind mount · No space left on the Colima disk · Engine socket missing (Colima stopped) · Network not found · Name already in use · Env var referenced in Compose but missing from `.env` · `depends_on` service failed · Build failed (with the failing step highlighted).

### 6.5 Free disk space
1. Engine screen shows a disk bar: images, containers, volumes, build cache (from `system df`).
2. **Clean up** opens a sheet listing what would be removed, grouped, with sizes and a total ("Frees 14.2 GB").
3. Volumes are unchecked by default, because they hold data.
4. The confirm button repeats the total: **Remove 41 items (14.2 GB)**. The result toast shows the same figure.

### 6.6 Exec into a container
Select a container and press ⌘T, or use **Terminal** in the detail. Berth tries `bash`, then `sh`. The terminal opens in the bottom panel as a tab named after the container, so several shells can stay open.

### 6.7 Edit and recreate a container
1. Select a container → **Edit** (or ⌘E).
2. A sheet shows its current settings as a form: image, command, env, ports, mounts, networks, restart policy, limits.
3. Changes to limits and restart policy apply live with **Apply limits**; no recreate needed.
4. Any other change makes the primary button **Recreate container**. Before it runs, the sheet shows a settings diff and warns what is lost ("Files written inside the container, outside volumes, are lost").
5. For a Compose service, Berth offers to write the same change into `compose.yaml` instead (through Review changes), so the folder stays the source of truth.

### 6.8 Add a service from a template
Project → **Add service** → pick a template (Postgres 17) → fill inputs (port 5432 is taken, so Berth suggests 5433; password generated) → Review changes shows the new `compose.yaml` block and `.env` lines → **Write files and start service**.

### 6.9 Deploy from Git (M4)
Projects → **New project** → **From Git** → repo URL and branch (SSH keys from `~/.ssh`) → target engine → local folder to clone into → Berth detects or generates the Compose file (same flow as 6.2) → **Deploy**. Afterwards the project row shows the commit, and **Pull and redeploy** fetches, shows the incoming commits, and redeploys. Optional polling checks every N minutes and shows "3 new commits" instead of redeploying on its own, unless auto-redeploy is switched on.

### 6.10 Add a remote engine (M4)
Engines → **Add engine** → SSH (pick a host from `~/.ssh/config`) or TCP+TLS → test the connection → name it → it appears in the engine switcher with a coloured marker, and every list can show it alone or merged with the others.

### 6.11 Coolify plugin (M4)
Built against the Coolify OpenAPI spec at tag **v4.5.0** (released 2026-10-23; 212 paths, identical on `main` that day). The plugin pins the spec version it was generated from and reads `GET /version` on connect; a newer major version shows "Built for Coolify 4.5; some actions may fail" instead of failing silently.

1. Plugins → Coolify → instance URL (`https://app.coolify.io` or self-hosted) and API token (created in Coolify under Keys & Tokens → API tokens; stored in the macOS Keychain).
2. **Test connection** calls `GET /health` then `GET /version`, and shows the instance version.
3. A Coolify section appears in the sidebar with: **Resources** (`GET /resources`, grouped by project and environment), **Servers** (`GET /servers`), **Deployments** (`GET /deployments`, the running and queued ones).
4. Application detail (`GET /applications/{uuid}`): status, domain, repo, branch, commit, build pack, CPU and memory limits; tabs for **Deployments** (`GET /deployments/applications/{uuid}`), **Logs** (`GET /applications/{uuid}/logs?lines=N`, polled, shown in the same log viewer as local containers), **Env** (`GET /applications/{uuid}/envs`; values hidden until clicked).
5. Actions in v1 (all `POST`):
   - **Deploy** (`/deploy?uuid=…`), with a **Force rebuild** option (`force=true`);
   - **Restart**, **Stop** and **Start** (`/applications/{uuid}/restart|stop|start`; the same for services and databases);
   - **Cancel deployment** (`/deployments/{uuid}/cancel`).
   Each confirms with the app name, server and branch.
6. A deployment row opens its build log (`GET /deployments/{uuid}`, `logs` field), polled until the status is final.
7. Coolify applications whose Git repo matches a local project's Git remote are linked: the local project row shows "Deployed on Coolify: running" and offers **Deploy to Coolify** after a push.
8. Coolify can expose its own MCP server (`POST /mcp/enable`). When it is on, Berth adds it to the Assistant's tool list, so the AI can read deploy logs and status on the server too.

Later (not v1): env var editing, rollback (`/applications/{uuid}/rollback`), preview deployments, database backups, server Docker cleanup.

### 6.12 AI assistant: interactive, with Docker tools
The assistant is the installed harness (Claude Code or muse) running a session that Berth drives, with Berth's Docker abilities given to it as tools. It is a conversation, not a one-shot generator.

1. **Detection, automatic:** on launch and when Settings opens, Berth looks for `claude` and `muse` on `PATH`, in `~/.local/bin` and in `/opt/homebrew/bin`, and runs `--version`. Found harnesses appear in the Assistant's picker; the last one used is the default. If neither is found, the Assistant tab shows how to install one, and every rule-based feature still works.
2. **Berth's tools, given to the harness over MCP:** Berth runs a local MCP server (`berth mcp`, stdio) and passes it to the harness (`--mcp-config` for Claude Code; the session config for muse). Tools:
   - **Read, run at once:** `engines.list`, `projects.list`, `containers.list`, `containers.inspect`, `containers.logs` (with `since`, `tail`, `grep`), `containers.stats`, `images.list`, `volumes.list`, `networks.list`, `compose.config` (the resolved Compose file), `problems.list`, `files.read` (inside the project folder only), `activity.list`.
   - **Change, needs one click:** `files.propose_edit` (creates a pending change that opens Review changes; the harness never writes files itself), `compose.up`, `compose.restart`, `containers.restart`, `containers.start`, `containers.stop`, `images.pull`, `images.build`.
   - **Destructive, needs the item name typed:** `containers.remove`, `volumes.remove`, `images.remove`, `system.prune`.
3. **What the user can ask, for example:** "Dockerise this folder", "Add Postgres and Redis to this project", "Why does `api` keep restarting?", "Make the build faster", "Write a production Dockerfile with a multi-stage build", "Turn this `docker run` command into a Compose service", "What is using 14 GB?".
4. **Logs into the conversation:** in any log view, select lines → **Ask about these lines**; or on a container → **Explain this error**. Berth attaches the selection as a context chip; the harness can then pull more lines itself with `containers.logs`.
5. **Watch mode:** **Watch and fix** on a project tells the assistant to follow the logs after each change; when an error line appears, the assistant proposes the next fix. Every change still goes through Review changes, so watch mode never edits on its own.
6. **Streaming:** text, tool calls and proposed changes stream into the Assistant tab as they happen. Each tool call is a compact row ("Read 200 lines of api-1 logs"), expandable to its arguments and result.
7. **Stop and resume:** ⌘. stops the turn; sessions are saved per project and resume where they left off.
8. **Open in terminal:** opens the same harness interactively in a terminal tab inside the project folder, with Berth's MCP server attached, for people who prefer the full TUI.

### 6.13 Logs: stream, read, act
1. **Logs** is a top-level section, not only a tab: pick one container, a whole project, or any set of containers across engines; lines stream merged, each prefixed with a coloured container name.
2. Toolbar: **Follow**, search (plain or regex, matches highlighted, N of M), filter by stream (stdout or stderr), level chips (Error, Warn, Info, Debug; detected from common formats and JSON `level` fields), time range (live, last 15 min, last hour, since start), wrap, timestamps, **Copy**, **Save**.
3. **Error lines are marked** in the gutter as they arrive (stderr, `error`/`panic`/`exception`/`fatal`, non-zero exit). A count shows in the toolbar; **Next error** jumps (F8).
4. JSON log lines render as a compact one-line summary that expands to a tree.
5. Stack traces fold into one expandable entry.
6. Every marked error has **Explain** (rules first, then the assistant) and **Ask about these lines**.
7. Remote engines and Coolify apps use the same viewer.

### 6.15 Drag a folder into Berth
1. Dragging a folder from Finder anywhere over the window shows a full-window drop overlay: "Drop to open **new-app**" with the folder path, and what Berth will do: "Berth reads the folder and shows what it finds. Nothing is written yet."
2. Dropping more than one folder queues them: "Open 3 folders" and each gets its own detection sheet in turn.
3. Dropping a file instead of a folder: a `compose.yaml` or `Dockerfile` opens its folder; any other file shows "Drop a folder, a compose file or a Dockerfile".
4. Dropping a folder Berth already knows jumps to that project instead of adding it twice.
5. After the drop, the **Detected** sheet shows what was found (Compose file, Dockerfile, `.env`, stack such as "Node 22 · pnpm · Next.js", services needed such as "Postgres (from DATABASE_URL)") and the next step: **Start project** when a Compose file exists, otherwise **Generate from recipe** or **Generate with Claude Code ▾**, which lead to Review changes (6.2).
6. Dropping onto a project row in the sidebar or the menu bar icon does the same.

### 6.16 Terminal for every container
1. Every container row has a **Terminal** button (⌘T), and the container inspector has a **Terminal** tab.
2. The terminal opens in the bottom panel as a tab named after the container and shell ("api-1 · bash"). Many can stay open; tabs show a dot when output arrives in the background and a grey "Exited" when the session ends.
3. Terminal toolbar: shell (bash, sh, zsh, ash: Berth tries bash, then sh), user (default or root), working directory, **Reconnect**, **Clear**, **Copy**, **Split** (two sessions side by side), **Pop out** (its own window), and **Open in Terminal app** (runs `docker exec -it <container> <shell>` in the user's default terminal: Terminal, iTerm2, Ghostty or WezTerm).
4. A project has **Open terminals for all services**, which opens one tab per running service.
5. Stopped container: the Terminal button reads **Start and open terminal**. Image with no shell: "This image has no shell (distroless or scratch). Browse its files instead" with **Files**.
6. Remote engines work the same through the engine's connection.

### 6.17 Find images online
1. Images has two tabs: **On this engine** (local images) and **Find images** (registries).
2. Find images searches Docker Hub by default (the engine's `/images/search` endpoint, plus the Hub API for tags), and any registry added under Registries (GHCR, private). Typing `ghcr.io/` scopes the search to that registry.
3. Each result shows the name, publisher badge (Docker Official Image, Verified Publisher, Sponsored OSS, Community), stars, pulls, last updated, short description, and whether it has a **linux/arm64** build. arm64 matters on Apple Silicon: images without it show "amd64 only · runs under emulation" in amber.
4. With no query, the tab shows Docker Official Images for the stacks Berth detected in your projects (postgres, redis, node, python) and images you pulled recently. No folder scanning: this comes from projects Berth already knows.
5. Selecting an image opens its detail on the right: description, tags (name, size per architecture, pushed date, digest), the copyable pull command, and actions **Pull postgres:17-alpine** (platform defaults to linux/arm64), **Run…** (the run sheet with ports and env prefilled) and **Add to project…** (adds a Compose service through Review changes).
6. Pulling shows per-layer progress in the row and in the image detail; the image appears under On this engine when done.
7. States: searching; no results ("No images named 'postgress'. Did you mean postgres?"); rate limited ("Docker Hub limits anonymous searches. Log in under Registries to search more."); offline; registry login required (private registry); pull failed with the reason.

### 6.18 Browse the file system to open a project
Besides drag and drop, **Open folder** (⌘O, the Projects header button, the empty state, the palette) opens Berth's own folder browser as a sheet:
1. **Left:** places (Home, Dev and any folders you add as favourites, Desktop, Documents, mounted volumes) and **Recent** (folders opened in Berth).
2. **Middle:** a Finder-style column view of the current folder, with an editable path bar (type or paste `~/Dev/side/new-app`, Tab completes) and a filter field for the current column. Hidden files toggle (⌘⇧.).
3. **Markers:** folders in the column you are looking at show a small marker when they contain `compose.yaml`/`docker-compose.yml` (Compose) or a `Dockerfile`, and "Open" when Berth already knows them. Berth reads only the folder on screen when you open it; it never scans folders in the background (decision in section 9).
4. **Right:** a preview of the highlighted folder: the same detection summary as the Detected sheet (files found, stack, services needed), its git branch, and size of the listing.
5. **Footer:** "Use system dialog…" (opens the standard macOS panel for people who prefer it), Cancel, and the primary **Open new-app** (or **Go to shop-api** for a known project).
6. **States:** loading a large folder; empty folder; no access (macOS privacy: "Berth can't read Documents. Allow it in System Settings → Privacy & Security → Files and Folders." with **Open System Settings**); path not found as you type; a network or external volume that is slow.
7. Inside a project, the project detail has a **Files** tab that browses the project folder (compose files, `.env` keys with values hidden, Dockerfile) and opens a file in your editor. Containers and volumes keep their own **Files** browsers (container detail, Volume browser).

### 6.14 Menu bar
A menu bar icon shows the overall state (all good, something failing, engine stopped). Its menu lists:
- engines with Start and Stop;
- pinned and running projects with Start, Stop and Open;
- failing containers with **Show logs**;
- **Open Berth** and **Quit**.

Closing the main window keeps the menu bar item running; the dock icon is an option in Settings.

---

## 7. Screen inventory

### 7.1 The shell (every screen lives in it)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ ● ● ●  [Engine: colima/default ▾]   [ ⌘K  Search or run a command…     ]  ⚠ 2 │ title bar, 38px
├────────────┬─────────────────────────────────────────────┬───────────────────┤
│ PROJECTS   │  List / table (the current section)         │ Inspector         │
│  api       │  toolbar: filters · group by · bulk actions  │ (selected item)   │
│  web       │  ─────────────────────────────────────────  │ tabs: Overview,   │
│  scratch   │  rows, 28px each, virtualised                │ Logs, Terminal,   │
│ ─────────  │                                              │ Stats, Inspect,   │
│ Containers │                                              │ Env, Mounts,      │
│ Logs       │                                              │                   │
│ Images     │                                              │ Files, Attach,    │
│ Volumes    │                                              │ Limits            │
│ Networks   │                                              │                   │
│ Templates  │                                              │                   │
│ ─────────  │                                              │                   │
│ Engines    │                                              │                   │
│ Registries │                                              │                   │
│ Activity   │                                              │                   │
│ COOLIFY ▸  │ (plugin section)                             │                   │
├────────────┴─────────────────────────────────────────────┴───────────────────┤
│ Bottom panel: Logs │ Terminal: api-db-1 │ Problems (2) │ Assistant   [—][□]  │ resizable, ⌃`
└──────────────────────────────────────────────────────────────────────────────┘
  Sidebar 220px (collapsible) · Inspector 360px (collapses below 1200px wide) · Bottom panel 30% height
```

Keyboard: ⌘K palette · ⌘1–⌘9 sections · ⌘⇧L Logs section · ⌘F filter current list · ⌘O open folder · ⌘N new project · ⌘T terminal · ⌘L logs · ⌘E edit selected · ⌘R restart selected · ⌘. stop selected · ⌫ remove (confirm) · Space toggles the inspector · J/K move selection · ⌃` bottom panel.

### 7.2 Action buttons: every common action is a visible button

Emmanuel's requirement: the actions people type in the terminal (start, stop, restart, delete…) are **real buttons on screen**, not only menu items or shortcuts. The palette and shortcuts are extra ways to do the same thing.

**Where the buttons are**

| Place | Buttons shown | Notes |
|---|---|---|
| **Container row** (Containers, Project detail, Logs picker) | Icon buttons at the right end of every row, visible on hover and always visible on the selected row: **Start** or **Stop** (one toggles to the other), **Restart**, **Logs**, **Terminal**, **Delete**, and **⋯** for the rest | Icons have tooltips with the shortcut ("Restart · ⌘R"); 24px buttons inside the 28px row |
| **Container detail header** (inspector and full page) | Labelled buttons, always visible: **Start / Stop**, **Restart**, **Pause / Resume**, **Kill**, **Delete**; **⋯** holds Edit, Duplicate, Recreate with latest image, Rename, Save as image | Labels, not just icons, because this is where the decision is made |
| **Project row** (Projects) | **Start project / Stop project**, **Restart project**, **Logs**, **⋯** (Rebuild, Pull images, Open folder, Open in editor, **Delete project**) | "Delete project" = `compose down`; the sheet asks whether to also remove volumes and images |
| **Project detail header** | **Start project / Stop project**, **Restart project**, **Rebuild**, **Pull images**, **Delete project** | Per-service rows inside use the container-row buttons |
| **Bulk bar** (appears when 2+ rows are selected in any table) | **Start N**, **Stop N**, **Restart N**, **Pause N**, **Delete N**, with the count in the label | Buttons that do not apply to every selected item show the count they do apply to ("Stop 3", when 2 of 5 are already stopped) |
| **Images** row and detail | **Run**, **Pull latest**, **Tag**, **Push**, **Delete** | Delete is disabled while containers use the image, with the reason in the tooltip and a **Delete containers too** option in the sheet |
| **Volumes** row and detail | **Browse**, **Back up**, **Delete** | Delete of a volume in use is disabled, with the reason |
| **Networks** row and detail | **Connect container**, **Disconnect**, **Delete** | Built-in networks (`bridge`, `host`, `none`) have no Delete button |
| **Engines** row and dashboard | **Start engine / Stop engine**, **Restart engine**, **Edit resources**, **Clean up** | |
| **Menu bar** | **Start / Stop** for engines and projects | |
| **Coolify** app row and detail | **Deploy**, **Restart**, **Stop / Start**, **Cancel deployment** (while one runs) | |

**How the buttons behave**

- **Instant feedback:** a pressed button shows its in-progress state at once ("Stopping…", spinner in place of the icon, label kept) and the row's status changes to the transitional state. The button is locked until the engine answers.
- **Result:** success updates the row in place, with no toast for routine actions (start, stop, restart). Failure keeps the row as it was, shows the engine's message inline under the row, and adds a Problem when Berth can explain it.
- **Disabled, never hidden, when the action is impossible:** for example, Start on a running container becomes Stop, Delete on an image in use is disabled. Hovering a disabled button says why.
- **Delete always confirms** in a sheet that names what goes: "Delete container `api-db-1`? Its writable layer is lost; volume `pgdata` (1.2 GB) is kept." It has a **Also delete its anonymous volumes** checkbox, unchecked. The button repeats the action: **Delete container**. For bulk: **Delete 4 containers**. A running container gets a **Stop and delete** button instead of a plain delete.
- **Kill** and **Delete volume** are styled as destructive (danger colour); Stop and Restart are not.
- **Every button has a shortcut and a palette command**, and the shortcut shows in its tooltip: Start/Stop ⌘. (toggle), Restart ⌘R, Delete ⌫, Logs ⌘L, Terminal ⌘T, Edit ⌘E.
- **Keyboard and screen reader:** every icon button has an accessible name ("Restart api-db-1"), is reachable with Tab within the selected row, and shows a visible focus ring.

### 7.3 Screens

| Screen | Role | Data shown | Primary action | States (n/a with reason) |
|---|---|---|---|---|
| **Projects** (home) | Developer | One row per project: name, folder path (shortened `~/Dev/app`), engine, git branch, services as status dots, ports, CPU and memory totals | **Start project** / **Stop project** on the row | default; empty ("Open a folder to see its containers here", button Open folder); loading (skeleton rows after 300 ms); partial (one engine unreachable: banner on its rows); error (engine down: "Colima default is stopped" + Start); offline n/a (local); no permission (socket not readable: show fix); long content (path truncates in the middle, full on hover); large text; dark |
| **Project detail** | Developer | Services table (state, image, ports, health, CPU, mem), compose files, `.env` keys (values hidden until clicked), combined logs | **Start / Stop project**; per service Restart, Rebuild, Logs, Terminal | default; not created (files found, nothing running); partial (some services failing: rows in error colour, Problems badge); error (compose file invalid: line and message); loading; dark |
| **Containers** | Developer | Table: status, name, project, image:tag, ports (clickable to open `localhost:port`), CPU, mem, uptime, engine. Group by project (default), engine, or none. Filters: state, engine, project, label | **Start / Stop** selected (bulk) | default; empty per filter ("No stopped containers"); loading; partial (stats unavailable: dashes, not zeros); error; long content; 500+ rows (virtualised); dark |
| **Container detail** (inspector or full page) | Developer | Overview (state, image, command, ports, mounts, networks, labels, health log), Logs, Terminal, Attach, Stats (sparklines 5 min), Inspect (JSON tree, searchable), Env, Files, Limits (CPU, memory, restart policy, editable live). Menu: Edit, Duplicate, Recreate with latest image, Rename, Pause, Kill, Save as image | **Restart** | default; exited (exit code, last 20 log lines, Problems link); restarting loop (count + "Stop the loop"); paused; removed while open ("This container was removed" + back); loading; dark |
| **Logs** (section, and a bottom panel tab) | Developer | Merged live lines from one container, a project, or any chosen set across engines; container prefix in colour; timestamps; stderr marked; error lines marked in the gutter; JSON lines summarised; stack traces folded | **Follow** toggle; search (regex, N of M), stream filter, level chips, time range, **Next error** (F8), **Explain**, **Ask about these lines**, Copy, Save | default; empty ("No output yet"); paused (follow off, "142 new lines" pill); disconnected (engine lost: "Reconnecting…", gap marked); very long (ring buffer of 100k lines per view, "Older lines dropped; Save to keep all"); burst (over 5k lines/s: rendering batches per frame, count shown); dark |
| **Terminal** (bottom panel tab and inspector tab; 6.16) | Developer | xterm.js session per container; tabs named "container · shell"; toolbar: shell, user, working dir, Reconnect, Clear, Copy, Split, Pop out, Open in Terminal app | type | default; several tabs (background output dot); split; connecting; stopped container ("Start and open terminal"); no shell ("This image has no shell. Browse its files instead."); exited (session ended, Reconnect); dark |
| **Open folder browser** (sheet; 6.18) | Developer | Places and Recent; column view with path bar and filter; Compose/Dockerfile/known markers on the visible column; detection preview of the highlighted folder | **Open <folder>** (or **Go to <project>**) | default; loading large folder; empty folder; no access (macOS privacy, Open System Settings); path not found; hidden files shown |
| **Drop folder** (overlay + Detected sheet; 6.15) | Developer | Full-window drop overlay with folder name and path; then detection results (files found, stack, services needed) | **Start project**, or **Generate from recipe** / **Generate with Claude Code ▾** | dragging (overlay); several folders (queue count); wrong file type; already known (jumps to project); detected with Compose; detected without Compose; detection failed (unreadable folder) |
| **Images** (tabs: On this engine · Find images) | Developer | On this engine: table of repo:tag, ID, size, arch (amd64 badge on arm64 host), created, used by. Find images (6.17): search across Docker Hub and added registries; results with publisher badge, stars, pulls, updated, arm64 support; detail panel with tags per architecture, pull command | **Pull image** | default; empty; loading; pulling (per-layer progress row); dangling filter; error (pull denied: reason); dark |
| **Image detail** | Developer | Tags, layers and history with sizes, containers using it, platform | **Run** (opens a run sheet: name, ports, env, volumes, prefilled) | default; in use (Remove disabled, reason shown: "Used by 2 containers"); dark |
| **Volumes** | Developer | Table: name, project, size, used by, driver, created | **Browse** | default; empty; size loading (sizes fill in later, "Calculating…"); unused filter; error; dark |
| **Volume browser** | Developer | File tree inside the volume, file preview, sizes | **Back up volume** (to .tar.gz) | default; empty volume; large dir (virtualised); read error; dark |
| **Networks** | Developer | Table: name, driver, subnet, containers, project | **Create network** | default; built-in networks marked and not removable (reason shown); dark |
| **Engines** | Developer | One row per engine: state, Docker version, CPU/mem/disk, arch, Rosetta, VM type, container counts | **Start / Stop engine**; **Add engine** | default; stopped; starting (progress, ~20 s); unreachable (remote: last seen, retry); dark |
| **Engine dashboard** | Developer | Counts (projects, containers running/stopped/unhealthy, images, volumes, networks), disk-usage bar by type, engine info (OS, kernel, Docker and Compose versions, storage driver), recent activity | **Clean up**; **Edit resources** (Colima) | default; stopped; editing (restart required notice: "Colima restarts; running containers stop for ~20 s"); unreachable; dark |
| **Edit and recreate** (sheet) | Developer | Container settings form (image, command, env, ports, mounts, networks, restart, limits), settings diff, data-loss warning | **Apply limits** (live) or **Recreate container** | default; limits-only change; recreating (progress); failed (old container kept, reason shown); Compose service (offers "Write to compose.yaml instead") |
| **New project** (sheet) | Developer | Three sources: Open folder, From Git, From editor (paste/write Compose) | **Create project** | default; cloning (progress); clone failed (auth or URL, with fix); invalid Compose (line and message) |
| **Templates** | Developer | Grid-free list grouped by category: name, version, image, ports, short description; built-in and custom | **Add to project** | default; custom empty ("Save any service as a template from its menu"); dark |
| **Registries** (M4) | Developer | Name, URL, username, status | **Add registry** | default; empty (Docker Hub only, logged out); login failed (reason); dark |
| **Build image** (sheet, M4) | Developer | Context folder or Dockerfile editor, tags, build args, platform (arm64/amd64), live build log | **Build image** | default; building (step N of M, log); failed (failing step highlighted, Problems link); done (size, tag, **Push** next) |
| **Create volume / Create network** (sheets) | Developer | Name, driver, options; for networks subnet, gateway, internal flag, attach containers | **Create volume** / **Create network** | default; name taken; invalid subnet (inline) |
| **Activity** | Developer | Live table: time, engine, origin (Engine / Berth / plugin), action, subject, outcome. Filters by engine, project, origin, outcome | **Pause feed** | default; empty ("Nothing has happened on this engine yet"); paused ("23 new events"); engine disconnected (gap marked in the feed); long history (virtualised, kept 30 days); dark |
| **Clean up** (sheet) | Developer | Grouped list of removable items with sizes, total | **Remove N items (X GB)** | default; nothing to clean; removing (progress); partial failure (which items failed and why); done (same total) |
| **Review changes** (sheet) | Developer | File tree, unified or split diff per file, summary line, source (Recipe / Claude Code / muse) | **Write files** / **Write files and start** | default; conflicts (file changed since generated: regenerate); write error; empty plan (nothing to change) |
| **Generate** (sheet) | Developer | Detection result, recipe choice, editable options (ports, database, Node version), harness choice | **Generate** | detecting; detected; low confidence ("Not sure this is a Django app: pick the stack"); no harness (AI option hidden, recipe stays) |
| **Problems** (bottom tab) | Developer | List grouped by project: severity, title, subject, age | **Apply fix** on the selected problem | default; empty ("No problems"); fix running; fix failed |
| **Assistant** (bottom tab, or full section; M3) | Developer | Harness picker (Claude Code / muse, auto-detected); conversation per project; context chips (log lines, problem, Compose file, Dockerfile, folder); streamed reply; tool-call rows; proposed changes as cards; approval prompts for change and destructive tools | **Review changes** on a proposed change; **Approve** on a tool call | default; no harness found (how to install, rules still work); thinking (streaming); waiting for approval (the tool, its arguments, Approve / Deny); watch mode on (banner, Stop watching); harness error (stderr shown, Retry); stopped; dark |
| **Menu bar menu** | Developer | Overall state icon; engines with state; pinned and running projects; failing containers | **Start / Stop** per row; **Open Berth** | all good; something failing (icon marked, failing list first); engine stopped; Berth window closed (menu still works) |
| **Command palette** | Developer | Commands, resources and recent items matching the query, with shortcuts | Enter runs | default; no match ("Run as docker command?" executes the typed CLI in a terminal tab); dark |
| **Global search** (palette mode) | Developer | Results across all engines grouped by type, matching name, tag, port (`:5432`), label, folder | Enter opens | default; empty; one engine slow (results stream in, engine marked) |
| **Plugins** | Developer | Installed plugins, permissions, state | **Enable** / **Configure** | default; error (plugin crashed: log); permission request |
| **Coolify section** (plugin) | Developer | Resources by project and environment (apps, services, databases with status and domain), servers (IP, proxy, reachability), deployments (status, commit and message, server, duration, webhook or API) | **Deploy** (with Force rebuild) | default; not configured (URL + token form); token invalid; instance unreachable; newer Coolify version warning; deploying (live build log, Cancel deployment); failed deploy (log scrolled to the failing step) |
| **Settings** | Developer | Engines, harnesses found (path and version, Rescan), appearance (light/dark/system, density), shortcuts, recipes folder, telemetry (off) | n/a | default |
| **First run** | Developer | Checks with pass/fail: Colima, Docker CLI, socket, Compose plugin, harnesses | **Start Colima** or **Continue** | all pass; Colima missing (copy install command); stopped |

States not applicable everywhere: **right-to-left** n/a for v1 (English-only developer tool, decision in section 9); **offline** n/a for local engines, applies to remote engines and the Coolify plugin.

---

## 8. Glossary

One word per concept, in UI copy and in code identifiers.

| Term | Meaning | Never call it |
|---|---|---|
| Engine | One Docker endpoint: a Colima profile, a remote host, or a socket | context, host, environment, daemon |
| Project | A folder Berth knows, usually with a Compose file, and its containers | stack, app, workspace |
| Service | One entry under `services:` in a Compose file | component |
| Container | A running or stopped container | instance, pod |
| Image | A Docker image | build, artifact |
| Volume | A named Docker volume | disk, storage |
| Network | A Docker network | |
| Recipe | A rule-based generator that writes a Dockerfile and Compose file for a whole codebase | preset, scaffold |
| Template | A ready-made service (Postgres, Redis…) added to a project | app template, stack template, preset |
| Registry | A place images are pulled from and pushed to | repository, hub |
| Activity | The feed of engine events and actions taken in Berth | events, audit log, history |
| Recreate | Remove a container and create it again with new settings | redeploy (that word is for projects and Coolify) |
| Harness | An installed AI agent CLI (Claude Code, muse) | bot, copilot, model |
| Problem | Something Berth detected as wrong | issue, alert, error (as a noun for the item) |
| Fix | A proposed change that resolves a problem | solution, patch |
| Review changes | The diff screen every file write goes through | preview, apply |
| Plugin | An external extension such as Coolify | extension, add-on, integration |

---

## 9. Open questions, with the default taken

| Question | Default taken | What would change it |
|---|---|---|
| ~~"Container": Portainer or Apple `container`?~~ | **Answered 2026-10-24: Portainer.** Developer-relevant Portainer features added (section 1.1) | — |
| ~~Who is it for?~~ | **Answered: for Emmanuel, released as open source; no accounts now, maybe later** (section 1.2) | — |
| Product name? | "Berth" as a placeholder | Your pick; check the name is free on GitHub, crates.io and Homebrew |
| Licence? | MIT OR Apache-2.0 | GPL-3.0 if you want forks to stay open |
| What will accounts be for, when they come? | Not designed; the data model only keeps `actor` and owner fields | Syncing settings across Macs, or sharing remote engines with other people (roles) |
| Git deploys: redeploy on new commits automatically? | Off by default; polling shows "N new commits" and you press **Pull and redeploy** | You want push-to-deploy like Portainer's GitOps |
| Dioxus renderer: desktop (webview) or the native renderer (Blitz)? | Desktop webview: mature, and it can host xterm.js for the terminal and CodeMirror for Compose editing | If the native renderer is stable when building starts and benchmarks better on 500-row tables |
| Which Colima runtimes matter: Docker only, or containerd and Incus too? | Docker only (bollard speaks the Docker API) | containerd would need a second client (nerdctl) and a different data model |
| ~~Scan folders for Compose files?~~ | **Answered: no scanning.** Projects come from running containers plus folders you pin | — |
| ~~How should the AI work?~~ | **Answered: interactive.** Multi-turn sessions driven by Berth, with Docker tools over MCP, log reading and watch mode (6.12) | — |
| Can a harness edit files directly, or only propose diffs? | Only propose; Berth writes after Review changes | You trust it to write and you review in git |
| Plugin format: separate process (JSON-RPC over stdio, any language) or WASM? | Separate process with a manifest declaring permissions and UI contributions as declarative views (table, detail, actions, log stream); plugins draw no custom UI | Plugins need custom UI, or you want sandboxing in-process (WASM) |
| Which Coolify actions in v1? | From the v4.5.0 spec: read resources, servers, deployments, app logs and envs; deploy (and force), start, stop, restart, cancel deployment | You also want env editing, rollback or creating apps from Berth |
| Should watch mode ever apply fixes without a click? | Never; it proposes, you approve | You want auto-apply for low-risk fixes (for example a port change) |
| Remote engines: SSH only, or also TCP+TLS? | Both, SSH first | |
| Languages? | English only, no RTL | Any other market |
| ~~Menu bar?~~ | **Answered: yes** (6.14) | — |

---

## 10. Instructions for the AI building the UI

1. **Build from this brief, in this order:** shell and navigation → Projects → Containers → Container detail with Logs, Terminal and Limits → Edit and recreate → Images → Volumes → Networks (with create sheets) → Engines and Engine dashboard → Logs section → Activity → menu bar → palette and global search → New project, Generate and Review changes → Templates → Problems. M3 and M4 screens (Assistant, Registries, Build image, From Git, remote engines, plugins) come after.
2. **Use mock data typed by section 5.** Put mocks in one module (`src/mocks/`) and read them through a trait (`trait EngineClient`) so the real `bollard` client replaces them without touching UI code. Mocks must include: 3 engines (one stopped, one remote unreachable), 6 projects (one failing, one partial, one pinned and not created), 40+ containers, an amd64 image on arm64, a 1.2 GB volume, a restart loop, and long names and paths.
3. **Dense by default.** 13px UI text, 28px table rows, 12px mono for IDs and logs, tabular figures for every number. No card grids for lists; tables are the default view.
4. **Every screen has every state in section 7.** Build the empty, loading, error and partial states alongside the default, not after.
5. **Every write goes through Review changes.** No button writes a file or removes data without showing what changes, with the same counts and sizes on the button and on the result.
6. **Keyboard first.** Every action in the UI is in the palette with its shortcut. Focus is always visible.
7. **Copy rules:** buttons are verb + object ("Start project", "Remove 3 images"), never "OK" or "Submit". Errors say what happened and what to do. Container IDs show 12 characters in mono, with Copy on hover.
8. **Status colour is meaning, not decoration:** running, stopped, restarting, unhealthy, exited with error, and created each have a fixed colour and an icon or label, never colour alone.
9. **Performance:** meet the budget in section 1.3. Virtualise every list over 100 rows; batch log lines per animation frame; subscribe to stats only for visible rows; drive updates from `docker events`, not polling; never block the UI thread on the engine.
10. **Design tokens** come from `design/tokens.json` once the system stage has run; until then, use CSS variables in one file and no raw colours in components.
11. **AI is a tool user, not a file writer:** the harness reaches Docker only through Berth's MCP tools (6.12), and every file change comes back as a proposed change for Review changes.
12. **Actions are visible buttons** (section 7.2): Start, Stop, Restart, Pause, Kill and Delete are on-screen buttons on rows, detail headers and the bulk bar, never only in menus or shortcuts.
