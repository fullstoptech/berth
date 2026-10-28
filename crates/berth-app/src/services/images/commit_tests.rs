//! P8-T4 tests for Save as image: what the field accepts, and the store's commit over the mock
//! engine (the image shows up after the refresh, the pending entry and the Activity entry, a
//! refusal, and cancellation).

use std::sync::Arc;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_core::{Container, EngineId, Outcome};
use berth_engine::EngineClient as _;

use super::*;
use crate::state::{use_app_state, AppState, PendingKey};

fn default_engine() -> EngineId {
    EngineId("colima:default".to_string())
}

// ---------------------------------------------------------------------
// Pure
// ---------------------------------------------------------------------

#[test]
fn save_as_image_reference_is_a_repo_and_a_tag() {
    let ok = |text: &str| commit_reference(text).expect(text);
    assert_eq!(
        ok("berthfx/web-snap:1"),
        ("berthfx/web-snap".to_string(), "1".to_string())
    );
    assert_eq!(
        ok(" my-app "),
        ("my-app".to_string(), "latest".to_string()),
        "no tag means latest, as docker commit reads it"
    );
    assert_eq!(
        ok("127.0.0.1:56500/berthfx/app:2"),
        ("127.0.0.1:56500/berthfx/app".to_string(), "2".to_string()),
        "a registry port is not a tag"
    );
}

#[test]
fn save_as_image_reference_refuses_what_docker_would() {
    for bad in [
        "",
        "My Image:1",
        "UPPER:1",
        "name with space:1",
        "app:",
        "app:bad tag",
        "app@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ] {
        assert!(commit_reference(bad).is_err(), "{bad:?} was accepted");
    }
    let digest = commit_reference(
        "app@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .expect_err("digest");
    assert!(digest.contains("not a digest"), "{digest}");
}

#[test]
fn save_as_image_the_suggested_name_is_always_acceptable() {
    for name in [
        "shop-api-web-1",
        "/Shop_API.Web",
        "Weird Name!!",
        "---",
        "",
        "ünï",
    ] {
        let suggestion = default_reference(name);
        assert!(
            commit_reference(&suggestion).is_ok(),
            "{name:?} suggested {suggestion:?}"
        );
    }
    assert_eq!(default_reference("shop-api-web-1"), "shop-api-web-1-snap:1");
}

// ---------------------------------------------------------------------
// The store over the mock engine
// ---------------------------------------------------------------------

thread_local! {
    static SLOT: std::cell::RefCell<Option<(AppState, ImageStore)>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    let store = ImageStore::new();
    use_hook(move || {
        // The store finds the app state in context, as it does under the App root.
        provide_context(state);
        SLOT.with(|slot| *slot.borrow_mut() = Some((state, store)));
    });
    rsx! { div {} }
}

fn captured() -> (VirtualDom, AppState, ImageStore) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let (state, store) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the store");
    (dom, state, store)
}

fn services_with_mock() -> (Services, Arc<berth_engine::MockEngine>) {
    let engine = Arc::new(
        berth_engine::MockEngine::from_world(berth_engine::MockWorld::bundled(), &default_engine())
            .expect("engine"),
    );
    let clients = crate::services::engines::client_map();
    if let Ok(mut map) = clients.lock() {
        map.insert(
            default_engine(),
            engine.clone() as Arc<dyn berth_engine::EngineClient>,
        );
    }
    (
        Services {
            store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
            clients,
            cancel: CancellationToken::new(),
        },
        engine,
    )
}

async fn until(dom: &mut VirtualDom, what: &str, mut done: impl FnMut(&VirtualDom) -> bool) {
    let waited = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if done(dom) {
                return;
            }
            let _ = tokio::time::timeout(std::time::Duration::from_millis(50), dom.wait_for_work())
                .await;
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
        }
    })
    .await;
    assert!(waited.is_ok(), "{what} did not happen");
}

async fn a_container(engine: &berth_engine::MockEngine) -> Container {
    engine
        .containers()
        .await
        .expect("containers")
        .into_iter()
        .find(|c| c.name == "pg-scratch")
        .expect("pg-scratch")
}

fn commit_key(reference: &str) -> PendingKey {
    PendingKey {
        subject: berth_core::Subject::Image(reference.to_string()),
        action: COMMIT_ACTION,
    }
}

fn run_commit(
    dom: &VirtualDom,
    store: ImageStore,
    services: &Services,
    container: &Container,
    reference: &str,
) -> std::rc::Rc<std::cell::RefCell<Option<Result<String, String>>>> {
    let heard = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = heard.clone();
    dom.in_scope(ScopeId::APP, || {
        store.save_as_image(services, container, reference, move |r| {
            *sink.borrow_mut() = Some(r);
        });
    });
    heard
}

#[tokio::test]
async fn save_as_image_commits_the_container_and_the_image_appears_after_the_refresh() {
    let (mut dom, state, store) = captured();
    let (services, engine) = services_with_mock();
    let container = a_container(&engine).await;
    let before = engine.images().await.expect("images").len();

    let heard = run_commit(&dom, store, &services, &container, "berthfx/web-snap:1");
    // Pending at once, with the words the activity strip shows.
    let pending = dom.in_runtime(|| {
        state
            .pending
            .peek()
            .get(&commit_key("berthfx/web-snap:1"))
            .cloned()
    });
    assert_eq!(
        pending,
        Some(crate::state::PendingState::Running {
            label: "Saving pg-scratch as berthfx/web-snap:1".to_string()
        })
    );
    until(&mut dom, "the commit result", |_| heard.borrow().is_some()).await;
    assert_eq!(*heard.borrow(), Some(Ok("berthfx/web-snap:1".to_string())));
    until(&mut dom, "the refreshed list", |dom| {
        dom.in_runtime(|| {
            store
                .by_engine
                .peek()
                .get(&default_engine())
                .is_some_and(|images| {
                    images
                        .iter()
                        .any(|i| i.repo_tags.iter().any(|t| t == "berthfx/web-snap:1"))
                })
        })
    })
    .await;
    assert_eq!(engine.images().await.expect("images").len(), before + 1);
    assert!(
        dom.in_runtime(|| !state
            .pending
            .peek()
            .contains_key(&commit_key("berthfx/web-snap:1"))),
        "the pending entry is gone"
    );
    let entries = services
        .store
        .activity_query(&berth_store::ActivityFilter::default(), None, 10)
        .expect("activity");
    let entry = entries
        .iter()
        .find(|e| e.action == COMMIT_ACTION)
        .expect("a commit entry");
    assert_eq!(entry.subject_name, "berthfx/web-snap:1");
    assert_eq!(entry.outcome, Outcome::Ok);
}

#[tokio::test]
async fn save_as_image_without_a_tag_saves_latest() {
    let (mut dom, _state, store) = captured();
    let (services, engine) = services_with_mock();
    let container = a_container(&engine).await;
    let heard = run_commit(&dom, store, &services, &container, "berthfx/untagged");
    until(&mut dom, "the commit result", |_| heard.borrow().is_some()).await;
    assert_eq!(
        *heard.borrow(),
        Some(Ok("berthfx/untagged:latest".to_string()))
    );
    let images = engine.images().await.expect("images");
    assert!(images
        .iter()
        .any(|i| i.repo_tags.iter().any(|t| t == "berthfx/untagged:latest")));
}

#[tokio::test]
async fn save_as_image_a_refused_name_never_reaches_the_engine() {
    let (dom, _state, store) = captured();
    let (services, engine) = services_with_mock();
    let container = a_container(&engine).await;
    let before = engine.images().await.expect("images").len();
    let heard = run_commit(&dom, store, &services, &container, "Not Valid:1");
    // Refused on the spot: no waiting for the I/O runtime.
    let result = heard.borrow().clone().expect("answered at once");
    assert!(result.is_err(), "{result:?}");
    assert_eq!(engine.images().await.expect("images").len(), before);
}

#[tokio::test]
async fn save_as_image_reports_a_container_the_engine_lacks_and_records_the_failure() {
    let (mut dom, state, store) = captured();
    let (services, engine) = services_with_mock();
    let mut ghost = a_container(&engine).await;
    ghost.id = berth_core::ContainerId("ghost0000000".to_string());
    let heard = run_commit(&dom, store, &services, &ghost, "berthfx/ghost:1");
    until(&mut dom, "the commit result", |_| heard.borrow().is_some()).await;
    let result = heard.borrow().clone().expect("a result");
    assert!(result.is_err(), "{result:?}");
    assert!(dom.in_runtime(|| !state
        .pending
        .peek()
        .contains_key(&commit_key("berthfx/ghost:1"))));
    let entries = services
        .store
        .activity_query(&berth_store::ActivityFilter::default(), None, 10)
        .expect("activity");
    let entry = entries
        .iter()
        .find(|e| e.action == COMMIT_ACTION)
        .expect("a commit entry");
    assert!(
        matches!(entry.outcome, Outcome::Failed { .. }),
        "{:?}",
        entry.outcome
    );
}

#[tokio::test]
async fn save_as_image_cancelled_work_says_cancelled_and_records_nothing() {
    let (mut dom, _state, store) = captured();
    let (services, engine) = services_with_mock();
    let container = a_container(&engine).await;
    let before = engine.images().await.expect("images").len();
    // The app is shutting down: every operation's token is a child of this one.
    services.cancel.cancel();
    let heard = run_commit(&dom, store, &services, &container, "berthfx/late:1");
    until(&mut dom, "the commit result", |_| heard.borrow().is_some()).await;
    assert_eq!(*heard.borrow(), Some(Err("Cancelled".to_string())));
    assert_eq!(engine.images().await.expect("images").len(), before);
    let entries = services
        .store
        .activity_query(&berth_store::ActivityFilter::default(), None, 10)
        .expect("activity");
    assert!(
        entries.iter().all(|e| e.action != COMMIT_ACTION),
        "{entries:?}"
    );
}
