//! The problems service (P5-T8; PLAN D35, D82, AD27, AD31).
//!
//! It keeps `AppState::problems` current. The rules (`berth_rules::problems::evaluate`) are pure and
//! run once per engine; this file owns everything around them:
//!
//! - **When.** On engine events (the App root nudges it when containers, projects, engines or
//!   pending actions change), after a Compose failure (the failing command's lines come from
//!   `ActionsService::compose_failure_lines`), and when a log batch holds error-marked lines
//!   ([`tap_errors`], called by the logs service). Nudges are coalesced: one pass per
//!   [`DEBOUNCE`], and a pass whose inputs did not change is skipped.
//! - **What the rules see.** Per engine: its containers, with the inspect and the end of the log of
//!   the ones that can be in trouble (a running, healthy container with no restarts is not read),
//!   its projects with their Compose text and `.env` text, the host ports taken, the disk usage
//!   (asked every [`DISK_EVERY`], never waited for) and the failure lines of each project's last
//!   failed Compose command. All I/O runs off the UI thread.
//! - **What the list keeps.** A [`Tracker`]: one entry per `ProblemId`, `first_seen` kept while the
//!   problem lasts, a fixed slot per problem so the list never reorders under the pointer (AD31;
//!   problems found in one pass take slots by severity, title and id; later ones come after),
//!   and a problem gone from the rules' output is removed (resolved).
//! - **Explain.** [`ProblemsService::explain`] runs the same rules over the lines a person selected
//!   in a log view and returns what they say, without touching the list.
//!
//! Env values are read only to mask them in evidence (the rules do that) and for the `EnvVarMissing`
//! fix. Nothing here logs a problem, a fact or a line.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use futures::StreamExt;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use berth_core::{
    Container, ContainerDetail, ContainerId, ContainerState, DiskUsage, Engine, EngineId,
    EngineState, Health, LogLine, LogQuery, Problem, ProblemId, Project, ProjectId, Stream,
    Subject,
};
use berth_engine::EngineClient;
use berth_rules::problems::{evaluate, ContainerFacts, ProblemInput, ProjectFacts};

use crate::services::actions::ActionsService;
use crate::services::detail::Signature;
use crate::services::plans::off_ui;
use crate::services::Services;
use crate::state::AppState;

/// Nudges inside this window make one pass.
pub const DEBOUNCE: Duration = Duration::from_millis(300);
/// How often an engine's disk usage is asked again (`system df` is slow on a big engine).
pub const DISK_EVERY: Duration = Duration::from_secs(300);
/// Lines of a container's log the rules read.
pub const LOG_TAIL: u32 = 40;
/// Error lines kept per container from log batches.
pub const KEEP_ERRORS: usize = 20;
/// The longest an error line from a log batch is kept (a minified stack can be huge).
const LINE_CHARS: usize = 400;
/// Containers read at the same time.
const READ_AT_ONCE: usize = 8;
/// The biggest Compose or `.env` file read.
const FILE_CAP: u64 = 256 * 1024;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------

/// One engine's problems at a time, merged into one list with a stable order.
#[derive(Debug, Default)]
pub struct Tracker {
    /// Slot and first sighting of every problem in the list.
    slots: HashMap<ProblemId, (u64, DateTime<Utc>)>,
    next_slot: u64,
    engines: BTreeMap<EngineId, Vec<Problem>>,
}

impl Tracker {
    /// Replace what `engine` has with `found` (the output of one `evaluate`). A problem seen before
    /// keeps its `first_seen` and its slot; a new one takes the next slots, the ones of one pass
    /// ordered by severity, title and id; one that is no longer found is gone, and if it comes back
    /// it is new.
    pub fn update(&mut self, engine: &EngineId, mut found: Vec<Problem>) {
        found.sort_by(|a, b| (a.severity, &a.title, &a.id).cmp(&(b.severity, &b.title, &b.id)));
        for problem in &mut found {
            if !self.slots.contains_key(&problem.id) {
                self.slots
                    .insert(problem.id.clone(), (self.next_slot, problem.first_seen));
                self.next_slot += 1;
            }
            if let Some((_, first_seen)) = self.slots.get(&problem.id) {
                problem.first_seen = *first_seen;
            }
        }
        self.engines.insert(engine.clone(), found);
        self.forget_resolved();
    }

    /// Keep only the engines that still exist.
    pub fn retain_engines(&mut self, keep: &[EngineId]) {
        self.engines.retain(|id, _| keep.contains(id));
        self.forget_resolved();
    }

    fn forget_resolved(&mut self) {
        let live: HashSet<&ProblemId> = self.engines.values().flatten().map(|p| &p.id).collect();
        self.slots.retain(|id, _| live.contains(id));
    }

    /// Every problem, in slot order.
    #[must_use]
    pub fn list(&self) -> Vec<Problem> {
        let mut all: Vec<&Problem> = self.engines.values().flatten().collect();
        all.sort_by_key(|p| self.slots.get(&p.id).map_or(u64::MAX, |(slot, _)| *slot));
        all.into_iter().cloned().collect()
    }
}

// ---------------------------------------------------------------------
// What one engine's rules are given
// ---------------------------------------------------------------------

/// Everything `check_engine` needs, copied from the signals so the work can leave the UI thread.
#[derive(Clone)]
pub struct EngineSnapshot {
    pub engine: Engine,
    pub containers: Vec<Container>,
    pub projects: Vec<Project>,
    /// The lines of each project's last failed Compose command.
    pub failures: HashMap<ProjectId, Vec<String>>,
    /// Error lines log batches have shown, by container.
    pub log_errors: HashMap<ContainerId, Vec<String>>,
    pub disk: Option<DiskUsage>,
}

/// What was read of one container, good while its list row has this signature.
#[derive(Clone)]
pub(crate) struct Read {
    signature: Signature,
    detail: Option<ContainerDetail>,
    last_lines: Vec<String>,
    error_lines: Vec<String>,
}

pub(crate) type Cache = Mutex<HashMap<ContainerId, Read>>;

/// A container that is running, healthy and has never restarted cannot have a problem the rules
/// find in its inspect or its last lines; every other state is read.
fn worth_reading(c: &Container) -> bool {
    !(c.state == ContainerState::Running
        && matches!(c.health, None | Some(Health::Healthy | Health::Starting))
        && c.restart_count == 0)
}

fn is_error_line(text: &str, stream: Stream) -> bool {
    berth_logs::classify(text, stream).1
}

/// The inspect and the end of the log of `c`.
async fn read_container(
    client: &dyn EngineClient,
    c: &Container,
) -> (Option<ContainerDetail>, Vec<String>, Vec<String>) {
    let detail = client.container(&c.id).await.ok();
    let query = LogQuery {
        since: None,
        tail: Some(LOG_TAIL),
        follow: false,
    };
    let cancel = CancellationToken::new();
    let mut stream = client.logs(&c.id, query, cancel.clone());
    let collect = async {
        let mut lines: Vec<(Stream, String)> = Vec::new();
        while let Some(Ok(line)) = stream.next().await {
            lines.push((line.stream, line.text));
        }
        lines
    };
    let lines = tokio::time::timeout(Duration::from_secs(5), collect)
        .await
        .unwrap_or_default();
    cancel.cancel();
    let error_lines = lines
        .iter()
        .filter(|(stream, text)| is_error_line(text, *stream))
        .map(|(_, text)| text.clone())
        .collect();
    let last_lines = lines.into_iter().map(|(_, text)| text).collect();
    (detail, last_lines, error_lines)
}

fn clip(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= LINE_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(LINE_CHARS).collect();
    format!("{cut}\u{2026}")
}

/// `existing` plus `more`, each line once, the newest [`KEEP_ERRORS`].
fn merged(existing: &[String], more: &[String]) -> Vec<String> {
    let mut out: Vec<String> = existing.to_vec();
    for line in more {
        if !out.contains(line) {
            out.push(line.clone());
        }
    }
    let skip = out.len().saturating_sub(KEEP_ERRORS);
    out.split_off(skip)
}

async fn container_facts(
    client: Option<&dyn EngineClient>,
    cache: &Cache,
    c: Container,
    log_errors: &[String],
) -> ContainerFacts {
    let signature = Signature::of(&c);
    let cached = lock(cache)
        .get(&c.id)
        .filter(|read| read.signature == signature)
        .cloned();
    let read = match (cached, client) {
        (Some(read), _) => read,
        (None, Some(client)) if worth_reading(&c) => {
            let (detail, last_lines, error_lines) = read_container(client, &c).await;
            let read = Read {
                signature,
                detail,
                last_lines,
                error_lines,
            };
            lock(cache).insert(c.id.clone(), read.clone());
            read
        }
        (None, _) => Read {
            signature,
            detail: None,
            last_lines: Vec::new(),
            error_lines: Vec::new(),
        },
    };
    ContainerFacts {
        error_lines: merged(&read.error_lines, log_errors),
        last_lines: read.last_lines,
        detail: read.detail,
        container: c,
    }
}

/// The first Compose file of `project` that can be read, and its `.env`. Blocking.
fn read_project_texts(project: &Project) -> (Option<String>, Option<String>) {
    let Some(folder) = &project.folder else {
        return (None, None);
    };
    let read = |path: &std::path::Path| -> Option<String> {
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() || meta.len() > FILE_CAP {
            return None;
        }
        std::fs::read_to_string(path).ok()
    };
    let compose = project
        .compose_files
        .iter()
        .map(|file| {
            if file.is_absolute() {
                file.clone()
            } else {
                folder.join(file)
            }
        })
        .find_map(|path| read(&path));
    (compose, read(&folder.join(".env")))
}

/// AD35: the strict binds (`create_host_path: false`) of the Compose text whose source is not
/// there. One `stat` each, no walk (D36). Blocking.
fn missing_binds(
    project: &Project,
    compose_text: Option<&str>,
) -> Vec<(String, std::path::PathBuf)> {
    let (Some(folder), Some(text)) = (&project.folder, compose_text) else {
        return Vec::new();
    };
    berth_rules::problems::strict_binds(folder, text)
        .into_iter()
        .filter(|(_, source)| {
            matches!(std::fs::metadata(source), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
        })
        .collect()
}

async fn project_facts(
    projects: Vec<Project>,
    mut failures: HashMap<ProjectId, Vec<String>>,
) -> Vec<ProjectFacts> {
    tokio::task::spawn_blocking(move || {
        projects
            .into_iter()
            .map(|project| {
                let (compose_text, env_text) = read_project_texts(&project);
                ProjectFacts {
                    failure_lines: failures.remove(&project.id).unwrap_or_default(),
                    missing_binds: missing_binds(&project, compose_text.as_deref()),
                    project,
                    compose_text,
                    env_text,
                    compose_error: None,
                }
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Give each container the project that lists it. The engine's container list does not say which
/// project a container belongs to (the grouping builds the projects from it afterwards), but the
/// rules do: a container's compose fix and the "same problem on the project" check both start from
/// `Container::project`.
fn link_projects(containers: &mut [Container], projects: &[Project]) {
    for container in containers.iter_mut().filter(|c| c.project.is_none()) {
        container.project = projects
            .iter()
            .find(|p| {
                p.engine == container.engine
                    && p.services
                        .iter()
                        .any(|s| s.containers.contains(&container.id))
            })
            .map(|p| p.id.clone());
    }
}

/// The Mac's architecture, which an image has to match (or be emulated for).
fn host_arch() -> berth_core::Arch {
    if cfg!(target_arch = "aarch64") {
        berth_core::Arch::Aarch64
    } else if cfg!(target_arch = "x86_64") {
        berth_core::Arch::X86_64
    } else {
        berth_core::Arch::Other(std::env::consts::ARCH.to_string())
    }
}

/// Gather one engine's facts and run the rules over them (AD27: one call per engine).
pub(crate) async fn check_engine(
    client: Option<&dyn EngineClient>,
    cache: &Cache,
    snapshot: EngineSnapshot,
    now: DateTime<Utc>,
) -> Vec<Problem> {
    let EngineSnapshot {
        engine,
        mut containers,
        projects,
        failures,
        log_errors,
        disk,
    } = snapshot;
    link_projects(&mut containers, &projects);
    let taken_ports = crate::services::ports::taken_ports(&containers);
    let ids: HashSet<ContainerId> = containers.iter().map(|c| c.id.clone()).collect();
    let facts: Vec<ContainerFacts> = futures::stream::iter(containers)
        .map(|c| {
            let seen = log_errors.get(&c.id).cloned().unwrap_or_default();
            async move { container_facts(client, cache, c, &seen).await }
        })
        .buffered(READ_AT_ONCE)
        .collect()
        .await;
    lock(cache).retain(|id, _| ids.contains(id));
    let input = ProblemInput {
        now,
        engine,
        host_arch: host_arch(),
        containers: facts,
        projects: project_facts(projects, failures).await,
        disk,
        taken_ports,
    };
    evaluate(&input)
}

// ---------------------------------------------------------------------
// The service
// ---------------------------------------------------------------------

/// Error lines log batches showed, with the list row they were seen under.
#[derive(Default)]
struct ErrorLines {
    signature: Option<Signature>,
    lines: Vec<String>,
}

struct DiskEntry {
    at: Instant,
    usage: Option<DiskUsage>,
    asking: bool,
}

struct Inner {
    wake: Notify,
    /// Bumped by what the signals do not show (error lines, a disk answer).
    version: AtomicU64,
    log_errors: Mutex<HashMap<ContainerId, ErrorLines>>,
    cache: Cache,
    disks: Mutex<HashMap<EngineId, DiskEntry>>,
    cancel: CancellationToken,
}

/// The problems service. Cheap to clone; the App root starts it and provides it as context.
#[derive(Clone)]
pub struct ProblemsService {
    inner: Arc<Inner>,
}

/// One engine's pass: the client that reads it and the facts to start from.
struct Job {
    client: Option<Arc<dyn EngineClient>>,
    snapshot: EngineSnapshot,
}

/// A line a person selected in a log view: the container's name as the view shows it, and the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainLine {
    pub container: String,
    pub text: String,
}

/// What the app currently knows, copied from the signals for a pass that leaves the UI thread.
#[derive(Debug, Clone, Default)]
pub struct World {
    pub engines: Vec<Engine>,
    pub containers: Vec<Container>,
    pub projects: Vec<Project>,
}

impl World {
    /// Read the signals (UI thread).
    #[must_use]
    pub fn of(state: &AppState) -> Self {
        Self {
            engines: state.engines.peek().clone(),
            containers: state.containers.peek().clone(),
            projects: state.projects.peek().clone(),
        }
    }
}

static TAP: Mutex<Option<ProblemsService>> = Mutex::new(None);

/// The logs service calls this with the error-marked lines of a batch it just prepared (D35, D82:
/// only those lines feed the rules). A no-op until a service has started.
pub fn tap_errors<'a>(lines: impl Iterator<Item = &'a LogLine>) {
    let service = lock(&TAP).clone();
    if let Some(service) = service {
        service.note_errors(lines);
    }
}

impl ProblemsService {
    fn new(cancel: CancellationToken) -> Self {
        Self {
            inner: Arc::new(Inner {
                wake: Notify::new(),
                version: AtomicU64::new(0),
                log_errors: Mutex::new(HashMap::new()),
                cache: Mutex::new(HashMap::new()),
                disks: Mutex::new(HashMap::new()),
                cancel,
            }),
        }
    }

    /// A service that never runs a pass (scenes, tests): `explain` still works.
    #[must_use]
    pub fn inert() -> Self {
        Self::new(CancellationToken::new())
    }

    /// Start from the App root (inside `use_hook`): the task that re-evaluates, and the tap the
    /// logs service feeds.
    #[must_use]
    pub fn start(mut state: AppState, services: Services, actions: ActionsService) -> Self {
        let service = Self::new(services.cancel.clone());
        *lock(&TAP) = Some(service.clone());
        let task = service.clone();
        spawn(async move {
            let mut tracker = Tracker::default();
            let mut last: Option<u64> = None;
            loop {
                tokio::select! {
                    () = task.inner.cancel.cancelled() => break,
                    () = task.inner.wake.notified() => {}
                }
                tokio::time::sleep(DEBOUNCE).await;
                let (jobs, engines, print) = task.collect(&state, &services, &actions);
                if last == Some(print) {
                    continue;
                }
                last = Some(print);
                let now = Utc::now();
                let inner = Arc::clone(&task.inner);
                let found = off_ui(async move {
                    let mut out: Vec<(EngineId, Vec<Problem>)> = Vec::new();
                    for job in jobs {
                        let id = job.snapshot.engine.id.clone();
                        let problems =
                            check_engine(job.client.as_deref(), &inner.cache, job.snapshot, now)
                                .await;
                        out.push((id, problems));
                    }
                    out
                })
                .await;
                let Some(found) = found else { break };
                for (engine, problems) in found {
                    tracker.update(&engine, problems);
                }
                tracker.retain_engines(&engines);
                let list = tracker.list();
                if *state.problems.peek() != list {
                    state.problems.set(list);
                }
            }
        });
        service
    }

    /// Ask for a pass. Cheap; call on every change of the signals the rules read.
    pub fn nudge(&self) {
        self.inner.wake.notify_one();
    }

    fn bump(&self) {
        self.inner.version.fetch_add(1, Ordering::Relaxed);
        self.inner.wake.notify_one();
    }

    /// Remember the error lines of a log batch for their containers and ask for a pass.
    pub fn note_errors<'a>(&self, lines: impl Iterator<Item = &'a LogLine>) {
        let mut any = false;
        // The newest lines of the batch: a burst of thousands keeps its end, not its start.
        let all: Vec<&LogLine> = lines.collect();
        let newest = &all[all.len().saturating_sub(2 * KEEP_ERRORS)..];
        {
            let mut map = lock(&self.inner.log_errors);
            for line in newest {
                let text = clip(&line.text);
                if text.is_empty() {
                    continue;
                }
                let entry = map.entry(line.container.clone()).or_default();
                if !entry.lines.contains(&text) {
                    entry.lines.push(text);
                    let skip = entry.lines.len().saturating_sub(KEEP_ERRORS);
                    entry.lines.drain(..skip);
                    any = true;
                }
            }
        }
        if any {
            self.bump();
        }
    }

    /// The error lines to hand the rules for `c`: those seen since its row last changed. A row
    /// that changed (restart, exit) starts again, the log tail read for it covers what came before.
    fn errors_for(&self, c: &Container) -> Vec<String> {
        let signature = Signature::of(c);
        let mut map = lock(&self.inner.log_errors);
        let Some(entry) = map.get_mut(&c.id) else {
            return Vec::new();
        };
        match &entry.signature {
            Some(seen) if *seen != signature => {
                entry.lines.clear();
                entry.signature = Some(signature);
            }
            None => entry.signature = Some(signature),
            Some(_) => {}
        }
        entry.lines.clone()
    }

    fn disk_for(
        &self,
        engine: &EngineId,
        client: Option<&Arc<dyn EngineClient>>,
    ) -> Option<DiskUsage> {
        let mut disks = lock(&self.inner.disks);
        let entry = disks.entry(engine.clone()).or_insert_with(|| DiskEntry {
            at: Instant::now(),
            usage: None,
            asking: false,
        });
        let stale = entry.usage.is_none() || entry.at.elapsed() >= DISK_EVERY;
        if stale && !entry.asking {
            if let Some(client) = client {
                entry.asking = true;
                let client = Arc::clone(client);
                let service = self.clone();
                let engine = engine.clone();
                crate::services::actions::spawn_io(async move {
                    let usage = client.disk_usage().await.ok();
                    {
                        let mut disks = lock(&service.inner.disks);
                        if let Some(entry) = disks.get_mut(&engine) {
                            entry.asking = false;
                            entry.at = Instant::now();
                            if usage.is_some() {
                                entry.usage = usage;
                            }
                        }
                    }
                    service.bump();
                });
            }
        }
        entry.usage.clone()
    }

    /// The jobs for this pass, the ids of every engine, and a fingerprint of everything the
    /// rules will read: equal to the last pass's means the pass can be skipped.
    fn collect(
        &self,
        state: &AppState,
        services: &Services,
        actions: &ActionsService,
    ) -> (Vec<Job>, Vec<EngineId>, u64) {
        let engines = state.engines.peek().clone();
        let containers = state.containers.peek().clone();
        let projects = state.projects.peek().clone();
        let mut hasher = DefaultHasher::new();
        self.inner.version.load(Ordering::Relaxed).hash(&mut hasher);
        let mut jobs = Vec::new();
        for engine in &engines {
            let reachable = engine.state == EngineState::Running;
            // Only an engine that is running (rules over its containers) or stopped or lost
            // (EngineSocketMissing) is judged; one that is starting or connecting is left alone.
            let judged = reachable
                || engine.state == EngineState::Stopped
                || matches!(engine.state, EngineState::Unreachable { .. });
            if !judged {
                continue;
            }
            let client = services.client(&engine.id);
            if reachable && client.is_none() {
                continue;
            }
            let mine: Vec<Container> = if reachable {
                containers
                    .iter()
                    .filter(|c| c.engine == engine.id)
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            let mine_projects: Vec<Project> = if reachable {
                projects
                    .iter()
                    .filter(|p| p.engine == engine.id)
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            let failures: HashMap<ProjectId, Vec<String>> = mine_projects
                .iter()
                .map(|p| (p.id.clone(), actions.compose_failure_lines(&p.id)))
                .filter(|(_, lines)| !lines.is_empty())
                .collect();
            let log_errors: HashMap<ContainerId, Vec<String>> = mine
                .iter()
                .map(|c| (c.id.clone(), self.errors_for(c)))
                .filter(|(_, lines)| !lines.is_empty())
                .collect();
            let disk = if reachable {
                self.disk_for(&engine.id, client.as_ref())
            } else {
                None
            };
            fingerprint(
                &mut hasher,
                engine,
                &mine,
                &mine_projects,
                &failures,
                &log_errors,
                disk.as_ref(),
            );
            jobs.push(Job {
                client,
                snapshot: EngineSnapshot {
                    engine: engine.clone(),
                    containers: mine,
                    projects: mine_projects,
                    failures,
                    log_errors,
                    disk,
                },
            });
        }
        let ids = engines.iter().map(|e| e.id.clone()).collect();
        (jobs, ids, hasher.finish())
    }

    /// What the rules say about the lines a person selected in a log view: the problems of the
    /// containers those lines belong to, with the fixes the rules have for them. Nothing is added
    /// to the list. Empty when no line belongs to a known container or nothing matches.
    pub async fn explain(
        &self,
        world: World,
        services: Option<&Services>,
        lines: Vec<ExplainLine>,
    ) -> Vec<Problem> {
        let World {
            engines,
            containers,
            projects,
        } = world;
        let mut by_container: HashMap<ContainerId, Vec<String>> = HashMap::new();
        for line in &lines {
            let found = containers
                .iter()
                .find(|c| c.name == line.container || c.id.0.starts_with(line.container.as_str()));
            if let Some(c) = found {
                by_container
                    .entry(c.id.clone())
                    .or_default()
                    .push(clip(&line.text));
            }
        }
        let mut jobs = Vec::new();
        for engine in engines {
            let mine: Vec<Container> = containers
                .iter()
                .filter(|c| c.engine == engine.id && by_container.contains_key(&c.id))
                .cloned()
                .collect();
            if mine.is_empty() {
                continue;
            }
            let wanted: HashSet<&ProjectId> =
                mine.iter().filter_map(|c| c.project.as_ref()).collect();
            let mine_projects: Vec<Project> = projects
                .iter()
                .filter(|p| p.engine == engine.id && wanted.contains(&p.id))
                .cloned()
                .collect();
            jobs.push(Job {
                client: services.and_then(|s| s.client(&engine.id)),
                snapshot: EngineSnapshot {
                    engine,
                    containers: mine,
                    projects: mine_projects,
                    failures: HashMap::new(),
                    log_errors: HashMap::new(),
                    disk: None,
                },
            });
        }
        let now = Utc::now();
        let wanted: HashSet<ContainerId> = by_container.keys().cloned().collect();
        // A cache of its own: the selected lines replace what the log tail says.
        let found = off_ui(async move {
            let mut out: Vec<Problem> = Vec::new();
            for job in jobs {
                let cache: Cache = Mutex::new(HashMap::new());
                let mut snapshot = job.snapshot;
                snapshot.log_errors = by_container
                    .iter()
                    .map(|(id, lines)| (id.clone(), lines.clone()))
                    .collect();
                out.extend(check_engine(job.client.as_deref(), &cache, snapshot, now).await);
            }
            out
        })
        .await
        .unwrap_or_default();
        found
            .into_iter()
            .filter(|p| match &p.subject {
                Subject::Container(id) => wanted.contains(id),
                _ => false,
            })
            .collect()
    }
}

/// Everything `check_engine` will read from the signals, reduced to a number: a pass whose
/// fingerprint equals the last one's is skipped.
fn fingerprint(
    hasher: &mut DefaultHasher,
    engine: &Engine,
    containers: &[Container],
    projects: &[Project],
    failures: &HashMap<ProjectId, Vec<String>>,
    log_errors: &HashMap<ContainerId, Vec<String>>,
    disk: Option<&DiskUsage>,
) {
    engine.id.hash(hasher);
    format!("{:?}", engine.state).hash(hasher);
    engine.docker_host.hash(hasher);
    for c in containers {
        c.id.hash(hasher);
        c.name.hash(hasher);
        format!("{:?}", (c.state, c.health, c.exit_code)).hash(hasher);
        c.restart_count.hash(hasher);
        c.started.hash(hasher);
        c.image.reference.hash(hasher);
        for port in &c.ports {
            (port.host_port, port.container_port).hash(hasher);
        }
        c.project.hash(hasher);
    }
    for p in projects {
        p.id.hash(hasher);
        p.folder.hash(hasher);
        p.compose_files.hash(hasher);
        format!("{:?}", p.status).hash(hasher);
    }
    let mut failed: Vec<(&ProjectId, &Vec<String>)> = failures.iter().collect();
    failed.sort();
    failed.hash(hasher);
    let mut errors: Vec<(&ContainerId, &Vec<String>)> = log_errors.iter().collect();
    errors.sort();
    errors.hash(hasher);
    disk.map(|d| format!("{d:?}")).hash(hasher);
}

#[cfg(test)]
#[path = "problems_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "problems_live_tests.rs"]
mod live_tests;
