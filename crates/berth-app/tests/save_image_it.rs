#![allow(clippy::expect_used)]
//! P8-T4's live check as a test: Save as image on the fixture container `berthfx-web-1` as
//! `berthfx/web-snap:1` through the `ImageStore`, see the image in the engine's list, then remove it.
//!
//! Engine-agnostic: it uses the first running engine discovery finds, and the Docker CLI it shells out
//! to for the final check and the clean-up is pointed at that engine's socket. It touches only the
//! `berthfx/web-snap:1` tag it makes (removed on every exit path); the committed image holds a copy of
//! the fixture's nginx layers and nothing else.
//!
//! Run with `BERTH_KEYRING=memory scripts/it.sh -p berth-app save_image_it` (fixtures up).

use std::path::PathBuf;
use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_app::services::engines::client_map;
use berth_app::services::images::ImageStore;
use berth_app::services::Services;
use berth_app::state::{use_app_state, AppState};
use berth_engine::EngineClient;

const REFERENCE: &str = "berthfx/web-snap:1";

thread_local! {
    static SLOT: std::cell::RefCell<Option<(AppState, ImageStore)>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    let store = ImageStore::new();
    use_hook(move || {
        provide_context(state);
        SLOT.with(|slot| *slot.borrow_mut() = Some((state, store)));
    });
    rsx! { div {} }
}

struct RemoveOnDrop {
    bin: PathBuf,
    host: String,
}

impl RemoveOnDrop {
    fn docker(&self, args: &[&str]) -> std::process::Output {
        std::process::Command::new(&self.bin)
            .env("DOCKER_HOST", &self.host)
            .args(args)
            .output()
            .expect("docker CLI runs")
    }
}

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = self.docker(&["rmi", REFERENCE]);
    }
}

#[tokio::test]
#[ignore = "needs a running engine; run with BERTH_IT=1 via scripts/it.sh"]
async fn save_image_it_commit_berthfx_web_shows_the_image_after_the_refresh() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
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
    let guard = RemoveOnDrop {
        bin: search
            .find("docker")
            .unwrap_or_else(|| PathBuf::from("docker")),
        host: engine.docker_host.clone().expect("docker host"),
    };
    println!("save_image_it: using engine {}", engine.id.0);
    let _ = guard.docker(&["rmi", REFERENCE]);

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

    let web = client
        .containers()
        .await
        .expect("containers")
        .into_iter()
        .find(|c| c.name == "berthfx-web-1")
        .expect("berthfx-web-1 (run scripts/fx-up.sh)");

    let heard = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = heard.clone();
    dom.in_scope(ScopeId::APP, || {
        store.save_as_image(&services, &web, REFERENCE, move |r| {
            *sink.borrow_mut() = Some(r);
        });
    });
    let waited = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        loop {
            if heard.borrow().is_some() {
                return;
            }
            let _ = tokio::time::timeout(std::time::Duration::from_millis(50), dom.wait_for_work())
                .await;
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
        }
    })
    .await;
    assert!(waited.is_ok(), "the commit did not end");
    assert_eq!(*heard.borrow(), Some(Ok(REFERENCE.to_string())));

    // The refresh the store makes lists the new image.
    let listed = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let seen = dom.in_runtime(|| {
                store
                    .by_engine
                    .peek()
                    .get(&engine.id)
                    .is_some_and(|images| {
                        images
                            .iter()
                            .any(|i| i.repo_tags.iter().any(|t| t == REFERENCE))
                    })
            });
            if seen {
                return;
            }
            let _ = tokio::time::timeout(std::time::Duration::from_millis(50), dom.wait_for_work())
                .await;
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
        }
    })
    .await;
    assert!(
        listed.is_ok(),
        "{REFERENCE} did not show in the Images list"
    );
    let out = guard.docker(&["image", "ls", "-q", REFERENCE]);
    assert!(
        !String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        "the CLI sees the image too"
    );
    // The container was paused for the commit and is running again.
    let after = client
        .containers()
        .await
        .expect("containers")
        .into_iter()
        .find(|c| c.name == "berthfx-web-1")
        .expect("web still there");
    assert_eq!(after.state, berth_core::ContainerState::Running);
    println!("save_image_it: {REFERENCE} committed and listed; removing it");
}
