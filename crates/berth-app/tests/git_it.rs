#![allow(clippy::expect_used)]
//! Live project from Git against the first running engine (P8-T5 done-when).
//!
//! The repository is a bare copy of `fixtures/git/berthfx-app.git` (built from
//! `fixtures/git/berthfx-app/` when `scripts/fx-git.sh` has not run) in a temp folder, so the
//! fixture itself is never pushed to. Berth clones it into `<tmp>/deploys/berthfx-app`, which is a
//! Compose project named `berthfx-app` on host port 56091 (CLAUDE.md, AD33). The test:
//!
//! 1. clones and deploys it (`Up`, as Start project does) and waits until it serves,
//! 2. pushes a new commit to the bare repo from another clone,
//! 3. polls: "1 new commit", with Auto-redeploy off nothing is pulled or redeployed (D72),
//! 4. pulls and redeploys (`Up` with a build) and sees the new commit's page served,
//! 5. ends with `docker compose down -v` on its own project, also when an assertion fails.
//!
//! Run from the worktree with `BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only git_it`
//! under the engine lock. The engine is the first running one discovery finds, not a fixed id.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::actions::{apply_finished, ActionsService, Finished};
use berth_app::services::engines::client_map;
use berth_app::services::git_deploy::{
    new_commits_label, parse_repo_url, CloneRequest, GitDeployHandle, GitDeployService,
};
use berth_app::services::plans::PlanRun;
use berth_app::state::{use_app_state, AppState};
use berth_core::ComposeCommand;

const PROJECT: &str = "berthfx-app";
const PORT: u16 = 56091;
const MARKER: &str = "pulled by Berth: third commit";

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

fn harness() -> (VirtualDom, AppState) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    (dom, state)
}

/// git with a fixed identity, no signing and no prompts; panics unless it exits 0.
fn git(git_bin: &Path, dir: Option<&Path>, args: &[&str]) -> String {
    let mut command = std::process::Command::new(git_bin);
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    let out = command
        .args([
            "-c",
            "user.name=Berth Fixtures",
            "-c",
            "user.email=fixtures@berth.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/git")
}

/// `<tmp>/remote/berthfx-app.git`: a bare copy of the fixture repo.
fn make_remote(git_bin: &Path, tmp: &Path) -> PathBuf {
    let remote = tmp.join("remote").join(format!("{PROJECT}.git"));
    std::fs::create_dir_all(remote.parent().expect("parent")).expect("mkdir");
    let built = fixtures().join(format!("{PROJECT}.git"));
    if built.is_dir() {
        git(
            git_bin,
            None,
            &[
                "clone",
                "-q",
                "--bare",
                built.to_str().expect("utf8"),
                remote.to_str().expect("utf8"),
            ],
        );
        return remote;
    }
    // scripts/fx-git.sh has not run: build the same two commits from the committed tree.
    let src = tmp.join("src");
    git(
        git_bin,
        None,
        &["init", "-q", "-b", "main", src.to_str().expect("utf8")],
    );
    let tree = fixtures().join(PROJECT);
    copy_tree(&tree, &src);
    git(git_bin, Some(&src), &["add", "-A"]);
    git(
        git_bin,
        Some(&src),
        &["commit", "-q", "-m", "berthfx-app: initial commit"],
    );
    let changelog = src.join("CHANGELOG.md");
    let mut text = std::fs::read_to_string(&changelog).expect("changelog");
    text.push_str("- Fixture build note (commit two).\n");
    std::fs::write(&changelog, text).expect("write changelog");
    git(
        git_bin,
        Some(&src),
        &["commit", "-q", "-am", "berthfx-app: note the fixture build"],
    );
    git(
        git_bin,
        None,
        &[
            "clone",
            "-q",
            "--bare",
            src.to_str().expect("utf8"),
            remote.to_str().expect("utf8"),
        ],
    );
    remote
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir").flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("copy");
        }
    }
}

/// One plain HTTP/1.0 GET of `/` on the fixture's published port.
fn http_get() -> Option<String> {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", PORT)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    stream
        .write_all(b"GET / HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .ok()?;
    let mut body = String::new();
    stream.read_to_string(&mut body).ok()?;
    Some(body)
}

async fn serves(needle: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if http_get().is_some_and(|body| body.contains(needle)) {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// `docker compose down -v` for the test's own project, also when an assertion fails.
struct DownOnDrop {
    docker: PathBuf,
    host: String,
    folder: PathBuf,
}

impl Drop for DownOnDrop {
    fn drop(&mut self) {
        assert_eq!(
            self.folder.file_name().and_then(|n| n.to_str()),
            Some(PROJECT),
            "only the berthfx-app clone"
        );
        let _ = std::process::Command::new(&self.docker)
            .env("DOCKER_HOST", &self.host)
            .args(["compose", "-p", PROJECT, "--project-directory"])
            .arg(&self.folder)
            .args(["down", "-v", "--remove-orphans"])
            .output();
    }
}

struct Live {
    engine: berth_core::Engine,
    docker: PathBuf,
    host: String,
    git: PathBuf,
    actions: ActionsService,
    finished: tokio::sync::mpsc::UnboundedReceiver<Finished>,
    store: Arc<berth_store::Store>,
    dom: VirtualDom,
    state: AppState,
}

async fn live() -> Live {
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima_cli = berth_engine::ColimaCli::new(search.clone());
    let engines = berth_engine::discovery::discover(&colima_cli, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.state == berth_core::EngineState::Running && e.docker_host.is_some())
        .expect("a running engine")
        .clone();
    let host = engine.docker_host.clone().expect("docker host");
    let docker = search
        .find("docker")
        .unwrap_or_else(|| PathBuf::from("docker"));
    let git = search.find("git").expect("git on PATH");
    let client: Arc<dyn berth_engine::EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let clients = client_map();
    clients
        .lock()
        .expect("clients")
        .insert(engine.id.clone(), client);
    let (actions, finished) = ActionsService::new(
        clients,
        Arc::clone(&store),
        Arc::new(berth_engine::CliCompose::new(search)),
        Arc::new(berth_engine::MockColima::bundled()),
        CancellationToken::new(),
    );
    let (dom, state) = harness();
    Live {
        engine,
        docker,
        host,
        git,
        actions,
        finished,
        store,
        dom,
        state,
    }
}

impl Live {
    fn docker(&self, args: &[&str]) -> String {
        let out = std::process::Command::new(&self.docker)
            .env("DOCKER_HOST", &self.host)
            .args(args)
            .output()
            .expect("docker runs");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// `name state` of every container of this test's Compose project.
    fn project_containers(&self) -> Vec<String> {
        self.docker(&[
            "ps",
            "-a",
            "--filter",
            &format!("label=com.docker.compose.project={PROJECT}"),
            "--format",
            "{{.Names}} {{.State}}",
        ])
        .lines()
        .map(str::to_string)
        .collect()
    }

    async fn wait_finished(&mut self, secs: u64) -> Result<(), String> {
        let fin = tokio::time::timeout(Duration::from_secs(secs), self.finished.recv())
            .await
            .expect("the project action finished in time")
            .expect("channel open");
        let result = fin.result.clone();
        let mut state = self.state;
        self.dom.in_runtime(|| apply_finished(&mut state, fin));
        result
    }
}

#[tokio::test]
#[ignore = "needs a running engine; run with BERTH_IT=1"]
async fn git_it_deploys_berthfx_app_then_pulls_and_redeploys_a_new_commit() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let mut live = live().await;
    // The clone is bind-mounted into the container, so it lives under the worktree (the home
    // the engine VM shares), not in the system temp dir.
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/git_it");
    std::fs::create_dir_all(&scratch).expect("scratch dir");
    let scratch = std::fs::canonicalize(&scratch).expect("canonical scratch");
    let tmp = tempfile::tempdir_in(&scratch).expect("tmp");
    let remote = make_remote(&live.git, tmp.path());
    let remote_url = parse_repo_url(remote.to_str().expect("utf8")).expect("file url");

    let service = GitDeployService::real(Arc::clone(&live.store));
    let handle = GitDeployHandle::new(
        service.clone(),
        live.actions.clone(),
        CancellationToken::new(),
        None,
    );

    // 1. Clone: the folder is named after the repository, so Compose calls the project berthfx-app.
    let remote_head = git(&live.git, Some(&remote), &["rev-parse", "--short", "HEAD"]);
    let heard = std::sync::Mutex::new(0usize);
    let project = service
        .clone_repo(
            &CloneRequest {
                url: remote_url,
                branch: None,
                ssh_key: None,
                parent: tmp.path().join("deploys"),
                engine: live.engine.id.clone(),
            },
            CancellationToken::new(),
            &|_| *heard.lock().expect("lock") += 1,
        )
        .await
        .expect("clone");
    assert_eq!(project.commit.as_deref(), Some(remote_head.as_str()));
    assert_eq!(project.branch, "main");
    assert!(project.folder.join("compose.yaml").is_file());
    assert_eq!(
        project.folder.file_name().and_then(|n| n.to_str()),
        Some(PROJECT)
    );
    let _down = DownOnDrop {
        docker: live.docker.clone(),
        host: live.host.clone(),
        folder: project.folder.clone(),
    };
    assert_eq!(
        service.projects(),
        vec![project.clone()],
        "the source is stored"
    );

    // 2. Deploy, as Start project does.
    let run = PlanRun::for_folder(
        &live.engine.id,
        &project.folder,
        vec![PathBuf::from("compose.yaml")],
    );
    {
        let mut state = live.state;
        let actions = live.actions.clone();
        live.dom
            .in_runtime(|| {
                actions.run_for_folder(
                    &mut state,
                    &run,
                    ComposeCommand::Up {
                        services: Vec::new(),
                        build: false,
                    },
                )
            })
            .expect("deploy started");
    }
    assert_eq!(
        live.wait_finished(300).await,
        Ok(()),
        "{:?}",
        live.project_containers()
    );
    assert!(
        live.project_containers()
            .iter()
            .any(|c| c.starts_with(&format!("{PROJECT}-web-1")) && c.ends_with("running")),
        "the project is running: {:?}",
        live.project_containers()
    );
    assert!(serves("deployed from Git by Berth").await, "the app serves");

    // 3. Another clone pushes a third commit to the bare repo.
    let other = tmp.path().join("other");
    git(
        &live.git,
        None,
        &[
            "clone",
            "-q",
            remote.to_str().expect("utf8"),
            other.to_str().expect("utf8"),
        ],
    );
    let page = other.join("site/index.html");
    let mut html = std::fs::read_to_string(&page).expect("page");
    html.push_str(&format!("<p>{MARKER}</p>\n"));
    std::fs::write(&page, html).expect("write page");
    git(
        &live.git,
        Some(&other),
        &["commit", "-q", "-am", "third commit"],
    );
    let new_head = git(&live.git, Some(&other), &["rev-parse", "--short", "HEAD"]);
    git(&live.git, Some(&other), &["push", "-q", "origin", "main"]);

    // 4. A poll shows exactly one new commit and, with Auto-redeploy off, changes nothing.
    let polled = service
        .poll(&project.folder, CancellationToken::new())
        .await
        .expect("poll");
    assert_eq!(polled.incoming.len(), 1);
    assert_eq!(new_commits_label(polled.incoming.len()), "1 new commit");
    assert_eq!(polled.incoming[0].hash, new_head);
    assert_eq!(polled.incoming[0].subject, "third commit");
    assert_eq!(polled.pulled, None, "Auto-redeploy is off (D72)");
    assert_eq!(
        service.projects()[0].commit.as_deref(),
        Some(remote_head.as_str())
    );
    assert!(
        http_get().is_some_and(|body| !body.contains(MARKER)),
        "nothing was redeployed by the poll"
    );

    // 5. Pull and redeploy brings the new commit up.
    let pulled = service
        .pull(&project.folder, CancellationToken::new())
        .await
        .expect("pull");
    assert_eq!(pulled.to.as_deref(), Some(new_head.as_str()));
    assert_eq!(pulled.commits.len(), 1);
    let stored = service.projects().remove(0);
    assert_eq!(stored.commit.as_deref(), Some(new_head.as_str()));
    {
        let state = live.state;
        live.dom
            .in_runtime(|| handle.redeploy(state, &stored))
            .expect("redeploy started");
    }
    assert_eq!(
        live.wait_finished(300).await,
        Ok(()),
        "{:?}",
        live.project_containers()
    );
    assert!(serves(MARKER).await, "the new commit's page is served");
    assert!(
        service
            .check(&project.folder, CancellationToken::new())
            .await
            .expect("check")
            .is_empty(),
        "nothing is left to pull"
    );
}
