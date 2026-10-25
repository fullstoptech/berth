#![allow(clippy::expect_used)]
//! Live terminal sessions (P3-T6 done-when). Run with
//! `BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only exec_it`
//! from a tree whose fixtures are already up (never `scripts/it.sh` from a
//! worktree: Compose would record the worktree folder).
//!
//! (a) a bash in `berthfx-web-1` lists `/` and follows a resize to 40x120;
//! (b) a container whose image has neither bash nor sh (a scratch image
//! holding one static binary, built here from the engine's own `caddy:2`)
//! ends in the no-shell state. The scratch image and its container carry
//! `dev.berth.fixture=1`, are named `berthfx-noshell`, and are removed
//! afterwards even when an assertion fails.

mod common;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use berth_app::bridge::TermEvent;
use berth_app::services::exec::{
    view_id, Attached, ExecCore, ExecSpec, Kind, Spec, Status, ViewMsg,
};
use berth_engine::EngineClient;

const NOSHELL: &str = "berthfx-noshell";

struct Live {
    client: Arc<dyn EngineClient>,
    core: ExecCore,
    engine: berth_core::EngineId,
    docker: std::path::PathBuf,
    host: String,
}

async fn live() -> Live {
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima_cli = berth_engine::ColimaCli::new(search.clone());
    let engines = berth_engine::discovery::discover(&colima_cli, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default" && e.state == berth_core::EngineState::Running)
        .expect("colima:default running")
        .clone();
    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let clients = berth_app::services::engines::client_map();
    clients
        .lock()
        .expect("clients")
        .insert(engine.id.clone(), Arc::clone(&client));
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let (core, _ui) = ExecCore::new(clients, store, CancellationToken::new());
    Live {
        host: client.docker_host().to_string(),
        client,
        core,
        engine: engine.id,
        docker: search.find("docker").expect("docker on PATH"),
    }
}

impl Live {
    fn docker(&self, args: &[&str]) -> std::process::Output {
        std::process::Command::new(&self.docker)
            .env("DOCKER_HOST", &self.host)
            .args(args)
            .output()
            .expect("docker runs")
    }

    async fn spec_for(&self, name: &str) -> Spec {
        let container = self
            .client
            .containers()
            .await
            .expect("list")
            .into_iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} exists"));
        Spec {
            engine: self.engine.clone(),
            project: None,
            kind: Kind::Exec(ExecSpec {
                container: container.id,
                name: container.name,
                label: name.to_string(),
                image: container.image.reference,
                shell: None,
                user: None,
                workdir: None,
                wait_running: false,
            }),
        }
    }
}

/// Everything a session needs from a test's side: the tab, its mounted
/// "host" (as a bridge id) and the channel the pane would read.
struct Pane {
    tab: Uuid,
    id: String,
    rx: UnboundedReceiver<ViewMsg>,
    output: Vec<u8>,
    status: Status,
}

impl Pane {
    fn mount(core: &ExecCore, spec: Spec) -> Self {
        let tab = Uuid::now_v7();
        core.register(tab, spec);
        let Attached { viewer, view, rx } = core.attach(tab).expect("attached");
        let id = view_id(tab, viewer);
        core.term_event(TermEvent::Ready { id: id.clone() });
        Self {
            tab,
            id,
            rx,
            output: Vec::new(),
            status: view.status,
        }
    }

    /// Read until `done` holds for what has arrived, or fail after 20 s.
    async fn until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while !done(self) {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let message = tokio::time::timeout(left, self.rx.recv())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "timed out waiting for {what}; status {:?}, output {:?}",
                        self.status,
                        String::from_utf8_lossy(&self.output)
                    )
                })
                .expect("channel open");
            match message {
                ViewMsg::Output(bytes) | ViewMsg::Replay(bytes) => self.output.extend(bytes),
                ViewMsg::Snapshot(view) => self.status = view.status,
                ViewMsg::Clear => self.output.clear(),
            }
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn exec_it_bash_lists_the_root_and_follows_a_resize() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    common::settle_fixture().await;
    let live = live().await;
    let spec = live.spec_for("berthfx-web-1").await;
    let mut pane = Pane::mount(&live.core, spec);
    pane.until("a live bash", |p| p.status.is_live()).await;

    live.core.term_event(TermEvent::Input {
        id: pane.id.clone(),
        data: b"ls /\n".to_vec(),
    });
    pane.until("the listing of /", |p| p.text().contains("usr"))
        .await;

    // The page reports a new size: 120 columns by 40 rows.
    live.core.term_event(TermEvent::Resize {
        id: pane.id.clone(),
        cols: 120,
        rows: 40,
    });
    live.core.term_event(TermEvent::Input {
        id: pane.id.clone(),
        data: b"stty size\n".to_vec(),
    });
    pane.until("stty size to say 40 120", |p| p.text().contains("40 120"))
        .await;

    // berthfx-web-1 is nginx:alpine: no bash, so the probe fell back to sh.
    let view = live.core.view(pane.tab).expect("view");
    assert_eq!(view.shell_used.as_deref(), Some("sh"));
    assert_eq!(view.title, "berthfx-web-1 \u{b7} sh");
    live.core.close(pane.tab);
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn exec_it_bash_is_the_first_choice_where_the_image_has_it() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    common::settle_fixture().await;
    let live = live().await;
    // berthfx-db-1 is a Debian image; the user picks nothing.
    let spec = live.spec_for("berthfx-db-1").await;
    let mut pane = Pane::mount(&live.core, spec);
    pane.until("a live shell", |p| p.status.is_live()).await;
    let view = live.core.view(pane.tab).expect("view");
    assert_eq!(view.shell_used.as_deref(), Some("bash"));
    assert_eq!(view.title, "berthfx-db-1 \u{b7} bash");
    live.core.close(pane.tab);
}

/// Removes the scratch container and image when the test ends, however.
struct Cleanup<'a>(&'a Live);

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let _ = self.0.docker(&["rm", "-f", NOSHELL]);
        let _ = self.0.docker(&["rmi", "-f", &format!("{NOSHELL}:test")]);
        let _ = self.0.docker(&["rm", "-f", "berthfx-noshell-src"]);
    }
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn exec_it_an_image_without_a_shell_ends_in_the_no_shell_state() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let live = live().await;
    let _cleanup = Cleanup(&live);
    // Leftovers of an aborted run.
    drop(Cleanup(&live));

    // One static binary from an image the engine already has.
    let dir = tempfile::tempdir().expect("temp dir");
    let label = "dev.berth.fixture=1";
    let created = live.docker(&[
        "create",
        "--label",
        label,
        "--name",
        "berthfx-noshell-src",
        "caddy:2",
    ]);
    assert!(created.status.success(), "{created:?}");
    let copied = live.docker(&[
        "cp",
        "berthfx-noshell-src:/usr/bin/caddy",
        &dir.path().join("caddy").to_string_lossy(),
    ]);
    assert!(copied.status.success(), "{copied:?}");
    live.docker(&["rm", "-f", "berthfx-noshell-src"]);
    std::fs::write(
        dir.path().join("Dockerfile"),
        "FROM scratch\nLABEL dev.berth.fixture=1\nCOPY caddy /caddy\n\
         ENTRYPOINT [\"/caddy\"]\nCMD [\"file-server\", \"--listen\", \":8080\"]\n",
    )
    .expect("Dockerfile");
    let built = live.docker(&[
        "build",
        "-t",
        &format!("{NOSHELL}:test"),
        &dir.path().to_string_lossy(),
    ]);
    assert!(built.status.success(), "{built:?}");
    let run = live.docker(&[
        "run",
        "-d",
        "--label",
        label,
        "--name",
        NOSHELL,
        &format!("{NOSHELL}:test"),
    ]);
    assert!(run.status.success(), "{run:?}");

    // Give the entrypoint a moment to be the running process.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let spec = live.spec_for(NOSHELL).await;
    let mut pane = Pane::mount(&live.core, spec);
    pane.until("the no-shell state", |p| {
        matches!(p.status, Status::NoShell { .. } | Status::Failed(_))
    })
    .await;
    assert_eq!(
        pane.status,
        Status::NoShell {
            tried: vec!["bash".to_string(), "sh".to_string()]
        }
    );
    // The tab shows the name alone, as in the frame.
    let view = live.core.view(pane.tab).expect("view");
    assert_eq!(view.title, NOSHELL);
    live.core.close(pane.tab);
}
