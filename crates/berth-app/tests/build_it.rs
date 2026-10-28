#![allow(clippy::expect_used)]
//! P8-T3's live check as a test: build `fixtures/folders/dockerfile-only`
//! as `127.0.0.1:56500/berthfx/app:1`, push it to the fixture registry
//! (logged in with the fixture password), export it to a tar, remove the
//! local image, import the tar back and see the image again, then remove
//! the images it made.
//!
//! Engine-agnostic: it uses the first running engine discovery finds
//! (not `colima:default`), and the Docker CLI it shells out to is pointed at
//! that engine's socket. It brings the `berthfx-registry` project up and
//! down itself (AD33: a `berthfx-*` project from the fixture folder, host
//! port 56500) and touches nothing else: the one image tag it makes is
//! removed on every exit path.
//!
//! Run with `BERTH_KEYRING=memory scripts/it.sh -p berth-app build_it`.
//! The base image (`nginx:1.27-alpine`) is pulled by the build if the
//! engine lacks it, and stays.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::engines::client_map;
use berth_app::services::images::{BuildPhase, ImageStore, PushPhase};
use berth_app::services::registries::{secret_key, RegistriesService};
use berth_app::services::Services;
use berth_app::state::{use_app_state, AppState};
use berth_core::{BuildContext, BuildSpec, EngineId};
use berth_engine::EngineClient;

const REGISTRY: &str = "127.0.0.1:56500";
const TAG: &str = "127.0.0.1:56500/berthfx/app:1";
const REGISTRY_USER: &str = "berthfx";

/// The fixture password, read from the one place it may live
/// (`fixtures/`, PLAN secrets rule) instead of copied here.
fn fixture_password() -> String {
    let env_file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/compose/berthfx/.env"
    );
    let text = std::fs::read_to_string(env_file).expect("fixture .env");
    text.lines()
        .find_map(|line| line.strip_prefix("BERTHFX_DB_PASSWORD="))
        .expect("fixture password key")
        .trim()
        .to_string()
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

thread_local! {
    static SLOT: std::cell::RefCell<Option<(AppState, ImageStore)>> =
        const { std::cell::RefCell::new(None) };
}

/// The App root's part: `AppState` and a registries service in context, as
/// the store's `build` and `push` find them, with the fixture registry
/// logged in. The password goes to an in-memory secret store, never the
/// Keychain.
#[component]
fn Harness() -> Element {
    let mut state = use_app_state();
    let store = ImageStore::new();
    use_hook(move || {
        let secrets: Arc<dyn berth_store::SecretStore> =
            Arc::new(berth_store::InMemorySecretStore::default());
        secrets
            .set(
                &secret_key(REGISTRY),
                &secrecy::SecretString::from(fixture_password()),
            )
            .expect("store the password");
        let registries = RegistriesService::new(
            Arc::new(berth_store::Store::open_in_memory().expect("store")),
            secrets,
        );
        state.registries.set(vec![berth_core::Registry {
            id: REGISTRY.to_string(),
            name: "berthfx".to_string(),
            url: url::Url::parse(&format!("http://{REGISTRY}")).expect("url"),
            username: Some(REGISTRY_USER.to_string()),
            status: berth_core::RegistryStatus::LoggedIn,
        }]);
        provide_context(state);
        provide_context(Some(registries));
        SLOT.with(|slot| *slot.borrow_mut() = Some((state, store)));
    });
    rsx! { div {} }
}

/// Runs the Docker CLI against one engine's socket.
#[derive(Clone)]
struct Cli {
    bin: PathBuf,
    host: String,
}

impl Cli {
    fn run(&self, args: &[&str]) -> std::process::Output {
        std::process::Command::new(&self.bin)
            .env("DOCKER_HOST", &self.host)
            .args(args)
            .output()
            .expect("docker CLI runs")
    }

    fn registry(&self, args: &[&str]) {
        let dir = fixtures().join("compose/berthfx-registry");
        let file = dir.join("compose.yaml");
        let output = self.run(
            &[
                &["compose", "--project-directory", dir.to_str().expect("dir")][..],
                &["-f", file.to_str().expect("file"), "-p", "berthfx-registry"],
                args,
            ]
            .concat(),
        );
        assert!(
            output.status.success(),
            "compose {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn has_image(&self, tag: &str) -> bool {
        !String::from_utf8_lossy(&self.run(&["image", "ls", "-q", tag]).stdout)
            .trim()
            .is_empty()
    }
}

/// Takes the registry down and removes the one tag this test makes, on
/// every exit path.
struct CleanOnDrop(Cli);

impl Drop for CleanOnDrop {
    fn drop(&mut self) {
        let _ = self.0.run(&["rmi", TAG]);
        let _ = self.0.run(&[
            "compose",
            "--project-directory",
            fixtures()
                .join("compose/berthfx-registry")
                .to_str()
                .unwrap_or_default(),
            "-p",
            "berthfx-registry",
            "down",
            "-v",
        ]);
    }
}

struct Rig {
    dom: VirtualDom,
    store: ImageStore,
    services: Services,
    client: Arc<dyn EngineClient>,
    engine: EngineId,
    cli: Cli,
}

impl Rig {
    /// Drive the dom until `done` says so (`secs` at most).
    async fn until(&mut self, what: &str, secs: u64, mut done: impl FnMut(&VirtualDom) -> bool) {
        let waited = tokio::time::timeout(std::time::Duration::from_secs(secs), async {
            loop {
                if done(&self.dom) {
                    return;
                }
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(50),
                    self.dom.wait_for_work(),
                )
                .await;
                self.dom.render_immediate(&mut dioxus::core::NoOpMutations);
            }
        })
        .await;
        assert!(waited.is_ok(), "{what} did not happen in {secs} s");
    }

    /// The engine's image list, asked again while the machine is busy.
    async fn tags(&self) -> Vec<String> {
        let mut last = None;
        for _ in 0..6 {
            match self.client.images().await {
                Ok(images) => return images.into_iter().flat_map(|i| i.repo_tags).collect(),
                Err(err) => {
                    last = Some(err);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
        panic!("images: {last:?}");
    }
}

async fn rig() -> Option<(Rig, CleanOnDrop)> {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return None;
    }
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let (_state, store) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the state");
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima_cli = berth_engine::ColimaCli::new(search.clone());
    // The first engine that is running and has a socket, whatever its id.
    let mut found = None;
    for _ in 0..6 {
        let engines = berth_engine::discovery::discover(&colima_cli, &[]).await;
        found = engines
            .into_iter()
            .find(|e| e.state == berth_core::EngineState::Running && e.docker_host.is_some());
        if found.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
    let engine = found.expect("a running engine");
    let cli = Cli {
        bin: search
            .find("docker")
            .unwrap_or_else(|| PathBuf::from("docker")),
        host: engine.docker_host.clone().expect("docker host"),
    };
    println!("build_it: using engine {}", engine.id.0);
    let guard = CleanOnDrop(cli.clone());
    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let clients = client_map();
    clients
        .lock()
        .expect("clients")
        .insert(engine.id.clone(), Arc::clone(&client));
    let services = Services {
        store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
        clients,
        cancel: CancellationToken::new(),
    };
    // A leftover of an earlier, interrupted run (our own tag only).
    let _ = cli.run(&["rmi", TAG]);
    cli.registry(&["up", "-d", "--wait"]);
    Some((
        Rig {
            dom,
            store,
            services,
            client,
            engine: engine.id.clone(),
            cli,
        },
        guard,
    ))
}

#[tokio::test]
#[ignore = "needs a running engine; run with BERTH_IT=1 via scripts/it.sh"]
async fn build_it_build_push_export_remove_import_round_trip() {
    let Some((mut rig, _guard)) = rig().await else {
        return;
    };
    let store = rig.store;
    let services = rig.services.clone();
    let engine = rig.engine.clone();

    // Build.
    let spec = BuildSpec {
        context: BuildContext::Folder(fixtures().join("folders/dockerfile-only")),
        dockerfile: None,
        tags: vec![TAG.to_string()],
        build_args: Vec::new(),
        platform: None,
    };
    let started = rig
        .dom
        .in_scope(ScopeId::APP, || store.build(&services, &engine, spec));
    assert_eq!(started, Ok(()));
    rig.until("the build to end", 300, |dom| {
        dom.in_runtime(|| store.builds.peek().iter().all(|b| !b.is_running()))
    })
    .await;
    let build = rig.dom.in_runtime(|| store.builds.peek()[0].clone());
    println!(
        "build_it: build phase={:?} step={:?} lines={}",
        build.phase,
        build.step,
        build.lines.len()
    );
    assert_eq!(build.phase, BuildPhase::Done, "{:?}", build.failure);
    assert_eq!(build.step.map(|(i, t)| i == t), Some(true), "{build:?}");
    assert!(rig.cli.has_image(TAG), "the CLI sees the built image");
    rig.until("the list to have the image", 30, |dom| {
        dom.in_runtime(|| {
            store.by_engine.peek().get(&engine).is_some_and(|images| {
                images
                    .iter()
                    .any(|i| i.repo_tags.contains(&TAG.to_string()))
            })
        })
    })
    .await;

    // Push, with the fixture registry's login.
    let started = rig
        .dom
        .in_scope(ScopeId::APP, || store.push(&services, &engine, TAG));
    assert_eq!(started, Ok(()));
    rig.until("the push to end", 120, |dom| {
        dom.in_runtime(|| store.pushes.peek().iter().all(|p| !p.is_running()))
    })
    .await;
    let push = rig.dom.in_runtime(|| store.pushes.peek()[0].clone());
    println!(
        "build_it: push phase={:?} note={:?} digest={:?}",
        push.phase, push.note, push.digest
    );
    assert_eq!(push.phase, PushPhase::Done);
    assert!(
        push.digest
            .as_deref()
            .is_some_and(|d| d.starts_with("sha256:")),
        "{push:?}"
    );
    let password = secrecy::SecretString::from(fixture_password());
    let tags = berth_registry::RegistryClient::new()
        .tags(
            &url::Url::parse(&format!("http://{REGISTRY}")).expect("url"),
            "berthfx/app",
            Some((REGISTRY_USER, &password)),
        )
        .await
        .expect("the registry lists the repository");
    assert_eq!(tags, ["1"], "the registry has the pushed tag");

    // Export, remove, import.
    let dir = tempfile::tempdir().expect("dir");
    let tar = dir.path().join("app.tar");
    let exported = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = exported.clone();
    let (target, name) = (tar.clone(), TAG.to_string());
    rig.dom.in_scope(ScopeId::APP, || {
        store.export(&services, &engine, &name, target, move |r| {
            *sink.borrow_mut() = Some(r);
        });
    });
    rig.until("the export", 120, |_| exported.borrow().is_some())
        .await;
    let size = exported
        .borrow_mut()
        .take()
        .expect("a result")
        .expect("exported");
    println!("build_it: exported {size} bytes");
    assert!(size > 0);
    assert_eq!(std::fs::metadata(&tar).expect("tar").len(), size);
    assert!(
        !dir.path().join("app.tar.part").exists(),
        "no partial file left"
    );

    rig.client
        .remove_image(TAG, false)
        .await
        .expect("remove the local image");
    assert!(!rig.tags().await.contains(&TAG.to_string()));
    assert!(!rig.cli.has_image(TAG));

    let imported = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = imported.clone();
    let source = tar.clone();
    rig.dom.in_scope(ScopeId::APP, || {
        store.import(&services, &engine, source, move |r| {
            *sink.borrow_mut() = Some(r);
        });
    });
    rig.until("the import", 120, |_| imported.borrow().is_some())
        .await;
    let names = imported
        .borrow_mut()
        .take()
        .expect("a result")
        .expect("imported");
    println!("build_it: imported {names:?}");
    assert_eq!(names, [TAG]);
    assert!(rig.tags().await.contains(&TAG.to_string()));
    assert!(rig.cli.has_image(TAG), "the CLI sees the imported image");

    // Remove what this test made (the guard does it again on a failure).
    rig.client
        .remove_image(TAG, false)
        .await
        .expect("remove the image");
    assert!(!rig.tags().await.contains(&TAG.to_string()));
}
