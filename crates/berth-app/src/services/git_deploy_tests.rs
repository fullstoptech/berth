//! Tests of the Git deploy service (P8-T5): the logic over a fake `git` and an in-memory store,
//! and the redeploy glue over the mock Compose.

use std::sync::Mutex;

use super::*;
use crate::state::{use_app_state, AppState};

// --- A fake git: one remote history, one local clone. ---

#[derive(Default)]
struct FakeGit {
    inner: Mutex<Fake>,
}

#[derive(Default)]
struct Fake {
    /// The remote's commits, oldest first: `(hash, subject)`.
    remote: Vec<(String, String)>,
    /// How many of them the clone has checked out.
    local: usize,
    /// How many the clone has fetched.
    fetched: usize,
    clone_error: Option<String>,
    pull_error: Option<String>,
    fetch_calls: usize,
    pull_calls: usize,
}

impl FakeGit {
    fn with_commits(commits: &[(&str, &str)]) -> Arc<Self> {
        let fake = Arc::new(Self::default());
        fake.lock().remote = commits
            .iter()
            .map(|(h, s)| ((*h).to_string(), (*s).to_string()))
            .collect();
        fake
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Fake> {
        self.inner.lock().expect("fake lock")
    }

    fn push(&self, hash: &str, subject: &str) {
        self.lock()
            .remote
            .push((hash.to_string(), subject.to_string()));
    }
}

#[async_trait::async_trait]
impl GitOps for FakeGit {
    async fn clone_repo(
        &self,
        _request: &CloneRequest,
        target: &Path,
        _cancel: CancellationToken,
        on_progress: &(dyn Fn(CloneProgress) + Send + Sync),
    ) -> Result<(), EngineError> {
        let mut fake = self.lock();
        if let Some(message) = fake.clone_error.clone() {
            return Err(EngineError::Docker(message));
        }
        on_progress(CloneProgress {
            phase: "Receiving objects".to_string(),
            percent: Some(40),
        });
        on_progress(CloneProgress {
            phase: "Resolving deltas".to_string(),
            percent: Some(100),
        });
        std::fs::create_dir_all(target).map_err(|e| EngineError::Docker(e.to_string()))?;
        fake.local = fake.remote.len();
        fake.fetched = fake.remote.len();
        Ok(())
    }

    async fn fetch(&self, _dir: &Path, _cancel: CancellationToken) -> Result<(), EngineError> {
        let mut fake = self.lock();
        fake.fetch_calls += 1;
        fake.fetched = fake.remote.len();
        Ok(())
    }

    async fn incoming(
        &self,
        _dir: &Path,
        _cancel: CancellationToken,
    ) -> Result<Vec<IncomingCommit>, EngineError> {
        let fake = self.lock();
        Ok(fake.remote[fake.local..fake.fetched]
            .iter()
            .rev()
            .map(|(hash, subject)| IncomingCommit {
                hash: hash.clone(),
                subject: subject.clone(),
            })
            .collect())
    }

    async fn pull_ff_only(
        &self,
        _dir: &Path,
        _cancel: CancellationToken,
    ) -> Result<(), EngineError> {
        let mut fake = self.lock();
        fake.pull_calls += 1;
        if let Some(message) = fake.pull_error.clone() {
            return Err(EngineError::Docker(message));
        }
        fake.local = fake.fetched;
        Ok(())
    }

    async fn current_commit(
        &self,
        _dir: &Path,
        _cancel: CancellationToken,
    ) -> Result<Option<String>, EngineError> {
        let fake = self.lock();
        Ok(fake.local.checked_sub(1).map(|i| fake.remote[i].0.clone()))
    }

    async fn branch(
        &self,
        _dir: &Path,
        _cancel: CancellationToken,
    ) -> Result<Option<String>, EngineError> {
        Ok(Some("main".to_string()))
    }
}

fn engine() -> EngineId {
    EngineId("colima:default".to_string())
}

fn store() -> Arc<Store> {
    Arc::new(Store::open_in_memory().expect("store"))
}

fn request(parent: &Path) -> CloneRequest {
    CloneRequest {
        url: parse_repo_url("https://example.com/acme/shop.git").expect("url"),
        branch: None,
        ssh_key: None,
        parent: parent.to_path_buf(),
        engine: engine(),
    }
}

/// A service whose remote has two commits, with the clone already made.
async fn cloned() -> (
    GitDeployService,
    Arc<FakeGit>,
    GitProject,
    tempfile::TempDir,
) {
    let git = FakeGit::with_commits(&[("b09ecba", "initial commit"), ("d544a3d", "second commit")]);
    let service = GitDeployService::new(store(), git.clone());
    let parent = tempfile::tempdir().expect("tempdir");
    let project = service
        .clone_repo(&request(parent.path()), CancellationToken::new(), &|_| {})
        .await
        .expect("clone");
    (service, git, project, parent)
}

fn token() -> CancellationToken {
    CancellationToken::new()
}

// --- The address ---

#[test]
fn git_deploy_address_forms_become_urls_and_a_password_is_refused() {
    assert_eq!(
        parse_repo_url("https://github.com/acme/shop.git")
            .expect("https")
            .as_str(),
        "https://github.com/acme/shop.git"
    );
    assert_eq!(
        parse_repo_url("git@github.com:acme/shop.git")
            .expect("scp")
            .as_str(),
        "ssh://git@github.com/acme/shop.git"
    );
    assert_eq!(
        parse_repo_url("/srv/git/shop.git").expect("path").as_str(),
        "file:///srv/git/shop.git"
    );
    let secret = parse_repo_url("https://me:hunter2@example.com/a/b.git").expect_err("password");
    assert!(secret.contains("password"), "{secret}");
    assert!(!secret.contains("hunter2"), "the refusal never repeats it");
    for bad in ["", "   ", "shop", "ftp://example.com/a.git", "https://"] {
        assert!(parse_repo_url(bad).is_err(), "{bad:?} is not an address");
    }
}

#[test]
fn git_deploy_folder_name_is_the_repo_without_dot_git() {
    let name = |text: &str| repo_folder_name(&parse_repo_url(text).expect("url"));
    assert_eq!(name("https://github.com/acme/shop.git"), "shop");
    assert_eq!(name("git@github.com:acme/shop-api.git"), "shop-api");
    assert_eq!(name("https://example.com/acme/shop/"), "shop");
    assert_eq!(name("/srv/git/berthfx-app.git"), "berthfx-app");
}

#[test]
fn git_deploy_clone_is_blocked_until_the_form_is_complete() {
    let parent = tempfile::tempdir().expect("tempdir");
    let url = "https://example.com/acme/shop.git";
    assert!(clone_blocker("", Some(parent.path()), Some(&engine()))
        .expect("blocked")
        .contains("address"));
    assert_eq!(
        clone_blocker(url, None, Some(&engine())).as_deref(),
        Some("Choose the folder to clone into.")
    );
    assert_eq!(
        clone_blocker(url, Some(parent.path()), None).as_deref(),
        Some("No engine is running. Start one first.")
    );
    assert_eq!(
        clone_blocker(url, Some(parent.path()), Some(&engine())),
        None
    );
    std::fs::create_dir(parent.path().join("shop")).expect("mkdir");
    assert!(clone_blocker(url, Some(parent.path()), Some(&engine()))
        .expect("exists")
        .contains("already exists"));
}

#[test]
fn git_deploy_ssh_keys_are_the_private_files_with_a_public_pair() {
    let dir = tempfile::tempdir().expect("tempdir");
    for name in [
        "id_ed25519",
        "id_ed25519.pub",
        "id_old",
        "known_hosts",
        "work_key",
        "work_key.pub",
    ] {
        std::fs::write(dir.path().join(name), "x").expect("write");
    }
    let keys: Vec<String> = list_ssh_keys(dir.path())
        .iter()
        .map(|p| p.file_name().expect("name").to_string_lossy().into_owned())
        .collect();
    assert_eq!(keys, ["id_ed25519", "work_key"]);
    assert!(list_ssh_keys(&dir.path().join("missing")).is_empty());
}

// --- Failures ---

#[test]
fn git_deploy_failures_say_what_to_fix() {
    let ssh = parse_repo_url("git@github.com:acme/shop.git").expect("ssh");
    let https = parse_repo_url("https://github.com/acme/shop.git").expect("https");

    let auth = classify_clone_failure("git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.", &ssh);
    assert_eq!(auth.kind, FailureKind::Auth);
    assert!(auth.fix.contains("ssh-add"), "{}", auth.fix);
    assert_eq!(
        auth.message,
        "fatal: Could not read from remote repository."
    );

    // Each phrase counts on its own, not only together with "publickey".
    for text in [
        "Permission denied, please try again.",
        "remote: Invalid username or password.\nfatal: Authentication failed for 'https://x/y.git/'",
    ] {
        assert_eq!(classify_clone_failure(text, &ssh).kind, FailureKind::Auth, "{text}");
    }

    let login = classify_clone_failure(
        "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
        &https,
    );
    assert_eq!(login.kind, FailureKind::Auth);
    assert!(login.fix.contains("credential helper"), "{}", login.fix);

    let url = classify_clone_failure(
        "fatal: repository 'https://github.com/acme/nope.git/' not found",
        &https,
    );
    assert_eq!(url.kind, FailureKind::Url);
    assert!(url.fix.contains("address"), "{}", url.fix);
    let branch = classify_clone_failure("warning: Could not find remote branch dev to clone.\nfatal: Remote branch dev not found in upstream origin", &https);
    assert_eq!(branch.kind, FailureKind::Url);

    let host = classify_clone_failure(
        "Host key verification failed.\nfatal: Could not read from remote repository.",
        &ssh,
    );
    assert_eq!(host.kind, FailureKind::HostKey);
    assert!(host.fix.contains("github.com"), "{}", host.fix);

    assert_eq!(classify_clone_failure("", &https).kind, FailureKind::Other);
}

#[tokio::test]
async fn git_deploy_failed_clone_stores_and_pins_nothing() {
    let git = FakeGit::with_commits(&[("b09ecba", "initial commit")]);
    git.lock().clone_error = Some("git@github.com: Permission denied (publickey).".to_string());
    let store = store();
    let service = GitDeployService::new(Arc::clone(&store), git);
    let parent = tempfile::tempdir().expect("tempdir");
    let failure = service
        .clone_repo(&request(parent.path()), token(), &|_| {})
        .await
        .expect_err("fails");
    assert_eq!(failure.kind, FailureKind::Auth);
    assert!(store.git_sources().expect("sources").is_empty());
    assert!(store.pins().expect("pins").is_empty());
}

// --- Clone ---

#[tokio::test]
async fn git_deploy_clone_reports_progress_remembers_the_source_and_pins() {
    let git = FakeGit::with_commits(&[("b09ecba", "initial commit"), ("d544a3d", "second commit")]);
    let store = store();
    let service = GitDeployService::new(Arc::clone(&store), git);
    let parent = tempfile::tempdir().expect("tempdir");
    let heard = Mutex::new(Vec::new());
    let project = service
        .clone_repo(&request(parent.path()), token(), &|p| {
            heard.lock().expect("lock").push((p.phase, p.percent));
        })
        .await
        .expect("clone");

    assert_eq!(
        *heard.lock().expect("lock"),
        [
            ("Receiving objects".to_string(), Some(40)),
            ("Resolving deltas".to_string(), Some(100))
        ]
    );
    assert_eq!(project.commit.as_deref(), Some("d544a3d"));
    assert_eq!(project.branch, "main");
    assert!(!project.auto_redeploy, "Auto-redeploy starts off (D72)");
    assert_eq!(project.poll, None, "polling starts off (D72)");
    assert_eq!(
        project.folder,
        std::fs::canonicalize(parent.path().join("shop")).expect("canonical")
    );
    assert_eq!(service.projects(), vec![project.clone()]);
    let pins = store.pins().expect("pins");
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].folder, project.folder);
    assert_eq!(pins[0].engine, engine());
}

// --- New commits, pull, poll ---

#[tokio::test]
async fn git_deploy_check_lists_exactly_the_new_commits() {
    let (service, git, project, _parent) = cloned().await;
    assert!(
        service
            .check(&project.folder, token())
            .await
            .expect("check")
            .is_empty(),
        "a fresh clone has nothing incoming"
    );
    git.push("aaa1111", "add the cache");
    let incoming = service
        .check(&project.folder, token())
        .await
        .expect("check");
    assert_eq!(
        incoming,
        [IncomingCommit {
            hash: "aaa1111".to_string(),
            subject: "add the cache".to_string()
        }]
    );
    assert_eq!(new_commits_label(incoming.len()), "1 new commit");
    assert_eq!(new_commits_label(3), "3 new commits");
    assert_eq!(new_commits_label(0), "0 new commits");
}

#[tokio::test]
async fn git_deploy_pull_brings_the_new_commit_and_remembers_it() {
    let (service, git, project, _parent) = cloned().await;
    git.push("aaa1111", "add the cache");
    service
        .check(&project.folder, token())
        .await
        .expect("check");

    let pulled = service.pull(&project.folder, token()).await.expect("pull");
    assert_eq!(pulled.from.as_deref(), Some("d544a3d"));
    assert_eq!(pulled.to.as_deref(), Some("aaa1111"));
    assert_eq!(pulled.commits.len(), 1);
    assert_eq!(pulled.commits[0].subject, "add the cache");
    assert_eq!(
        service.projects()[0].commit.as_deref(),
        Some("aaa1111"),
        "the project row shows the new commit"
    );
    assert!(service
        .check(&project.folder, token())
        .await
        .expect("check")
        .is_empty());
}

#[tokio::test]
async fn git_deploy_refused_pull_keeps_the_commit_and_says_why() {
    let (service, git, project, _parent) = cloned().await;
    git.push("aaa1111", "rewrite history");
    git.lock().pull_error = Some("fatal: Not possible to fast-forward, aborting.".to_string());
    service
        .check(&project.folder, token())
        .await
        .expect("check");

    let err = service
        .pull(&project.folder, token())
        .await
        .expect_err("refused");
    assert!(err.contains("fast-forward"), "{err}");
    assert_eq!(service.projects()[0].commit.as_deref(), Some("d544a3d"));
}

#[tokio::test]
async fn git_deploy_poll_with_auto_redeploy_off_only_reports() {
    let (service, git, project, _parent) = cloned().await;
    git.push("aaa1111", "add the cache");

    let outcome = service.poll(&project.folder, token()).await.expect("poll");
    assert_eq!(outcome.incoming.len(), 1, "shows 1 new commit");
    assert_eq!(outcome.pulled, None, "and does not pull");
    assert_eq!(git.lock().pull_calls, 0);
    assert_eq!(service.projects()[0].commit.as_deref(), Some("d544a3d"));
}

#[tokio::test]
async fn git_deploy_poll_with_auto_redeploy_on_pulls_for_the_caller_to_redeploy() {
    let (service, git, project, _parent) = cloned().await;
    service
        .set_auto_redeploy(&project.folder, true)
        .expect("switch on");
    // Nothing new: nothing pulled, nothing to redeploy.
    let quiet = service.poll(&project.folder, token()).await.expect("poll");
    assert_eq!(
        quiet,
        PollOutcome {
            incoming: Vec::new(),
            pulled: None
        }
    );
    assert_eq!(git.lock().pull_calls, 0);

    git.push("aaa1111", "add the cache");
    let outcome = service.poll(&project.folder, token()).await.expect("poll");
    let pulled = outcome.pulled.expect("pulled");
    assert_eq!(pulled.to.as_deref(), Some("aaa1111"));
    assert!(outcome.incoming.is_empty());
    assert_eq!(service.projects()[0].commit.as_deref(), Some("aaa1111"));
}

#[tokio::test]
async fn git_deploy_settings_persist_per_project() {
    let (service, _git, project, _parent) = cloned().await;
    service
        .set_poll(&project.folder, Some(Duration::from_secs(300)))
        .expect("poll");
    service
        .set_auto_redeploy(&project.folder, true)
        .expect("auto");
    let stored = &service.projects()[0];
    assert_eq!(stored.poll, Some(Duration::from_secs(300)));
    assert!(stored.auto_redeploy);
    service.set_poll(&project.folder, None).expect("off");
    assert_eq!(service.projects()[0].poll, None);
    assert!(
        service
            .set_auto_redeploy(Path::new("/not/cloned"), true)
            .is_err(),
        "only Git projects have the switch"
    );
}

#[test]
fn git_deploy_poll_is_due_only_when_on_and_the_interval_has_passed() {
    let now = Utc::now();
    let five = Some(Duration::from_secs(300));
    assert!(!poll_due(None, None, now), "off is never due");
    assert!(poll_due(five, None, now), "never checked is due");
    assert!(!poll_due(
        five,
        Some(now - chrono::Duration::minutes(4)),
        now
    ));
    assert!(poll_due(
        five,
        Some(now - chrono::Duration::minutes(5)),
        now
    ));
}

// --- Compose check after a clone ---

#[tokio::test]
async fn git_deploy_invalid_compose_is_reported_and_no_compose_passes() {
    let dir = Path::new("/x/shop");
    let invalid = judge_compose(dir, vec![PathBuf::from("compose.yaml")], |_, _| async {
        Err("compose.yaml does not load, line 3: services.web must be a mapping".to_string())
    })
    .await
    .expect_err("invalid");
    assert!(invalid.contains("line 3"), "{invalid}");

    judge_compose(dir, vec![PathBuf::from("compose.yaml")], |_, _| async {
        Ok(())
    })
    .await
    .expect("valid");
    // No Compose file: the generate flow handles it; Compose is never asked.
    judge_compose(dir, Vec::new(), |_, _| async {
        panic!("validate must not run without a compose file")
    })
    .await
    .expect("no compose");
}

// --- The redeploy glue ---

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some(state));
    });
    rsx! { div {} }
}

#[tokio::test]
async fn git_deploy_redeploy_runs_up_with_a_build_on_the_projects_engine() {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT.with(|slot| slot.borrow_mut().take()).expect("state");

    let folder = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        folder.path().join("compose.yaml"),
        "services:\n  web:\n    image: nginx:alpine\n",
    )
    .expect("compose file");
    let clients = crate::services::engines::client_map();
    clients.lock().expect("clients").insert(
        engine(),
        Arc::new(berth_engine::MockEngine::bundled(&engine()))
            as Arc<dyn berth_engine::EngineClient>,
    );
    let compose = Arc::new(berth_engine::MockCompose::bundled());
    let (actions, mut finished) = ActionsService::new(
        clients,
        store(),
        Arc::clone(&compose) as Arc<dyn berth_engine::ComposeRunner>,
        Arc::new(berth_engine::MockColima::bundled()),
        token(),
    );
    let handle = GitDeployHandle::new(
        GitDeployService::new(store(), FakeGit::with_commits(&[])),
        actions,
        token(),
        None,
    );
    let project = GitProject {
        folder: folder.path().to_path_buf(),
        engine: engine(),
        repo: parse_repo_url("https://example.com/acme/shop.git").expect("url"),
        branch: "main".to_string(),
        commit: Some("d544a3d".to_string()),
        poll: None,
        auto_redeploy: false,
    };

    dom.in_runtime(|| handle.redeploy(state, &project))
        .expect("accepted");
    tokio::time::timeout(Duration::from_secs(5), finished.recv())
        .await
        .expect("a Finished within 5 s")
        .expect("channel open");

    let ran = compose.recorded();
    assert_eq!(ran.len(), 1);
    assert_eq!(
        ran[0].1,
        ComposeCommand::Up {
            services: Vec::new(),
            build: true
        },
        "Pull and redeploy runs Up with a build"
    );
    assert_eq!(redeploy_command(), ran[0].1);
}

// --- Work that outlives the component that started it ---

type UnmountSlot = (AppState, GitDeployView, Signal<bool>);

thread_local! {
    static UNMOUNT: std::cell::RefCell<Option<UnmountSlot>> =
        const { std::cell::RefCell::new(None) };
    static UNMOUNT_CALLER: std::cell::Cell<Option<ScopeId>> = const { std::cell::Cell::new(None) };
}

/// The App root's part (the state and the Git view) with a child that can be removed.
#[component]
fn UnmountHarness() -> Element {
    let state = use_app_state();
    let view = GitDeployView {
        cloning: use_signal(|| CloneState::Idle),
        projects: use_signal(BTreeMap::new),
    };
    let shown = use_signal(|| true);
    use_hook(move || UNMOUNT.with(|slot| *slot.borrow_mut() = Some((state, view, shown))));
    rsx! {
        div {}
        if shown() {
            UnmountCaller {}
        }
    }
}

#[component]
fn UnmountCaller() -> Element {
    use_hook(|| UNMOUNT_CALLER.with(|slot| slot.set(Some(dioxus::core::current_scope_id()))));
    rsx! { div {} }
}

/// P8-HC: the From Git sheet starts the clone from its form and swaps that form out at once;
/// the clone must still land its result.
#[tokio::test]
async fn git_deploy_clone_started_from_a_child_that_unmounts_still_lands() {
    let mut dom = VirtualDom::new(UnmountHarness);
    let _ = dom.rebuild_to_vec();
    let (state, view, mut shown) = UNMOUNT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness");
    let caller = UNMOUNT_CALLER
        .with(std::cell::Cell::get)
        .expect("caller mounted");
    let (actions, _finished) = ActionsService::new(
        crate::services::engines::client_map(),
        store(),
        Arc::new(berth_engine::MockCompose::bundled()) as Arc<dyn berth_engine::ComposeRunner>,
        Arc::new(berth_engine::MockColima::bundled()),
        token(),
    );
    let handle = GitDeployHandle::new(
        GitDeployService::new(
            store(),
            FakeGit::with_commits(&[("b09ecba", "initial commit")]),
        ),
        actions,
        token(),
        None,
    );
    let parent = tempfile::tempdir().expect("tempdir");
    dom.in_scope(caller, || {
        handle.clone_project(view, state, request(parent.path()));
    });
    assert!(matches!(
        dom.in_runtime(|| view.cloning.peek().clone()),
        CloneState::Cloning { .. }
    ));
    dom.in_runtime(|| shown.set(false));
    dom.render_immediate(&mut dioxus::core::NoOpMutations);

    let waited = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let landed = dom.in_runtime(|| {
                !matches!(*view.cloning.peek(), CloneState::Cloning { .. })
                    && !view.projects.peek().is_empty()
            });
            if landed {
                return;
            }
            let _ = tokio::time::timeout(Duration::from_millis(50), dom.wait_for_work()).await;
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
        }
    })
    .await;
    assert!(waited.is_ok(), "the clone's result did not land");
}
