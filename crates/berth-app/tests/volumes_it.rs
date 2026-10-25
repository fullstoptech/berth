#![allow(clippy::expect_used)]
//! The by-hand checks of P4-T4's done-when, as live tests:
//!
//! * the fixture's `pgdata` volume shows a size, browsing it lists
//!   `PG_VERSION`, and Back up writes a `.tar.gz` where
//!   `tar -tzf <file> | grep -c PG_VERSION` counts at least 1, without
//!   stopping `berthfx-db-1`;
//! * creating `berthfx-vol` and deleting it with the typed name works.
//!
//! Run with `BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only
//! volumes_it` while the berthfx fixtures are up. Only resources labelled
//! `dev.berth.fixture=1` are touched; Colima is never started or stopped.

mod common;

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::actions::{apply_finished, ActionsService};
use berth_app::services::confirm;
use berth_app::services::engines::client_map;
use berth_app::services::files::{EngineFiles, FileSource};
use berth_app::services::volumes::{self, VolumeOp};
use berth_app::services::{files, volume_backup};
use berth_app::state::{use_app_state, AppState};
use berth_core::{ContainerState, FileTarget, VolumeSpec};
use berth_engine::EngineClient;

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

const NEW_VOLUME: &str = "berthfx-vol";

struct Live {
    engine: berth_core::Engine,
    client: Arc<dyn EngineClient>,
    docker: std::path::PathBuf,
    search: berth_sys::SearchPath,
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
    let docker = search
        .find("docker")
        .unwrap_or_else(|| std::path::PathBuf::from("docker"));
    Live {
        engine,
        client,
        docker,
        search,
    }
}

fn docker(live: &Live, args: &[&str]) -> std::process::Output {
    std::process::Command::new(&live.docker)
        .env(
            "DOCKER_HOST",
            live.engine.docker_host.clone().expect("docker host"),
        )
        .args(args)
        .output()
        .expect("docker CLI runs")
}

/// Removes the throwaway volume when the test ends, whatever happened
/// (fixture label checked first).
struct Cleanup {
    docker: std::path::PathBuf,
    host: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let out = std::process::Command::new(&self.docker)
            .env("DOCKER_HOST", &self.host)
            .args([
                "volume",
                "ls",
                "-q",
                "--filter",
                "label=dev.berth.fixture=1",
                "--filter",
                &format!("name=^{NEW_VOLUME}$"),
            ])
            .output();
        if let Ok(out) = out {
            if String::from_utf8_lossy(&out.stdout).trim() == NEW_VOLUME {
                let _ = std::process::Command::new(&self.docker)
                    .env("DOCKER_HOST", &self.host)
                    .args(["volume", "rm", NEW_VOLUME])
                    .output();
            }
        }
    }
}

fn skip() -> bool {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return true;
    }
    false
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn volumes_it_pgdata_shows_a_size_browses_to_pg_version_and_backs_up() {
    if skip() {
        return;
    }
    common::settle_fixture().await;
    let live = live().await;

    // The fixture's pgdata (compose prefixes the project: berthfx_berthfx_pgdata).
    let listed = live.client.volumes().await.expect("volumes");
    let pgdata = listed
        .iter()
        .find(|v| {
            v.labels.get("dev.berth.fixture").map(String::as_str) == Some("1")
                && v.name.ends_with("pgdata")
        })
        .expect("the berthfx pgdata volume")
        .clone();

    // A size, from system df.
    let sizes = volumes::fetch_sizes(vec![Arc::clone(&live.client)]).await;
    let size = sizes.get(&pgdata.name).copied().expect("pgdata has a size");
    assert!(size.as_u64() > 0, "pgdata is not empty: {size}");

    // The helper image is there (this test never pulls); browsing lists PG_VERSION.
    assert!(
        volumes::helper_present(live.client.as_ref())
            .await
            .expect("images list"),
        "alpine:3 is expected on this engine"
    );
    let source: Arc<dyn FileSource> = Arc::new(EngineFiles::for_target(
        Arc::clone(&live.client),
        FileTarget::Volume(pgdata.name.clone()),
    ));
    let entries = files::list_dir(
        Arc::clone(&source),
        "/".to_string(),
        CancellationToken::new(),
    )
    .await
    .expect("list the volume root");
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"PG_VERSION"), "{names:?}");

    // Back up to a temp file; the database keeps running.
    let before = live.client.containers().await.expect("containers");
    let db_before = before
        .iter()
        .find(|c| c.name == "berthfx-db-1")
        .expect("berthfx-db-1");
    assert_eq!(db_before.state, ContainerState::Running);
    let started = db_before.started;

    let dir = tempfile::tempdir().expect("temp dir");
    let dest = dir.path().join("pgdata.tar.gz");
    let report = volume_backup::backup(Arc::clone(&source), dest.clone(), CancellationToken::new())
        .await
        .expect("backup");
    let counted = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "tar -tzf '{}' | grep -c PG_VERSION",
            dest.display()
        ))
        .output()
        .expect("tar runs");
    let count: u32 = String::from_utf8_lossy(&counted.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    println!(
        "volumes_it: volume={} size={size} entries={} backup_files={} backup_bytes={} tar_grep_count={count}",
        pgdata.name,
        names.len(),
        report.files,
        report.bytes
    );
    assert!(count >= 1, "tar -tzf lists PG_VERSION: {count}");
    assert!(report.files > 0);

    // The database was not stopped, and no helper container was left behind.
    let after = live.client.containers().await.expect("containers again");
    let db_after = after
        .iter()
        .find(|c| c.name == "berthfx-db-1")
        .expect("berthfx-db-1 still exists");
    assert_eq!(db_after.state, ContainerState::Running);
    assert_eq!(db_after.started, started, "it was not restarted");
    // Only helpers this test made are judged; one that was already on the
    // engine before it started is none of its business (AD28).
    let helpers = |all: &[berth_core::Container]| -> Vec<String> {
        all.iter()
            .filter(|c| c.name.starts_with("berth-helper-"))
            .map(|c| c.name.clone())
            .collect()
    };
    let before_helpers = helpers(&before);
    let leftover: Vec<String> = helpers(&after)
        .into_iter()
        .filter(|name| !before_helpers.contains(name))
        .collect();
    assert!(leftover.is_empty(), "helpers left behind: {leftover:?}");
    drop(live.search);
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn volumes_it_create_berthfx_vol_then_delete_it_with_the_typed_name() {
    if skip() {
        return;
    }
    common::settle_fixture().await;
    let live = live().await;
    let host = live.engine.docker_host.clone().expect("docker host");
    let _cleanup = Cleanup {
        docker: live.docker.clone(),
        host,
    };
    let _ = docker(&live, &["volume", "rm", NEW_VOLUME]);

    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let mut state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");

    let clients = client_map();
    clients
        .lock()
        .expect("clients")
        .insert(live.engine.id.clone(), Arc::clone(&live.client));
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let (service, mut finished) = ActionsService::new(
        clients,
        store,
        Arc::new(berth_engine::CliCompose::new(live.search.clone())),
        Arc::new(berth_engine::MockColima::bundled()),
        CancellationToken::new(),
    );

    // Create, the way the sheet does: the sheet's spec plus the fixture label.
    let mut spec = volumes::create_spec(NEW_VOLUME, "local", "").expect("a valid spec");
    spec.labels = [("dev.berth.fixture".to_string(), "1".to_string())]
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    let VolumeSpec { name, .. } = spec.clone();
    assert_eq!(name, NEW_VOLUME);
    dom.in_runtime(|| {
        service
            .volume(
                &mut state,
                &live.engine.id,
                NEW_VOLUME,
                VolumeOp::Create(spec),
            )
            .expect("create starts");
    });
    let created = tokio::time::timeout(std::time::Duration::from_secs(30), finished.recv())
        .await
        .expect("the create finishes")
        .expect("channel open");
    assert_eq!(created.result, Ok(()));
    assert_eq!(created.message.as_deref(), Some("Created 1 volume"));
    dom.in_runtime(|| apply_finished(&mut state, created));

    let listed = live.client.volumes().await.expect("volumes");
    let made = listed
        .iter()
        .find(|v| v.name == NEW_VOLUME)
        .expect("the volume exists")
        .clone();
    assert_eq!(
        made.labels.get("dev.berth.fixture").map(String::as_str),
        Some("1"),
        "refusing to delete a volume without the fixture label"
    );

    // The delete dialog for it: typed name, from the one builder.
    let containers = live.client.containers().await.expect("containers");
    dom.in_runtime(|| {
        state.engines.set(vec![live.engine.clone()]);
        state.containers.set(containers.clone());
        state.volumes.set(listed.clone());
    });
    let model = dom
        .in_runtime(|| confirm::delete_volume_model(&state, NEW_VOLUME))
        .expect("a delete dialog for berthfx-vol");
    assert_eq!(model.title, "Delete volume berthfx-vol?");
    assert_eq!(model.effect.typed_name.as_deref(), Some(NEW_VOLUME));
    let button = confirm::button_label(&model.effect);
    assert_eq!(button, "Delete volume");
    assert!(!berth_app::ui::confirm_dialog::name_matches(
        &model.effect,
        "berthfx"
    ));
    assert!(berth_app::ui::confirm_dialog::name_matches(
        &model.effect,
        NEW_VOLUME
    ));

    let launched = dom.in_runtime(|| confirm::run(&mut state, &service, &model, false));
    assert_eq!(launched.refused, Vec::<String>::new());
    assert_eq!(launched.keys.len(), 1);
    let done = tokio::time::timeout(std::time::Duration::from_secs(30), finished.recv())
        .await
        .expect("the delete finishes")
        .expect("channel open");
    assert_eq!(done.result, Ok(()), "the engine removed it");
    dom.in_runtime(|| apply_finished(&mut state, done));

    let pending = dom.in_runtime(|| state.pending.peek().clone());
    let succeeded = confirm::finished_count(&pending, &launched.keys).expect("nothing runs now");
    let result = confirm::result_label(&model.effect, succeeded);
    let left = docker(
        &live,
        &[
            "volume",
            "ls",
            "-q",
            "--filter",
            &format!("name=^{NEW_VOLUME}$"),
        ],
    );
    let left = String::from_utf8_lossy(&left.stdout).trim().to_string();
    println!(
        "volumes_it: title={:?} button={button:?} result={result:?} left={left:?}",
        model.title
    );
    assert!(left.is_empty(), "docker volume ls still lists it");
    assert_eq!(result, "Deleted 1 volume");
}
