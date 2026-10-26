//! First run (brief 6.1, P6-T2): what Berth needs on this Mac and whether it is there.
//!
//! Two halves. [`evaluate`] is pure: it turns what a [`Probe`] found, the engines discovery
//! listed and the harnesses `berth_assistant::detect` found into the rows of the checks card
//! (the same rows fill the full-screen setup and the card on Projects empty). [`refresh`] does the
//! I/O off the UI path: it finds the binaries through `berth_sys::SearchPath`, reads their
//! versions, runs `detect` and puts the results in the [`FirstRunStore`] and `AppState::harnesses`.
//!
//! Berth never installs anything: the setup screen shows the Homebrew command to copy. The one
//! thing it can do is start a stopped Colima profile ([`start_plan`], run through
//! `ActionsService::engine`).

use std::path::{Path, PathBuf};

use dioxus::prelude::*;

use crate::state::{AppState, Section};

/// The command the setup screen offers to copy (brief 6.1).
pub const INSTALL_COMMAND: &str = "brew install colima docker docker-compose";

/// The line under the rows (frame "Projects / empty / 1440").
pub const HARNESS_NOTE: &str =
    "Berth works without the assistants; they only add Fix with Claude Code.";

/// What a new Colima profile named `default` is started with when none exists yet (brief 6.1).
pub const NEW_PROFILE_CPUS: u16 = 2;
pub const NEW_PROFILE_MEMORY_GIB: u32 = 8;

/// What finding the tools on this Mac turned up. Plain facts from I/O, so tests build them by hand.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Probe {
    pub colima: Option<PathBuf>,
    pub colima_version: Option<String>,
    pub docker: Option<PathBuf>,
    /// The client version (`docker --version`); the server's comes from the engine.
    pub docker_version: Option<String>,
    pub compose_version: Option<String>,
}

impl Probe {
    /// Everything present, for `BERTH_MOCK=1` and scenes (the mock world has Colima 0.10.3).
    #[must_use]
    pub fn all_found() -> Self {
        Self {
            colima: Some(PathBuf::from("/opt/homebrew/bin/colima")),
            colima_version: Some("0.10.3".to_string()),
            docker: Some(PathBuf::from("/opt/homebrew/bin/docker")),
            docker_version: Some("28.4.0".to_string()),
            compose_version: Some("2.39.1".to_string()),
        }
    }
}

/// How one row reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Present and working.
    Pass,
    /// Present but not ready (a stopped profile).
    Warn,
    /// Missing or broken.
    Fail,
    /// Not installed and that is fine (an assistant).
    Absent,
}

/// One line of the checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub label: String,
    pub status: Status,
    /// The right-hand text: a version, "running", "not found".
    pub value: String,
    /// A second line under a failing row.
    pub hint: Option<String>,
}

/// The Colima command "Start default" runs, and how its button reads.
#[derive(Debug, Clone, PartialEq)]
pub struct StartPlan {
    pub engine: berth_core::Engine,
    pub op: berth_core::ColimaOp,
    pub label: String,
}

/// The checks as the screens draw them.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub rows: Vec<Row>,
    pub colima_missing: bool,
    pub docker_missing: bool,
    pub start: Option<StartPlan>,
}

impl Report {
    /// The full-screen setup shows only when Colima or Docker is missing; every other state (a
    /// stopped profile, a missing Compose plugin, no assistants) is a card on Projects empty.
    #[must_use]
    pub fn needs_setup(&self) -> bool {
        self.colima_missing || self.docker_missing
    }

    /// The row with this id.
    #[must_use]
    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|row| row.id == id)
    }
}

fn colima_profiles(engines: &[berth_core::Engine]) -> Vec<&berth_core::Engine> {
    engines
        .iter()
        .filter(|e| matches!(e.kind, berth_core::EngineKind::Colima { .. }))
        .collect()
}

/// The socket path of an engine whose `docker_host` is a unix socket.
fn socket_of(engine: &berth_core::Engine) -> Option<PathBuf> {
    engine
        .docker_host
        .as_deref()
        .and_then(|host| host.strip_prefix("unix://"))
        .map(PathBuf::from)
}

fn row(
    id: &str,
    label: &str,
    status: Status,
    value: impl Into<String>,
    hint: Option<String>,
) -> Row {
    Row {
        id: id.to_string(),
        label: label.to_string(),
        status,
        value: value.into(),
        hint,
    }
}

/// The engine row of a profile Colima has not created yet.
fn new_default_engine() -> berth_core::Engine {
    berth_core::Engine {
        id: berth_core::EngineId("colima:default".to_string()),
        name: "default".to_string(),
        kind: berth_core::EngineKind::Colima {
            profile: "default".to_string(),
        },
        state: berth_core::EngineState::Stopped,
        docker_host: None,
        docker_version: None,
        resources: None,
        counts: berth_core::ResourceCounts::default(),
        owner: berth_core::OwnerId::Local,
    }
}

/// What the Start button runs: a stopped profile is started as it is (its sizes are the owner's),
/// and when Colima has no profile yet, `default` is created with 2 CPU and 8 GB (brief 6.1).
#[must_use]
pub fn start_plan(engines: &[berth_core::Engine]) -> StartPlan {
    let profiles = colima_profiles(engines);
    let stopped: Vec<&&berth_core::Engine> = profiles
        .iter()
        .filter(|e| e.state == berth_core::EngineState::Stopped)
        .collect();
    let chosen = stopped
        .iter()
        .find(|e| e.name == "default")
        .or_else(|| stopped.first());
    if let Some(engine) = chosen {
        return StartPlan {
            engine: (***engine).clone(),
            op: berth_core::ColimaOp::Start,
            label: format!("Start {}", engine.name),
        };
    }
    StartPlan {
        engine: new_default_engine(),
        op: berth_core::ColimaOp::Apply(berth_core::ColimaStartOptions {
            cpus: Some(NEW_PROFILE_CPUS),
            memory_gib: Some(NEW_PROFILE_MEMORY_GIB),
            disk_gib: None,
            vm_type: None,
            rosetta: None,
            arch: None,
        }),
        label: format!("Start default ({NEW_PROFILE_CPUS} CPU, {NEW_PROFILE_MEMORY_GIB} GB)"),
    }
}

/// "running" for the profile named default (the frame), "work running" for any other.
fn profile_state(engine: &berth_core::Engine, word: &str) -> String {
    if engine.name == "default" {
        word.to_string()
    } else {
        format!("{} {word}", engine.name)
    }
}

/// The Colima row, with the socket check folded in.
fn colima_row(
    probe: &Probe,
    engines: &[berth_core::Engine],
    socket_exists: &dyn Fn(&Path) -> bool,
) -> (Row, Option<StartPlan>) {
    if probe.colima.is_none() {
        return (
            row(
                "colima",
                "Colima",
                Status::Fail,
                "not found",
                Some(format!("Install it with {INSTALL_COMMAND}")),
            ),
            None,
        );
    }
    let version = probe.colima_version.as_deref().unwrap_or("installed");
    let profiles = colima_profiles(engines);
    let running = profiles
        .iter()
        .find(|e| e.state == berth_core::EngineState::Running && e.name == "default")
        .or_else(|| {
            profiles
                .iter()
                .find(|e| e.state == berth_core::EngineState::Running)
        });
    if let Some(engine) = running {
        let socket = socket_of(engine);
        if let Some(path) = socket.filter(|path| !socket_exists(path)) {
            return (
                row(
                    "colima",
                    "Colima",
                    Status::Fail,
                    format!("{version} \u{b7} no socket"),
                    Some(format!(
                        "{} is running but {} is missing; try colima restart {}",
                        engine.name,
                        path.display(),
                        engine.name
                    )),
                ),
                None,
            );
        }
        return (
            row(
                "colima",
                "Colima",
                Status::Pass,
                format!("{version} \u{b7} {}", profile_state(engine, "running")),
                None,
            ),
            None,
        );
    }
    if let Some(engine) = profiles
        .iter()
        .find(|e| e.state == berth_core::EngineState::Starting)
    {
        return (
            row(
                "colima",
                "Colima",
                Status::Warn,
                format!("{version} \u{b7} {}", profile_state(engine, "starting")),
                None,
            ),
            None,
        );
    }
    if let Some(engine) = profiles
        .iter()
        .find(|e| matches!(e.state, berth_core::EngineState::Unreachable { .. }))
    {
        return (
            row(
                "colima",
                "Colima",
                Status::Fail,
                format!(
                    "{version} \u{b7} {}",
                    profile_state(engine, "not answering")
                ),
                Some("Colima says it runs but the Docker socket does not answer.".to_string()),
            ),
            None,
        );
    }
    let plan = start_plan(engines);
    let value = if profiles.is_empty() {
        format!("{version} \u{b7} no profile yet")
    } else {
        format!(
            "{version} \u{b7} {}",
            profile_state(&plan.engine, "stopped")
        )
    };
    (
        row("colima", "Colima", Status::Warn, value, None),
        Some(plan),
    )
}

/// The server version of the first engine that reports one, else the client's.
fn docker_value(probe: &Probe, engines: &[berth_core::Engine]) -> String {
    engines
        .iter()
        .find_map(|e| e.docker_version.clone())
        .or_else(|| probe.docker_version.clone())
        .unwrap_or_else(|| "found".to_string())
}

/// The checks for what was found. `socket_exists` is injected so a test needs no real socket.
#[must_use]
pub fn evaluate(
    probe: &Probe,
    engines: &[berth_core::Engine],
    harnesses: &[berth_core::Harness],
    socket_exists: &dyn Fn(&Path) -> bool,
) -> Report {
    let (colima, start) = colima_row(probe, engines, socket_exists);
    let docker = if probe.docker.is_some() {
        row(
            "docker",
            "Docker",
            Status::Pass,
            docker_value(probe, engines),
            None,
        )
    } else {
        row(
            "docker",
            "Docker",
            Status::Fail,
            "not found",
            Some(format!("Install it with {INSTALL_COMMAND}")),
        )
    };
    let compose = if probe.docker.is_none() {
        row(
            "compose",
            "Compose plugin",
            Status::Fail,
            "needs Docker",
            None,
        )
    } else if probe.compose_version.is_some() {
        row("compose", "Compose plugin", Status::Pass, "found", None)
    } else {
        row(
            "compose",
            "Compose plugin",
            Status::Fail,
            "not found",
            Some("Install it with brew install docker-compose".to_string()),
        )
    };
    let mut rows = vec![colima, docker, compose];
    for harness in harnesses {
        rows.push(if harness.available {
            row(
                &harness.id.0,
                &harness.name,
                Status::Pass,
                harness
                    .version
                    .clone()
                    .unwrap_or_else(|| "found".to_string()),
                None,
            )
        } else {
            row(
                &harness.id.0,
                &harness.name,
                Status::Absent,
                "not found",
                None,
            )
        });
    }
    Report {
        rows,
        colima_missing: probe.colima.is_none(),
        docker_missing: probe.docker.is_none(),
        start,
    }
}

// ---------------------------------------------------------------------
// Version parsing
// ---------------------------------------------------------------------

/// "Docker version 29.8.0, build 1234" gives "29.8.0".
#[must_use]
pub fn parse_docker_version(stdout: &str) -> Option<String> {
    let rest = stdout
        .lines()
        .next()?
        .trim()
        .strip_prefix("Docker version ")?;
    let version = rest.split([',', ' ']).next()?.trim();
    (!version.is_empty()).then(|| version.to_string())
}

/// "Docker Compose version v5.5.1" gives "5.5.1"; the plugin's first line is all that counts.
#[must_use]
pub fn parse_compose_version(stdout: &str) -> Option<String> {
    let rest = stdout
        .lines()
        .next()?
        .trim()
        .strip_prefix("Docker Compose version ")?;
    let version = rest.split_whitespace().next()?.trim_start_matches('v');
    (!version.is_empty()).then(|| version.to_string())
}

async fn version_of(
    binary: &Path,
    args: &[&str],
    search: &berth_sys::SearchPath,
) -> Option<String> {
    let args: Vec<std::ffi::OsString> = args.iter().map(std::ffi::OsString::from).collect();
    let output = berth_sys::run_capture(
        binary,
        &args,
        berth_sys::SpawnOpts {
            path: Some(search.path_var()),
            timeout: Some(std::time::Duration::from_secs(5)),
            ..berth_sys::SpawnOpts::default()
        },
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .ok()?;
    (output.code == Some(0)).then_some(output.stdout)
}

/// Find the tools through `search` and read their versions. Read-only: only `--version`-style
/// commands run.
pub async fn probe(search: &berth_sys::SearchPath) -> Probe {
    let colima = search.find("colima");
    let docker = search.find("docker");
    let (colima_version, docker_version, compose_version) = tokio::join!(
        async {
            match &colima {
                Some(binary) => version_of(binary, &["version"], search)
                    .await
                    .and_then(|out| crate::services::colima::parse_version(&out)),
                None => None,
            }
        },
        async {
            match &docker {
                Some(binary) => version_of(binary, &["--version"], search)
                    .await
                    .and_then(|out| parse_docker_version(&out)),
                None => None,
            }
        },
        async {
            match &docker {
                Some(binary) => version_of(binary, &["compose", "version"], search)
                    .await
                    .and_then(|out| parse_compose_version(&out)),
                None => None,
            }
        },
    );
    Probe {
        colima,
        colima_version,
        docker,
        docker_version,
        compose_version,
    }
}

// ---------------------------------------------------------------------
// The store the screens read
// ---------------------------------------------------------------------

/// What the last scan found, shared by Projects empty, the full-screen setup and Settings. `None`
/// until the first scan answers.
#[derive(Clone, Copy)]
pub struct FirstRunStore {
    pub probe: Signal<Option<Probe>>,
    /// A scan is running (Rescan shows it).
    pub scanning: Signal<bool>,
    /// Look for a running profile's socket on disk. Scenes turn it off, so a frame reads the same
    /// on a Mac without Colima.
    pub check_sockets: Signal<bool>,
}

impl FirstRunStore {
    /// The socket test the checks use.
    fn socket_check(&self) -> impl Fn(&Path) -> bool {
        let on = *self.check_sockets.peek();
        move |path: &Path| !on || path.exists()
    }

    /// The checks for what the last scan found, against the engines and harnesses now; `None`
    /// until a scan has answered.
    #[must_use]
    pub fn report(
        &self,
        engines: &[berth_core::Engine],
        harnesses: &[berth_core::Harness],
    ) -> Option<Report> {
        let probe = self.probe.read().clone()?;
        Some(evaluate(&probe, engines, harnesses, &self.socket_check()))
    }
}

/// Create the store signals; the App root and a scene root call this once through `providers`.
pub fn provide_first_run_store() -> FirstRunStore {
    use_context_provider(|| FirstRunStore {
        probe: Signal::new(None),
        scanning: Signal::new(false),
        check_sockets: Signal::new(true),
    })
}

/// How a scan finishes: whether the first answer may send the window to the setup screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scan {
    /// The launch scan: a missing Colima or Docker opens the setup screen.
    Launch,
    /// A Rescan or Settings: results only.
    Again,
}

/// Scan this Mac: find the tools, read their versions, run `detect`. Runs off the UI path;
/// results land in the store and in `state.harnesses`. `mock` (`BERTH_MOCK=1`) reports every tool
/// present, so a mock run never opens the setup screen; `detect` only ever reads `--version`.
pub fn refresh(mut state: AppState, store: FirstRunStore, mock: bool, scan: Scan) {
    let mut scanning = store.scanning;
    if *scanning.peek() {
        return;
    }
    scanning.set(true);
    let overrides = state.settings.peek().harness_paths.clone();
    let io = tokio::spawn(async move {
        let search = crate::services::login_path::search_path().await;
        let found = if mock {
            Probe::all_found()
        } else {
            probe(&search).await
        };
        let harnesses = berth_assistant::detect(&search, &overrides).await;
        (found, harnesses)
    });
    spawn(async move {
        let result = io.await;
        scanning.set(false);
        let Ok((found, harnesses)) = result else {
            tracing::warn!("first-run scan task failed");
            return;
        };
        let mut probe_signal = store.probe;
        let setup = scan == Scan::Launch
            && evaluate(
                &found,
                &state.engines.peek(),
                &harnesses,
                &store.socket_check(),
            )
            .needs_setup();
        probe_signal.set(Some(found));
        state.harnesses.set(harnesses);
        if setup && *state.section.peek() == Section::Projects {
            state.section.set(Section::FirstRun);
        }
    });
}

/// Run a [`StartPlan`]: puts a profile Colima has not created into the engine list (so the
/// Starting row has somewhere to show), then starts it through the actions service.
///
/// # Errors
/// The actions service refused (a start is already running, say).
pub fn run_start(
    state: &mut AppState,
    actions: &crate::services::actions::ActionsService,
    plan: &StartPlan,
) -> Result<(), String> {
    let known = state.engines.peek().iter().any(|e| e.id == plan.engine.id);
    if !known {
        state.engines.write().push(plan.engine.clone());
    }
    actions.engine(state, &plan.engine, plan.op.clone())
}

#[cfg(test)]
#[path = "first_run_tests.rs"]
mod tests;
