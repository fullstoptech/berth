//! Project from Git (P8-T5, D49, D72): clone a repository into a folder, hand the folder to the
//! Detected flow (P5), keep its Git source, look for new commits and pull and redeploy.
//!
//! The logic is [`GitDeployService`]: async functions over a [`GitOps`] (the `git` CLI in the app,
//! a fake in tests) and the store. It knows nothing of signals; [`GitDeployHandle`] is the thin
//! layer that runs it off the UI path and writes [`GitDeployView`] for the screens (D23).
//!
//! Nothing stored here is a secret: an address with a password in it is refused, and an ssh key is
//! only a path that lives in the repository's own git config. Pulling never merges (`--ff-only`)
//! and a poll redeploys only for a project whose Auto-redeploy switch is on (D72).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use berth_core::{ComposeCommand, EngineId, ProjectSource};
use berth_engine::git::{CloneProgress, IncomingCommit};
use berth_engine::EngineError;
use berth_store::Store;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use crate::services::actions::ActionsService;
use crate::services::plans::PlanRun;
use crate::state::{AppState, Overlay};

/// How often the polling task looks at which projects are due.
pub const TICK: Duration = Duration::from_secs(30);

/// The polling intervals the project's Git panel offers; `None` is off (D72).
pub const POLL_CHOICES: [(Option<u64>, &str); 4] = [
    (None, "Off"),
    (Some(5), "Every 5 minutes"),
    (Some(15), "Every 15 minutes"),
    (Some(60), "Every hour"),
];

// ---------------------------------------------------------------------------------------------
// The address and the clone request

/// Read what was typed in the Repository field as a URL git can clone.
///
/// Accepted: `https://`, `http://`, `ssh://`, `git://`, `file://`, `user@host:path` (written as
/// `ssh://user@host/path`) and an absolute folder path (`file://`). An address that carries a
/// password is refused: Berth stores the address and never a secret.
///
/// # Errors
/// A sentence for the field.
pub fn parse_repo_url(text: &str) -> Result<url::Url, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Give the repository address".to_string());
    }
    let parsed = if text.contains("://") {
        url::Url::parse(text).map_err(|_| format!("{text} is not a repository address"))?
    } else if let Some((user_host, path)) = scp_like(text) {
        url::Url::parse(&format!("ssh://{user_host}/{path}"))
            .map_err(|_| format!("{text} is not a repository address"))?
    } else if Path::new(text).is_absolute() {
        url::Url::from_file_path(text)
            .map_err(|()| format!("{text} is not a repository address"))?
    } else {
        return Err(
            "Use an https:// or ssh:// address, git@host:owner/repo.git, or a full folder path"
                .to_string(),
        );
    };
    if !matches!(parsed.scheme(), "https" | "http" | "ssh" | "git" | "file") {
        return Err(format!(
            "{}:// is not a repository address",
            parsed.scheme()
        ));
    }
    if parsed.password().is_some() {
        return Err(
            "Take the password out of the address. Use an SSH key or a git credential helper instead."
                .to_string(),
        );
    }
    if parsed.scheme() != "file" && parsed.host_str().is_none_or(str::is_empty) {
        return Err("The address needs a host".to_string());
    }
    Ok(parsed)
}

/// `git@github.com:owner/repo.git` as `("git@github.com", "owner/repo.git")`.
fn scp_like(text: &str) -> Option<(&str, &str)> {
    let (user_host, path) = text.split_once(':')?;
    if user_host.is_empty()
        || path.is_empty()
        || user_host.contains('/')
        || user_host.contains(char::is_whitespace)
    {
        return None;
    }
    Some((user_host, path.trim_start_matches('/')))
}

/// The folder name a clone of `url` gets: the last path segment without `.git`.
#[must_use]
pub fn repo_folder_name(url: &url::Url) -> String {
    let segment = url
        .path_segments()
        .and_then(|mut parts| parts.rfind(|part| !part.is_empty()))
        .unwrap_or_default();
    let name = segment.strip_suffix(".git").unwrap_or(segment);
    if name.is_empty() {
        "repository".to_string()
    } else {
        name.to_string()
    }
}

/// Everything the From Git sheet asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneRequest {
    pub url: url::Url,
    /// `None` clones the repository's default branch.
    pub branch: Option<String>,
    /// A private key from `~/.ssh`, used through the agent. `None`: whatever the agent offers.
    pub ssh_key: Option<PathBuf>,
    /// The folder the clone goes inside; the repository gets its own folder in it.
    pub parent: PathBuf,
    pub engine: EngineId,
}

impl CloneRequest {
    /// The folder git creates.
    #[must_use]
    pub fn target(&self) -> PathBuf {
        self.parent.join(repo_folder_name(&self.url))
    }
}

/// Why Clone is off, or `None`.
#[must_use]
pub fn clone_blocker(
    url_text: &str,
    parent: Option<&Path>,
    engine: Option<&EngineId>,
) -> Option<String> {
    if let Err(message) = parse_repo_url(url_text) {
        return Some(message);
    }
    let Some(parent) = parent else {
        return Some("Choose the folder to clone into.".to_string());
    };
    if engine.is_none() {
        return Some("No engine is running. Start one first.".to_string());
    }
    let target = parent.join(repo_folder_name(&parse_repo_url(url_text).ok()?));
    if target.exists() {
        return Some(format!(
            "{} already exists. Choose another folder.",
            target.display()
        ));
    }
    None
}

/// The private keys in `ssh_dir`: files with a `.pub` beside them, by name. Berth never reads a
/// key; the agent does the signing.
#[must_use]
pub fn list_ssh_keys(ssh_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(ssh_dir) else {
        return Vec::new();
    };
    let mut keys: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().is_none_or(|ext| ext != "pub")
                && path
                    .with_file_name(format!(
                        "{}.pub",
                        path.file_name()
                            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
                    ))
                    .is_file()
        })
        .collect();
    keys.sort();
    keys
}

// ---------------------------------------------------------------------------------------------
// Failures

/// What went wrong with a clone, in the words the sheet needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The server refused who we are.
    Auth,
    /// The address or branch is wrong, or the repository is not there for us.
    Url,
    /// ssh does not know the host yet.
    HostKey,
    /// Anything else (no git, no network, disk).
    Other,
}

/// A failed clone: git's own last words and the fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneFailure {
    pub kind: FailureKind,
    pub message: String,
    pub fix: String,
}

/// Sort git's error text into a [`FailureKind`] with the fix for `url`.
#[must_use]
pub fn classify_clone_failure(stderr: &str, url: &url::Url) -> CloneFailure {
    let lower = stderr.to_lowercase();
    let message = stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("git clone failed")
        .to_string();
    let ssh = url.scheme() == "ssh";
    let (kind, fix) = if lower.contains("host key verification failed") {
        (
            FailureKind::HostKey,
            format!(
                "ssh does not know {} yet. Run ssh -T {} once in Terminal to trust it, then try again.",
                url.host_str().unwrap_or("the host"),
                url.host_str().unwrap_or("host")
            ),
        )
    } else if lower.contains("permission denied")
        || lower.contains("authentication failed")
        || lower.contains("could not read username")
        || lower.contains("could not read password")
        || lower.contains("terminal prompts disabled")
        || lower.contains("publickey")
    {
        (
            FailureKind::Auth,
            if ssh {
                "The server did not accept your key. Run ssh-add for it so your agent offers it, or choose the key in the list.".to_string()
            } else {
                "The server asked for a login. Use the ssh:// address with a key, or set a git credential helper (git config --global credential.helper) and try again.".to_string()
            },
        )
    } else if lower.contains("repository not found")
        || lower.contains("does not appear to be a git repository")
        || lower.contains("does not exist")
        || lower.contains("could not resolve host")
        || lower.contains("not found")
        || lower.contains("remote branch")
        || lower.contains("unable to access")
    {
        (
            FailureKind::Url,
            "Check the address and the branch name. For a private repository, make sure your key or login can see it.".to_string(),
        )
    } else {
        (
            FailureKind::Other,
            "Check that git works in Terminal (git --version) and that there is room on the disk, then try again.".to_string(),
        )
    };
    CloneFailure { kind, message, fix }
}

/// "1 new commit", "3 new commits".
#[must_use]
pub fn new_commits_label(count: usize) -> String {
    if count == 1 {
        "1 new commit".to_string()
    } else {
        format!("{count} new commits")
    }
}

// ---------------------------------------------------------------------------------------------
// Git operations

/// The `git` calls the service makes. [`CliGit`] is the real one.
#[async_trait::async_trait]
pub trait GitOps: Send + Sync {
    /// Clone; `on_progress` hears each phase git reports.
    async fn clone_repo(
        &self,
        request: &CloneRequest,
        target: &Path,
        cancel: CancellationToken,
        on_progress: &(dyn Fn(CloneProgress) + Send + Sync),
    ) -> Result<(), EngineError>;
    async fn fetch(&self, dir: &Path, cancel: CancellationToken) -> Result<(), EngineError>;
    async fn incoming(
        &self,
        dir: &Path,
        cancel: CancellationToken,
    ) -> Result<Vec<IncomingCommit>, EngineError>;
    async fn pull_ff_only(&self, dir: &Path, cancel: CancellationToken) -> Result<(), EngineError>;
    async fn current_commit(
        &self,
        dir: &Path,
        cancel: CancellationToken,
    ) -> Result<Option<String>, EngineError>;
    /// The branch HEAD is on, when it is on one.
    async fn branch(
        &self,
        dir: &Path,
        cancel: CancellationToken,
    ) -> Result<Option<String>, EngineError>;
}

/// The `git` binary, found through the shared login `PATH` (D30).
#[derive(Debug, Clone, Copy, Default)]
pub struct CliGit;

impl CliGit {
    async fn cli() -> berth_engine::GitCli {
        berth_engine::GitCli::new(crate::services::login_path::search_path().await)
    }
}

#[async_trait::async_trait]
impl GitOps for CliGit {
    async fn clone_repo(
        &self,
        request: &CloneRequest,
        target: &Path,
        cancel: CancellationToken,
        on_progress: &(dyn Fn(CloneProgress) + Send + Sync),
    ) -> Result<(), EngineError> {
        Self::cli()
            .await
            .clone_repo(
                request.url.as_str(),
                target,
                request.branch.as_deref(),
                request.ssh_key.as_deref(),
                cancel,
                on_progress,
            )
            .await
    }

    async fn fetch(&self, dir: &Path, cancel: CancellationToken) -> Result<(), EngineError> {
        Self::cli().await.fetch(dir, cancel).await
    }

    async fn incoming(
        &self,
        dir: &Path,
        cancel: CancellationToken,
    ) -> Result<Vec<IncomingCommit>, EngineError> {
        Self::cli().await.incoming(dir, cancel).await
    }

    async fn pull_ff_only(&self, dir: &Path, cancel: CancellationToken) -> Result<(), EngineError> {
        Self::cli().await.pull_ff_only(dir, cancel).await
    }

    async fn current_commit(
        &self,
        dir: &Path,
        cancel: CancellationToken,
    ) -> Result<Option<String>, EngineError> {
        Self::cli().await.current_commit(dir, cancel).await
    }

    async fn branch(
        &self,
        dir: &Path,
        cancel: CancellationToken,
    ) -> Result<Option<String>, EngineError> {
        Ok(Self::cli().await.status(dir, cancel).await?.branch)
    }
}

// ---------------------------------------------------------------------------------------------
// The stored source

/// One folder cloned from Git, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitProject {
    pub folder: PathBuf,
    pub engine: EngineId,
    pub repo: url::Url,
    pub branch: String,
    /// The short hash last deployed (the project row's commit).
    pub commit: Option<String>,
    /// Look for new commits this often; `None` is off.
    pub poll: Option<Duration>,
    pub auto_redeploy: bool,
}

impl GitProject {
    fn from_row(folder: PathBuf, engine: EngineId, source: ProjectSource) -> Option<Self> {
        match source {
            ProjectSource::Git {
                repo,
                branch,
                last_commit,
                poll,
                auto_redeploy,
            } => Some(Self {
                folder,
                engine,
                repo,
                branch,
                commit: last_commit,
                poll,
                auto_redeploy,
            }),
            _ => None,
        }
    }

    fn source(&self) -> ProjectSource {
        ProjectSource::Git {
            repo: self.repo.clone(),
            branch: self.branch.clone(),
            last_commit: self.commit.clone(),
            poll: self.poll,
            auto_redeploy: self.auto_redeploy,
        }
    }
}

/// What a pull did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pulled {
    pub from: Option<String>,
    pub to: Option<String>,
    pub commits: Vec<IncomingCommit>,
}

/// What one poll found and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollOutcome {
    pub incoming: Vec<IncomingCommit>,
    /// Set when Auto-redeploy was on and the new commits were pulled: the caller redeploys.
    pub pulled: Option<Pulled>,
}

/// Is a project with this interval due for a look at `now`? Never checked counts as due.
#[must_use]
pub fn poll_due(
    poll: Option<Duration>,
    last_checked: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    let Some(every) = poll else {
        return false;
    };
    let Some(last) = last_checked else {
        return true;
    };
    chrono::Duration::from_std(every).is_ok_and(|every| now - last >= every)
}

fn git_message(err: &EngineError) -> String {
    match err {
        EngineError::Docker(message) => message.clone(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// The service

/// The Git deploy logic. Cheap to clone.
#[derive(Clone)]
pub struct GitDeployService {
    store: Arc<Store>,
    git: Arc<dyn GitOps>,
}

impl std::fmt::Debug for GitDeployService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitDeployService").finish_non_exhaustive()
    }
}

impl GitDeployService {
    #[must_use]
    pub fn new(store: Arc<Store>, git: Arc<dyn GitOps>) -> Self {
        Self { store, git }
    }

    /// The service over the real `git`.
    #[must_use]
    pub fn real(store: Arc<Store>) -> Self {
        Self::new(store, Arc::new(CliGit))
    }

    /// Every stored Git project, by folder.
    #[must_use]
    pub fn projects(&self) -> Vec<GitProject> {
        self.store
            .git_sources()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(folder, engine, source)| GitProject::from_row(folder, engine, source))
            .collect()
    }

    fn project(&self, folder: &Path) -> Result<GitProject, String> {
        self.projects()
            .into_iter()
            .find(|p| p.folder == folder)
            .ok_or_else(|| "Berth did not clone this folder from Git".to_string())
    }

    fn save(&self, project: &GitProject) -> Result<(), String> {
        self.store
            .set_git_source(&project.folder, &project.engine, &project.source())
            .map_err(|e| e.to_string())
    }

    /// Clone, then remember the source (polling off, Auto-redeploy off) and pin the folder on the
    /// request's engine.
    ///
    /// # Errors
    /// The classified failure; a folder git half-made is gone.
    pub async fn clone_repo(
        &self,
        request: &CloneRequest,
        cancel: CancellationToken,
        on_progress: &(dyn Fn(CloneProgress) + Send + Sync),
    ) -> Result<GitProject, CloneFailure> {
        let fail = |kind: FailureKind, message: String, fix: &str| CloneFailure {
            kind,
            message,
            fix: fix.to_string(),
        };
        let target = request.target();
        if let Err(err) = self
            .git
            .clone_repo(request, &target, cancel.clone(), on_progress)
            .await
        {
            return Err(classify_clone_failure(&git_message(&err), &request.url));
        }
        let folder = std::fs::canonicalize(&target).unwrap_or(target);
        let commit = self
            .git
            .current_commit(&folder, cancel.clone())
            .await
            .ok()
            .flatten();
        let branch = match &request.branch {
            Some(branch) => branch.clone(),
            None => self
                .git
                .branch(&folder, cancel)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| "main".to_string()),
        };
        let project = GitProject {
            folder: folder.clone(),
            engine: request.engine.clone(),
            repo: request.url.clone(),
            branch,
            commit,
            poll: None,
            auto_redeploy: false,
        };
        self.save(&project).map_err(|message| {
            fail(
                FailureKind::Other,
                message,
                "Berth cloned the folder but could not remember it. Open the folder from Open folder.",
            )
        })?;
        let now = Utc::now();
        let pins_ok = self.store.pin(&berth_core::PinnedFolder {
            folder: folder.clone(),
            engine: request.engine.clone(),
            added: now,
        });
        if let Err(err) = pins_ok.and_then(|()| self.store.push_recent(&folder)) {
            tracing::warn!(%err, "pinning the cloned folder failed");
        }
        Ok(project)
    }

    /// Fetch, then list what the upstream has that HEAD lacks (`git log --oneline HEAD..@{u}`).
    ///
    /// # Errors
    /// Git's message.
    pub async fn check(
        &self,
        folder: &Path,
        cancel: CancellationToken,
    ) -> Result<Vec<IncomingCommit>, String> {
        self.git
            .fetch(folder, cancel.clone())
            .await
            .map_err(|e| git_message(&e))?;
        self.git
            .incoming(folder, cancel)
            .await
            .map_err(|e| git_message(&e))
    }

    /// Pull the incoming commits (fast-forward only) and remember the new commit. The caller
    /// redeploys after (`Up { build: true }`); a pull that git refuses leaves the folder as it was
    /// and the stored commit alone, so nothing is redeployed.
    ///
    /// # Errors
    /// Git's message.
    pub async fn pull(&self, folder: &Path, cancel: CancellationToken) -> Result<Pulled, String> {
        let mut project = self.project(folder)?;
        let commits = self
            .git
            .incoming(folder, cancel.clone())
            .await
            .map_err(|e| git_message(&e))?;
        self.git
            .pull_ff_only(folder, cancel.clone())
            .await
            .map_err(|e| git_message(&e))?;
        let to = self
            .git
            .current_commit(folder, cancel)
            .await
            .map_err(|e| git_message(&e))?;
        let from = project.commit.clone();
        project.commit.clone_from(&to);
        self.save(&project)?;
        Ok(Pulled { from, to, commits })
    }

    /// One look at a project: fetch and list. With Auto-redeploy on and new commits, pull too
    /// (`pulled` is set and the caller redeploys); with it off the commits are only reported and
    /// nothing changes (D72).
    ///
    /// # Errors
    /// Git's message.
    pub async fn poll(
        &self,
        folder: &Path,
        cancel: CancellationToken,
    ) -> Result<PollOutcome, String> {
        let project = self.project(folder)?;
        let incoming = self.check(folder, cancel.clone()).await?;
        if incoming.is_empty() || !project.auto_redeploy {
            return Ok(PollOutcome {
                incoming,
                pulled: None,
            });
        }
        let pulled = self.pull(folder, cancel).await?;
        Ok(PollOutcome {
            incoming: Vec::new(),
            pulled: Some(pulled),
        })
    }

    /// Turn Auto-redeploy on or off for a project.
    ///
    /// # Errors
    /// A sentence when the project is not a Git project or the store fails.
    pub fn set_auto_redeploy(&self, folder: &Path, on: bool) -> Result<GitProject, String> {
        let mut project = self.project(folder)?;
        project.auto_redeploy = on;
        self.save(&project)?;
        Ok(project)
    }

    /// Set the polling interval; `None` turns polling off.
    ///
    /// # Errors
    /// A sentence when the project is not a Git project or the store fails.
    pub fn set_poll(&self, folder: &Path, poll: Option<Duration>) -> Result<GitProject, String> {
        let mut project = self.project(folder)?;
        project.poll = poll;
        self.save(&project)?;
        Ok(project)
    }
}

/// Does the cloned folder's Compose file load? A folder with no Compose file passes (the generate
/// flow takes it from there); one that has files must satisfy `validate` (`docker compose config`).
///
/// # Errors
/// Compose's own words.
pub async fn judge_compose<F, Fut>(
    folder: &Path,
    files: Vec<PathBuf>,
    validate: F,
) -> Result<(), String>
where
    F: FnOnce(PathBuf, Vec<PathBuf>) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    if files.is_empty() {
        return Ok(());
    }
    validate(folder.to_path_buf(), files).await
}

// ---------------------------------------------------------------------------------------------
// What the screens read

/// The clone sheet's state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CloneState {
    #[default]
    Idle,
    Cloning {
        url: String,
        phase: String,
        percent: Option<u8>,
    },
    Failed(CloneFailure),
    /// The clone worked but Compose will not load its file.
    InvalidCompose {
        folder: PathBuf,
        message: String,
    },
}

/// What a Git project is doing now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Busy {
    #[default]
    Idle,
    Checking,
    Pulling,
}

/// One Git project as the screens show it.
#[derive(Debug, Clone, PartialEq)]
pub struct GitStatus {
    pub project: GitProject,
    pub incoming: Vec<IncomingCommit>,
    pub busy: Busy,
    pub last_checked: Option<DateTime<Utc>>,
    /// A sentence when the last check or pull failed.
    pub error: Option<String>,
    /// A sentence when the last pull ran: "Deployed d544a3d."
    pub notice: Option<String>,
}

impl GitStatus {
    #[must_use]
    pub fn new(project: GitProject) -> Self {
        Self {
            project,
            incoming: Vec::new(),
            busy: Busy::Idle,
            last_checked: None,
            error: None,
            notice: None,
        }
    }
}

/// The signals the Git screens read. Provided once from the providers registry.
#[derive(Clone, Copy)]
pub struct GitDeployView {
    pub cloning: Signal<CloneState>,
    pub projects: Signal<BTreeMap<PathBuf, GitStatus>>,
}

/// Provide the view as context (once, from the providers registry).
pub fn provide_git_deploy_view() -> GitDeployView {
    use_context_provider(|| GitDeployView {
        cloning: Signal::new(CloneState::Idle),
        projects: Signal::new(BTreeMap::new()),
    })
}

impl GitDeployView {
    /// The status of the project in `folder`, if Berth cloned it.
    #[must_use]
    pub fn status(&self, folder: &Path) -> Option<GitStatus> {
        self.projects.read().get(folder).cloned()
    }
}

// ---------------------------------------------------------------------------------------------
// The handle: signals around the service

/// The service with what it needs to run on the UI's behalf. Provided as `Option<GitDeployHandle>`
/// context (None in a scene).
#[derive(Clone)]
pub struct GitDeployHandle {
    service: GitDeployService,
    actions: ActionsService,
    cancel: CancellationToken,
    link: Option<crate::services::engines::EngineLink>,
    /// The running clone's own token, so its Cancel button ends only that.
    clone_token: Arc<std::sync::Mutex<Option<CancellationToken>>>,
}

impl std::fmt::Debug for GitDeployHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitDeployHandle").finish_non_exhaustive()
    }
}

/// The command a redeploy runs: Up with a build, for every service.
#[must_use]
pub fn redeploy_command() -> ComposeCommand {
    ComposeCommand::Up {
        services: Vec::new(),
        build: true,
    }
}

fn unfinished(what: &str) -> String {
    format!("The {what} did not finish.")
}

impl GitDeployHandle {
    #[must_use]
    pub fn new(
        service: GitDeployService,
        actions: ActionsService,
        cancel: CancellationToken,
        link: Option<crate::services::engines::EngineLink>,
    ) -> Self {
        Self {
            service,
            actions,
            cancel,
            link,
            clone_token: Arc::default(),
        }
    }

    #[must_use]
    pub fn service(&self) -> &GitDeployService {
        &self.service
    }

    /// Load the stored projects into the view and start the polling task. Call once, inside
    /// `use_hook`.
    pub fn start(&self, mut view: GitDeployView, state: AppState) {
        let rows: BTreeMap<PathBuf, GitStatus> = self
            .service
            .projects()
            .into_iter()
            .map(|p| (p.folder.clone(), GitStatus::new(p)))
            .collect();
        view.projects.set(rows);
        let handle = self.clone();
        spawn(async move {
            loop {
                tokio::select! {
                    () = handle.cancel.cancelled() => return,
                    () = tokio::time::sleep(TICK) => {}
                }
                let now = Utc::now();
                let due: Vec<PathBuf> = view
                    .projects
                    .peek()
                    .values()
                    .filter(|s| {
                        s.busy == Busy::Idle && poll_due(s.project.poll, s.last_checked, now)
                    })
                    .map(|s| s.project.folder.clone())
                    .collect();
                for folder in due {
                    handle.poll_project(view, state, &folder).await;
                }
            }
        });
    }

    fn update(view: &mut GitDeployView, folder: &Path, change: impl FnOnce(&mut GitStatus)) {
        if let Some(status) = view.projects.write().get_mut(folder) {
            change(status);
        }
    }

    /// The redeploy of a pulled project: `Up { build: true }` through the actions service, so the
    /// project row shows "Starting..." and the outcome lands in the activity log.
    ///
    /// # Errors
    /// A sentence when the folder cannot be read, the engine is not connected or the same action
    /// is already running.
    pub fn redeploy(&self, mut state: AppState, project: &GitProject) -> Result<(), String> {
        let files = berth_rules::detect::detect(&project.folder)
            .map(|d| crate::sheets::detected::compose_files(&d))
            .map_err(|e| e.to_string())?;
        let run = PlanRun::for_folder(&project.engine, &project.folder, files);
        self.actions
            .run_for_folder(&mut state, &run, redeploy_command())
            .map(|_| ())
    }

    /// Fetch and list the incoming commits of one project.
    pub fn check_project(&self, mut view: GitDeployView, folder: PathBuf) {
        if view.status(&folder).is_none_or(|s| s.busy != Busy::Idle) {
            return;
        }
        Self::update(&mut view, &folder, |s| {
            s.busy = Busy::Checking;
            s.error = None;
            s.notice = None;
        });
        let service = self.service.clone();
        let cancel = self.cancel.clone();
        spawn(async move {
            let result = crate::services::plans::off_ui({
                let folder = folder.clone();
                async move { service.check(&folder, cancel).await }
            })
            .await
            .unwrap_or_else(|| Err(unfinished("check")));
            Self::update(&mut view, &folder, |s| {
                s.busy = Busy::Idle;
                s.last_checked = Some(Utc::now());
                match result {
                    Ok(incoming) => s.incoming = incoming,
                    Err(message) => s.error = Some(message),
                }
            });
        });
    }

    /// Pull the incoming commits, then redeploy.
    pub fn pull_project(&self, mut view: GitDeployView, state: AppState, folder: PathBuf) {
        if view.status(&folder).is_none_or(|s| s.busy != Busy::Idle) {
            return;
        }
        Self::update(&mut view, &folder, |s| {
            s.busy = Busy::Pulling;
            s.error = None;
            s.notice = None;
        });
        let handle = self.clone();
        spawn(async move {
            let result = handle.pull_now(state, &folder).await;
            Self::update(&mut view, &folder, |s| {
                s.busy = Busy::Idle;
                match result {
                    Ok(pulled) => {
                        s.incoming.clear();
                        s.project.commit.clone_from(&pulled.to);
                        s.notice = Some(pulled.to.map_or_else(
                            || "Pulled. Redeploying.".to_string(),
                            |commit| format!("Pulled {commit}. Redeploying."),
                        ));
                    }
                    Err(message) => s.error = Some(message),
                }
            });
        });
    }

    /// The pull (off the UI path), then the redeploy (which needs the signals).
    async fn pull_now(&self, state: AppState, folder: &Path) -> Result<Pulled, String> {
        let service = self.service.clone();
        let cancel = self.cancel.clone();
        let target = folder.to_path_buf();
        let pulled =
            crate::services::plans::off_ui(async move { service.pull(&target, cancel).await })
                .await
                .unwrap_or_else(|| Err(unfinished("pull")))?;
        let project = self.service.project(folder)?;
        self.redeploy(state, &project)?;
        Ok(pulled)
    }

    /// One poll of one project, as the polling task makes it (D72).
    async fn poll_project(&self, mut view: GitDeployView, state: AppState, folder: &Path) {
        Self::update(&mut view, folder, |s| {
            s.busy = Busy::Checking;
            s.error = None;
        });
        let service = self.service.clone();
        let cancel = self.cancel.clone();
        let target = folder.to_path_buf();
        let found =
            crate::services::plans::off_ui(async move { service.poll(&target, cancel).await })
                .await
                .unwrap_or_else(|| Err(unfinished("check")));
        // An automatic pull deploys from here, where the signals are.
        let found = found.and_then(|outcome| {
            if outcome.pulled.is_some() {
                self.redeploy(state, &self.service.project(folder)?)?;
            }
            Ok(outcome)
        });
        Self::update(&mut view, folder, |s| {
            s.busy = Busy::Idle;
            s.last_checked = Some(Utc::now());
            match found {
                Ok(outcome) => {
                    s.incoming = outcome.incoming;
                    if let Some(pulled) = outcome.pulled {
                        s.project.commit.clone_from(&pulled.to);
                        s.notice = Some(format!(
                            "{} pulled. Redeploying.",
                            new_commits_label(pulled.commits.len())
                        ));
                    }
                }
                Err(message) => s.error = Some(message),
            }
        });
    }

    /// Switch Auto-redeploy for a project (D72).
    pub fn set_auto_redeploy(&self, mut view: GitDeployView, folder: &Path, on: bool) {
        let saved = self.service.set_auto_redeploy(folder, on);
        Self::update(&mut view, folder, |s| match saved {
            Ok(project) => s.project = project,
            Err(message) => s.error = Some(message),
        });
    }

    /// Set a project's polling interval in minutes (`None` is off).
    pub fn set_poll_minutes(&self, mut view: GitDeployView, folder: &Path, minutes: Option<u64>) {
        let saved = self
            .service
            .set_poll(folder, minutes.map(|m| Duration::from_secs(m * 60)));
        Self::update(&mut view, folder, |s| match saved {
            Ok(project) => s.project = project,
            Err(message) => s.error = Some(message),
        });
    }

    /// Clone for the From Git sheet. Progress and failure land in `view.cloning`; on success the
    /// folder is pinned and the Detected sheet opens (or the invalid-compose state shows).
    pub fn clone_project(&self, mut view: GitDeployView, state: AppState, request: CloneRequest) {
        if matches!(*view.cloning.peek(), CloneState::Cloning { .. }) {
            return;
        }
        let url = request.url.to_string();
        view.cloning.set(CloneState::Cloning {
            url: url.clone(),
            phase: "Starting".to_string(),
            percent: None,
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<CloneProgress>();
        let service = self.service.clone();
        let cancel = self.cancel.child_token();
        if let Ok(mut slot) = self.clone_token.lock() {
            *slot = Some(cancel.clone());
        }
        let ended = cancel.clone();
        // `tx` lives in the progress closure, so the receiver ends when the clone does.
        let work = crate::services::plans::off_ui(async move {
            service
                .clone_repo(&request, cancel, &move |progress| {
                    let _ = tx.send(progress);
                })
                .await
        });
        let handle = self.clone();
        spawn(async move {
            let watching = async {
                while let Some(progress) = rx.recv().await {
                    view.cloning.set(CloneState::Cloning {
                        url: url.clone(),
                        phase: progress.phase,
                        percent: progress.percent,
                    });
                }
            };
            let (_, cloned) = futures::join!(watching, work);
            if ended.is_cancelled() {
                // Cancel was pressed: the folder git began is already removed.
                view.cloning.set(CloneState::Idle);
                return;
            }
            match cloned {
                None => view.cloning.set(CloneState::Failed(CloneFailure {
                    kind: FailureKind::Other,
                    message: unfinished("clone"),
                    fix: "Try again.".to_string(),
                })),
                Some(Err(failure)) => view.cloning.set(CloneState::Failed(failure)),
                Some(Ok(project)) => {
                    if let Some(link) = &handle.link {
                        link.pins_changed();
                    }
                    let folder = project.folder.clone();
                    view.projects
                        .write()
                        .insert(folder.clone(), GitStatus::new(project));
                    handle.after_clone(view, state, folder).await;
                }
            }
        });
    }

    /// End the clone in progress (the sheet's Cancel).
    pub fn cancel_clone(&self) {
        if let Ok(slot) = self.clone_token.lock() {
            if let Some(token) = slot.as_ref() {
                token.cancel();
            }
        }
    }

    /// The clone worked: read the folder, check its Compose file, then open the Detected sheet
    /// (Deploy and the generate flow live there) or show why Compose refuses it.
    async fn after_clone(&self, mut view: GitDeployView, mut state: AppState, folder: PathBuf) {
        let read = {
            let folder = folder.clone();
            crate::services::plans::blocking(move || {
                berth_rules::detect::detect(&folder)
                    .map(|d| crate::sheets::detected::compose_files(&d))
            })
            .await
        };
        let files = match read {
            Some(Ok(files)) => files,
            Some(Err(err)) => {
                view.cloning.set(CloneState::InvalidCompose {
                    folder,
                    message: err.to_string(),
                });
                return;
            }
            None => Vec::new(),
        };
        let actions = self.actions.clone();
        let verdict = judge_compose(&folder, files, |dir, files| async move {
            actions.validate_compose(&dir, &files).await
        })
        .await;
        match verdict {
            Ok(()) => {
                view.cloning.set(CloneState::Idle);
                let mut overlays = state.overlays.write();
                overlays.retain(|o| !matches!(o, Overlay::NewProject));
                overlays.push(Overlay::Detected(folder));
            }
            Err(message) => view
                .cloning
                .set(CloneState::InvalidCompose { folder, message }),
        }
    }
}

#[cfg(test)]
#[path = "git_deploy_tests.rs"]
mod tests;
