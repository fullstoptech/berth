//! Pending generation plans (PLAN 6.11 `plans.rs`; ADR 0012, D31, D70).
//!
//! A plan is built by a recipe, a template, a rule fix or (P7) a harness, then parked here by uuid.
//! `Overlay::ReviewChanges(uuid)` shows it and is the only caller of [`write_plan`], which is the
//! only caller of `berth_rules::write::apply`: nothing reaches a project's files without the click.
//!
//! A plan holds real generated passwords (the new `.env` text). It is never logged: the types here
//! do not derive `Debug` on the plan and nothing in this file puts one in a tracing field.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use berth_core::{ComposeCommand, EngineId, GenerationPlan, ProjectId};
use berth_rules::RulesError;
use uuid::Uuid;

/// Builds the plan again from what is on disk now (the Regenerate button of the conflict state).
pub type Regenerate = Arc<dyn Fn() -> Result<GenerationPlan, RulesError> + Send + Sync>;

/// What `after_write` runs against: the project the plan's folder is, or is about to become.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanRun {
    pub engine: EngineId,
    /// The folder as the engine will see it (canonical), also the compose project directory.
    pub folder: PathBuf,
    /// Compose files, relative to `folder`.
    pub files: Vec<PathBuf>,
    pub project: ProjectId,
    /// The project's name in the Projects list.
    pub name: String,
    /// The name Compose knows the project by (`-p`): the folder's name for a new folder, the
    /// project's own name for one that exists.
    pub compose_name: String,
}

impl PlanRun {
    /// The run for `folder` on `engine`: the folder is made canonical so the id matches the project
    /// the engine's containers form (`ProjectId::for_folder` of the working directory).
    #[must_use]
    pub fn for_folder(engine: &EngineId, folder: &Path, files: Vec<PathBuf>) -> Self {
        let folder = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
        let name = folder
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        Self {
            engine: engine.clone(),
            project: ProjectId::for_folder(&folder, engine),
            compose_name: crate::services::compose::compose_project_name(&folder),
            folder,
            files,
            name,
        }
    }

    /// The run for a project that exists: its folder, compose files, id and name. `None` for a
    /// project with no folder on this Mac or no known compose file.
    #[must_use]
    pub fn for_project(project: &berth_core::Project) -> Option<Self> {
        let folder = project.folder.clone()?;
        if project.compose_files.is_empty() {
            return None;
        }
        Some(Self {
            engine: project.engine.clone(),
            folder,
            files: project.compose_files.clone(),
            project: project.id.clone(),
            name: project.name.clone(),
            compose_name: project.name.clone(),
        })
    }
}

/// One plan waiting for a decision.
#[derive(Clone)]
pub struct PendingPlan {
    pub plan: GenerationPlan,
    /// The name in the sheet's title ("new-app"); a harness plan is titled by the harness instead.
    pub title: String,
    /// Where `after_write` runs. `None`: Write is the only button.
    pub run: Option<PlanRun>,
    /// Pin the folder on this engine and add it to Recent once the files are written (a folder that
    /// has no project yet).
    pub pin: Option<EngineId>,
    pub regenerate: Option<Regenerate>,
    /// The line left of the buttons ("Volume shop-api_pgdata (1.2 GB) is kept.").
    pub footer_note: Option<String>,
}

impl PendingPlan {
    #[must_use]
    pub fn new(plan: GenerationPlan, title: impl Into<String>) -> Self {
        Self {
            plan,
            title: title.into(),
            run: None,
            pin: None,
            regenerate: None,
            footer_note: None,
        }
    }
}

/// The plan holds passwords, so `Debug` says how many files it has and nothing else.
impl std::fmt::Debug for PendingPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingPlan")
            .field("files", &self.plan.files.len())
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

/// The pending plans, shared by every screen that proposes changes. Cheap to clone.
#[derive(Clone, Default)]
pub struct PlanStore {
    inner: Arc<Mutex<BTreeMap<Uuid, PendingPlan>>>,
}

impl PlanStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<Uuid, PendingPlan>> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Park a plan; the id goes in `Overlay::ReviewChanges`.
    pub fn insert(&self, plan: PendingPlan) -> Uuid {
        let id = Uuid::now_v7();
        self.lock().insert(id, plan);
        id
    }

    #[must_use]
    pub fn get(&self, id: &Uuid) -> Option<PendingPlan> {
        self.lock().get(id).cloned()
    }

    /// Put a regenerated plan where the old one was, so the open sheet shows it.
    pub fn replace(&self, id: &Uuid, plan: GenerationPlan) {
        if let Some(entry) = self.lock().get_mut(id) {
            entry.plan = plan;
        }
    }

    pub fn remove(&self, id: &Uuid) {
        self.lock().remove(id);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

/// Provide the store as context (once, from the providers registry).
pub fn provide_plan_store() -> PlanStore {
    dioxus::prelude::use_context_provider(PlanStore::default)
}

// ---------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------

/// Why a write did not happen. Both leave every file as it was (`write::apply` writes nothing when
/// a check fails, and undoes its renames when one fails midway).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteFailure {
    /// A file changed since the plan was made (or a file to create appeared): Regenerate.
    Conflict(PathBuf),
    /// Anything else, in a sentence.
    Failed(String),
}

impl WriteFailure {
    fn of(err: RulesError) -> Self {
        match err {
            RulesError::Conflict(path) => Self::Conflict(path),
            other => Self::Failed(other.to_string()),
        }
    }
}

/// Run `future` on the tokio runtime and wait for it from a UI task: the I/O of a click (writing,
/// regenerating, `compose config`) never runs on the UI path (D23). `None` when the task was
/// dropped (the app is quitting).
pub async fn off_ui<T: Send + 'static>(
    future: impl std::future::Future<Output = T> + Send + 'static,
) -> Option<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::services::actions::spawn_io(async move {
        let _ = tx.send(future.await);
    });
    rx.await.ok()
}

/// [`off_ui`] for a blocking closure (reading a folder, building a plan).
pub async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    off_ui(async move { tokio::task::spawn_blocking(work).await.ok() })
        .await
        .flatten()
}

/// The engine a plan for a new folder runs on: the one the engine switcher shows, else the first
/// that is running.
#[must_use]
pub fn target_engine(
    scope: &crate::state::EngineScope,
    engines: &[berth_core::Engine],
) -> Option<EngineId> {
    match scope {
        crate::state::EngineScope::One(id) => Some(id.clone()),
        crate::state::EngineScope::All => engines
            .iter()
            .find(|e| e.state == berth_core::EngineState::Running)
            .map(|e| e.id.clone()),
    }
}

/// Write the plan's files: the one call of `berth_rules::write::apply` in the app. Runs on a
/// blocking thread; the caller has just been clicked.
///
/// # Errors
/// [`WriteFailure`], with nothing written.
pub async fn write_plan(plan: GenerationPlan) -> Result<Vec<PathBuf>, WriteFailure> {
    let done = tokio::task::spawn_blocking(move || berth_rules::write::apply(&plan))
        .await
        .map_err(|e| WriteFailure::Failed(format!("the write did not finish: {e}")))?;
    done.map(|report| report.written).map_err(WriteFailure::of)
}

/// What a click on Write did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Committed {
    /// The files are on disk; a folder that has no project yet is now pinned and in Recent.
    Written { count: usize },
    /// A file changed since the plan was made: nothing was written.
    Conflict(PathBuf),
    /// Anything else: nothing was written.
    Failed(String),
}

/// Write a plan the user approved and remember its folder. The only path from a plan to the disk.
pub async fn commit(pending: PendingPlan, store: Option<Arc<berth_store::Store>>) -> Committed {
    let count = pending.plan.files.len();
    match write_plan(pending.plan.clone()).await {
        Ok(_) => {
            if let (Some(store), Some(run), Some(engine)) = (&store, &pending.run, &pending.pin) {
                remember_folder(store, run, engine);
            }
            Committed::Written { count }
        }
        Err(WriteFailure::Conflict(path)) => Committed::Conflict(path),
        Err(WriteFailure::Failed(message)) => Committed::Failed(message),
    }
}

/// Build the plan again, off the UI thread.
///
/// # Errors
/// The rules' error as a sentence.
pub async fn regenerate(make: Regenerate) -> Result<GenerationPlan, String> {
    tokio::task::spawn_blocking(move || make())
        .await
        .map_err(|e| format!("the plan did not finish: {e}"))?
        .map_err(|e| e.to_string())
}

/// After a successful write of a folder's first files: pin it and put it in Recent (D36: a folder
/// is known because the user opened it).
pub fn remember_folder(store: &berth_store::Store, run: &PlanRun, engine: &EngineId) {
    let pin = berth_core::PinnedFolder {
        folder: run.folder.clone(),
        engine: engine.clone(),
        added: chrono::Utc::now(),
    };
    if let Err(err) = store.pin(&pin) {
        tracing::warn!(%err, "pinning the folder failed");
    }
    if let Err(err) = store.push_recent(&run.folder) {
        tracing::warn!(%err, "adding the folder to Recent failed");
    }
}

// ---------------------------------------------------------------------
// Checking the compose text before Write is enabled
// ---------------------------------------------------------------------

/// The compose file a plan writes, with its new text: the first of Compose's four names.
#[must_use]
pub fn compose_file_of(plan: &GenerationPlan) -> Option<(&Path, &str)> {
    const NAMES: [&str; 4] = [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yml",
        "docker-compose.yaml",
    ];
    plan.files.iter().find_map(|f| {
        let name = f.path.file_name()?.to_str()?;
        let text = f.new_text.as_deref()?;
        (f.path.components().count() == 1 && NAMES.contains(&name))
            .then_some((f.path.as_path(), text))
    })
}

/// The values a new `.env` would give, for `ComposeRunner::validate_text` (it passes them to the
/// child as environment so `${POSTGRES_PASSWORD:?}` resolves before the file exists). Empty when
/// the plan does not write `.env`: the folder's own file is read then.
#[must_use]
pub fn env_for_validation(plan: &GenerationPlan) -> Vec<(String, String)> {
    plan.files
        .iter()
        .find(|f| f.path == Path::new(".env"))
        .and_then(|f| f.new_text.as_deref())
        .map(parse_env)
        .unwrap_or_default()
}

/// `KEY=value` lines of a `.env` text. Quotes around a value are dropped; comments and blank
/// lines are skipped. Only for handing the values to a child process.
fn parse_env(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

/// The action id a project run is recorded under (the pending key and the activity log).
#[must_use]
pub fn action_id_for(command: &ComposeCommand) -> &'static str {
    match command {
        ComposeCommand::Up { .. } | ComposeCommand::Start { .. } => "project.start",
        ComposeCommand::Restart { .. } => "project.restart",
        ComposeCommand::Build { .. } => "project.rebuild",
        ComposeCommand::Pull { .. } => "project.pull_images",
        ComposeCommand::Stop { .. } => "project.stop",
        ComposeCommand::Down { .. } => "project.delete",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use berth_core::{FileChange, FileDiff, PlanSource};

    fn file(path: &str, text: &str) -> FileDiff {
        FileDiff {
            path: PathBuf::from(path),
            change: FileChange::Create,
            old_sha256: None,
            new_text: Some(text.to_string()),
            hunks: Vec::new(),
            added: 1,
            removed: 0,
        }
    }

    fn plan(files: Vec<FileDiff>) -> GenerationPlan {
        GenerationPlan {
            root: PathBuf::from("/x/new-app"),
            files,
            source: PlanSource::Editor,
            notes: Vec::new(),
            after_write: None,
        }
    }

    #[test]
    fn review_changes_store_parks_plans_by_uuid_and_replaces_in_place() {
        let store = PlanStore::default();
        let a = store.insert(PendingPlan::new(plan(vec![file("a", "1")]), "a"));
        let b = store.insert(PendingPlan::new(plan(Vec::new()), "b"));
        assert_ne!(a, b);
        assert_eq!(store.len(), 2);
        assert_eq!(store.get(&a).map(|p| p.plan.files.len()), Some(1));
        store.replace(&a, plan(vec![file("a", "1"), file("b", "2")]));
        assert_eq!(store.get(&a).map(|p| p.plan.files.len()), Some(2));
        assert_eq!(store.get(&a).map(|p| p.title), Some("a".to_string()));
        store.remove(&a);
        assert!(store.get(&a).is_none());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn review_changes_pending_plan_debug_never_prints_the_plan() {
        let secret = "s3cr3t-generated-pw";
        let pending = PendingPlan::new(
            plan(vec![file(".env", &format!("POSTGRES_PASSWORD={secret}\n"))]),
            "new-app",
        );
        let text = format!("{pending:?}");
        assert!(!text.contains(secret), "{text}");
        assert!(text.contains("new-app"), "{text}");
    }

    #[test]
    fn review_changes_compose_file_of_finds_the_compose_text_only_at_the_root() {
        let p = plan(vec![
            file("sub/compose.yaml", "x"),
            file("compose.yaml", "y"),
        ]);
        assert_eq!(compose_file_of(&p), Some((Path::new("compose.yaml"), "y")));
        assert_eq!(compose_file_of(&plan(vec![file(".env", "A=1")])), None);
    }

    #[test]
    fn review_changes_env_for_validation_reads_the_plans_env_and_ignores_the_rest() {
        let p = plan(vec![
            file("compose.yaml", "services: {}"),
            file(
                ".env",
                "# comment\nA=1\nexport B=\"two words\"\nC='q'\n\nnovalue\nD=a=b\n",
            ),
        ]);
        assert_eq!(
            env_for_validation(&p),
            [
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "two words".to_string()),
                ("C".to_string(), "q".to_string()),
                ("D".to_string(), "a=b".to_string()),
            ]
        );
        assert!(env_for_validation(&plan(vec![file("compose.yaml", "x")])).is_empty());
    }

    #[test]
    fn review_changes_run_ids_match_the_project_the_pin_becomes() {
        let engine = EngineId("colima:default".to_string());
        let dir = tempfile::tempdir().expect("dir");
        let run = PlanRun::for_folder(&engine, dir.path(), vec![PathBuf::from("compose.yaml")]);
        let canonical = std::fs::canonicalize(dir.path()).expect("canonical");
        assert_eq!(run.folder, canonical);
        assert_eq!(run.project, ProjectId::for_folder(&canonical, &engine));
        assert_eq!(
            run.name,
            canonical
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
    }

    #[test]
    fn review_changes_commands_map_to_the_project_actions_the_row_shows() {
        let up = ComposeCommand::Up {
            services: Vec::new(),
            build: true,
        };
        assert_eq!(action_id_for(&up), "project.start");
        assert_eq!(
            action_id_for(&ComposeCommand::Restart {
                services: vec!["db".to_string()]
            }),
            "project.restart"
        );
    }

    #[tokio::test]
    async fn review_changes_write_plan_writes_and_reports_a_conflict_without_writing() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join(".env"), "A=1\n").expect("env");
        let env = berth_rules::diff::file_diff(Path::new(".env"), Some("A=1\n"), Some("A=2\n"));
        let compose =
            berth_rules::diff::file_diff(Path::new("compose.yaml"), None, Some("services: {}\n"));
        let mut p = plan(vec![compose, env]);
        p.root = dir.path().to_path_buf();
        // Someone edits .env after the plan was made.
        std::fs::write(dir.path().join(".env"), "A=changed\n").expect("edit");
        let failure = write_plan(p.clone()).await.expect_err("conflict");
        assert_eq!(failure, WriteFailure::Conflict(PathBuf::from(".env")));
        assert!(
            !dir.path().join("compose.yaml").exists(),
            "a conflict writes nothing"
        );
        // Undo the edit: the same plan now writes both files.
        std::fs::write(dir.path().join(".env"), "A=1\n").expect("restore");
        let written = write_plan(p).await.expect("written");
        assert_eq!(written.len(), 2);
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".env")).expect("env"),
            "A=2\n"
        );
        assert!(dir.path().join("compose.yaml").exists());
    }
}
