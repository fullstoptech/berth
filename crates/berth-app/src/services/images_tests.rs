//! P4-T1 tests for the images service: how a pull stream folds into layers,
//! which references Pull accepts, and the store's one-request-per-image,
//! pull, tag and run paths over the mock engine.

use std::sync::Arc;

use berth_engine::EngineClient as _;

use super::*;
use crate::state::{use_app_state, AppState, PendingState};

fn default_engine() -> EngineId {
    EngineId("colima:default".to_string())
}

fn event(
    layer: Option<&str>,
    status: &str,
    current: Option<u64>,
    total: Option<u64>,
) -> PullProgress {
    PullProgress {
        layer: layer.map(str::to_string),
        status: status.to_string(),
        current,
        total,
    }
}

fn fresh() -> PullState {
    PullState::new(default_engine(), "postgres:17-alpine", None)
}

// ---------------------------------------------------------------------
// Pull progress
// ---------------------------------------------------------------------

#[test]
fn images_pull_time_left_follows_the_share_done_and_the_time_spent() {
    let mut pull = fresh();
    // No size yet, or too soon: no guess.
    pull.apply(&event(Some("aaaaaaaaaaaa"), "Pulling fs layer", None, None));
    assert_eq!(pull.eta_label(), None);
    pull.apply(&event(
        Some("aaaaaaaaaaaa"),
        "Downloading",
        Some(10_000_000),
        Some(100_000_000),
    ));
    assert_eq!(pull.eta_label(), None, "under two seconds in");
    // Ten seconds for a quarter: thirty to go.
    pull.started = std::time::Instant::now() - std::time::Duration::from_secs(10);
    pull.apply(&event(
        Some("aaaaaaaaaaaa"),
        "Downloading",
        Some(25_000_000),
        Some(100_000_000),
    ));
    let secs = pull.eta_secs.expect("an estimate");
    assert!((29..=31).contains(&secs), "{secs}");
    assert_eq!(pull.eta_label(), Some(format!("{secs} s left")));
    // Minutes past a minute and a half.
    pull.eta_secs = Some(150);
    assert_eq!(pull.eta_label(), Some("3 min left".to_string()));
}

#[test]
fn images_pull_events_fold_into_one_state_per_layer() {
    let mut pull = fresh();
    for e in [
        event(
            Some("17-alpine"),
            "Pulling from library/postgres",
            None,
            None,
        ),
        event(Some("aaaaaaaaaaaa"), "Pulling fs layer", None, None),
        event(Some("bbbbbbbbbbbb"), "Pulling fs layer", None, None),
        event(
            Some("aaaaaaaaaaaa"),
            "Downloading",
            Some(10_000_000),
            Some(40_000_000),
        ),
        event(Some("bbbbbbbbbbbb"), "Waiting", None, None),
        event(
            Some("aaaaaaaaaaaa"),
            "Downloading",
            Some(30_000_000),
            Some(40_000_000),
        ),
    ] {
        pull.apply(&e);
    }
    // "Pulling from" names the image, not a layer.
    assert_eq!(pull.layers.len(), 2, "{:?}", pull.layers);
    assert_eq!(pull.note, "Pulling from library/postgres");
    let first = &pull.layers[0];
    assert_eq!(
        (first.id.as_str(), first.phase),
        ("aaaaaaaaaaaa", LayerPhase::Downloading)
    );
    assert_eq!((first.current, first.total), (30_000_000, 40_000_000));
    assert_eq!(pull.layers[1].phase, LayerPhase::Waiting);
    assert!((first.fraction() - 0.75).abs() < 1e-6);
    assert_eq!(pull.layers[1].fraction(), 0.0);
}

#[test]
fn images_pull_percent_and_bytes_count_finished_layers_whole() {
    let mut pull = fresh();
    for e in [
        event(Some("a"), "Pulling fs layer", None, None),
        event(Some("b"), "Pulling fs layer", None, None),
        event(Some("a"), "Downloading", Some(20_000_000), Some(60_000_000)),
        event(Some("a"), "Download complete", None, None),
        event(Some("b"), "Downloading", Some(10_000_000), Some(40_000_000)),
    ] {
        pull.apply(&e);
    }
    // a is whole (60 MB), b is 10 of 40 MB.
    assert_eq!(pull.bytes(), (70_000_000, 100_000_000));
    assert_eq!(pull.percent(), Some(70));
    assert_eq!(pull.bytes_label(), "70 MB of 100 MB");
    pull.apply(&event(Some("a"), "Pull complete", None, None));
    assert_eq!(pull.layers_done(), (1, 2));
    assert_eq!(pull.layers[0].label(), "Pull complete");
    assert_eq!(pull.layers[1].label(), "10 MB of 40 MB");
}

#[test]
fn images_pull_has_no_percent_until_a_layer_size_is_known() {
    let mut pull = fresh();
    pull.apply(&event(Some("a"), "Pulling fs layer", None, None));
    assert_eq!(pull.percent(), None);
    pull.apply(&event(
        None,
        "Status: Image is up to date for alpine:3",
        None,
        None,
    ));
    assert_eq!(
        pull.bytes_label(),
        "Status: Image is up to date for alpine:3"
    );
}

#[test]
fn images_pull_layers_the_engine_already_has_count_as_done() {
    let mut pull = fresh();
    pull.apply(&event(Some("a"), "Already exists", None, None));
    assert_eq!(pull.layers_done(), (1, 1));
    assert_eq!(pull.layers[0].label(), "Already exists");
    assert_eq!(pull.layers[0].fraction(), 1.0);
}

#[test]
fn images_pull_a_progress_line_never_runs_past_the_layer_total() {
    let mut pull = fresh();
    pull.apply(&event(Some("a"), "Downloading", Some(5), Some(100)));
    pull.apply(&event(Some("a"), "Downloading", Some(500), Some(100)));
    assert_eq!(pull.layers[0].current, 100);
    assert_eq!(pull.percent(), Some(100));
}

// ---------------------------------------------------------------------
// References, names and the starting spec
// ---------------------------------------------------------------------

#[test]
fn images_check_reference_accepts_what_docker_pull_accepts() {
    for ok in [
        "alpine",
        "alpine:3",
        "library/alpine:3.20",
        "ghcr.io/berth/app:v1.2.3",
        "localhost:5000/app",
        "localhost:5000/app:dev",
        "postgres@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "berthfx/web_1-2:latest",
    ] {
        assert_eq!(check_reference(ok).as_deref(), Ok(ok), "{ok}");
    }
}

#[test]
fn images_check_reference_says_what_is_wrong() {
    assert!(check_reference("").is_err());
    assert!(check_reference("   ").is_err());
    assert!(check_reference("two words")
        .unwrap_err()
        .contains("no spaces"));
    assert!(check_reference("Alpine:3").unwrap_err().contains("Alpine"));
    assert!(check_reference("alpine:")
        .unwrap_err()
        .contains("not a valid tag"));
    assert!(check_reference("alpine:ba d").is_err());
    assert!(check_reference("alpine@md5:abc")
        .unwrap_err()
        .contains("digest"));
    assert!(check_reference("/alpine").is_err());
    assert!(check_reference("a//b").is_err());
}

#[test]
fn images_with_default_tag_adds_latest_only_when_there_is_no_tag() {
    assert_eq!(with_default_tag("alpine"), "alpine:latest");
    assert_eq!(with_default_tag("alpine:3"), "alpine:3");
    assert_eq!(
        with_default_tag("localhost:5000/app"),
        "localhost:5000/app:latest"
    );
    assert_eq!(with_default_tag("a@sha256:abc"), "a@sha256:abc");
}

#[test]
fn images_suggest_name_uses_the_repository_and_steps_past_taken_names() {
    assert_eq!(suggest_name("nginx", &[]), "nginx");
    assert_eq!(suggest_name("louislam/uptime-kuma", &[]), "uptime-kuma");
    assert_eq!(suggest_name("nginx", &["nginx"]), "nginx-2");
    assert_eq!(suggest_name("nginx", &["nginx", "nginx-2"]), "nginx-3");
    assert_eq!(suggest_name("", &[]), "container");
    assert_eq!(suggest_name("a b", &[]), "a-b");
}

fn image(tags: &[&str]) -> Image {
    Image {
        id: "sha256:abcdef0123456789".to_string(),
        repo_tags: tags.iter().map(|t| (*t).to_string()).collect(),
        digests: Vec::new(),
        size: bytesize::ByteSize::mb(10),
        created: chrono::Utc::now(),
        arch: berth_core::Arch::Aarch64,
        os: "linux".to_string(),
        used_by: Vec::new(),
        dangling: tags.is_empty(),
        engine: berth_core::EngineId("colima:default".to_string()),
    }
}

#[test]
fn images_base_spec_runs_the_first_tag_and_falls_back_to_the_id() {
    let tagged = base_spec(&image(&["nginx:alpine", "nginx:1"]), "web");
    assert_eq!(tagged.image.reference, "nginx:alpine");
    assert_eq!(tagged.image.id.as_deref(), Some("sha256:abcdef0123456789"));
    assert_eq!(tagged.name, "web");
    let dangling = base_spec(&image(&[]), "x");
    assert_eq!(dangling.image.reference, "sha256:abcdef0123456789");
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
        SLOT.with(|slot| *slot.borrow_mut() = Some((state, store)));
    });
    rsx! { div {} }
}

fn captured() -> (VirtualDom, ImageStore) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let (_state, store) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the store");
    (dom, store)
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

/// Drive the dom until `done` says so (5 s at most).
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

#[tokio::test]
async fn images_store_refresh_lists_every_connected_engine() {
    let (mut dom, store) = captured();
    let (services, engine) = services_with_mock();
    let expected = engine.images().await.expect("images").len();
    dom.in_scope(ScopeId::APP, || store.refresh(&services));
    until(&mut dom, "the list", |dom| {
        dom.in_runtime(|| store.by_engine.peek().contains_key(&default_engine()))
    })
    .await;
    let listed = dom.in_runtime(|| store.by_engine.peek()[&default_engine()].len());
    assert_eq!(listed, expected);
    assert!(expected > 0);
}

#[tokio::test]
async fn images_store_asks_for_a_detail_once_and_keeps_the_layers_and_ports() {
    let (mut dom, store) = captured();
    let (services, engine) = services_with_mock();
    let images = engine.images().await.expect("images");
    let target = images
        .iter()
        .find(|i| i.repo_tags.iter().any(|t| t == "shop-api-api:latest"))
        .expect("an image")
        .clone();
    let key = (default_engine(), target.id.clone());
    dom.in_scope(ScopeId::APP, || {
        store.ensure_detail(&services, &default_engine(), &target.id);
    });
    until(&mut dom, "the detail", |dom| {
        dom.in_runtime(|| store.details.peek().contains_key(&key))
    })
    .await;
    let entry = dom.in_runtime(|| store.details.peek().get(&key).cloned());
    let Some(ImageEntry::Ready(detail)) = entry else {
        panic!("a loaded detail, got {entry:?}");
    };
    assert_eq!(detail.image.id, target.id);
    assert!(!detail.layers.is_empty());

    // The same image again sends nothing: a hand-edited entry stays.
    dom.in_scope(ScopeId::APP, || {
        let mut details = store.details;
        details.write().insert(
            key.clone(),
            ImageEntry::Failed {
                message: "marker".to_string(),
                gone: false,
            },
        );
        store.ensure_detail(&services, &default_engine(), &target.id);
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    dom.render_immediate(&mut dioxus::core::NoOpMutations);
    let again = dom.in_runtime(|| store.details.peek().get(&key).cloned());
    assert!(
        matches!(again, Some(ImageEntry::Failed { .. })),
        "{again:?}"
    );

    // After an invalidate it reads again.
    dom.in_scope(ScopeId::APP, || {
        store.invalidate(&default_engine(), &target.id);
        store.ensure_detail(&services, &default_engine(), &target.id);
    });
    until(&mut dom, "the second read", |dom| {
        dom.in_runtime(|| matches!(store.details.peek().get(&key), Some(ImageEntry::Ready(_))))
    })
    .await;
}

#[tokio::test]
async fn images_store_detail_of_an_unconnected_engine_says_so() {
    let (dom, store) = captured();
    let services = Services {
        store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
        clients: crate::services::engines::client_map(),
        cancel: CancellationToken::new(),
    };
    dom.in_scope(ScopeId::APP, || {
        store.ensure_detail(&services, &default_engine(), "sha256:x");
    });
    let entry = dom.in_runtime(|| {
        store
            .details
            .peek()
            .get(&(default_engine(), "sha256:x".to_string()))
            .cloned()
    });
    match entry {
        Some(ImageEntry::Failed { message, gone }) => {
            assert!(message.contains("not connected"), "{message}");
            assert!(!gone);
        }
        other => panic!("a failed entry, got {other:?}"),
    }
}

#[tokio::test]
async fn images_store_pull_runs_to_done_adds_the_image_and_records_the_action() {
    let (mut dom, store) = captured();
    let (services, engine) = services_with_mock();
    let before = engine.images().await.expect("images").len();
    let result = dom.in_scope(ScopeId::APP, || {
        store.pull(
            &services,
            &default_engine(),
            "alpine:3",
            Some("linux/arm64"),
        )
    });
    assert_eq!(result, Ok(()));
    // The card exists at once, running.
    let started = dom.in_runtime(|| store.pulls.peek().clone());
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].reference, "alpine:3");
    assert_eq!(started[0].platform.as_deref(), Some("linux/arm64"));
    until(&mut dom, "the pull", |dom| {
        dom.in_runtime(|| {
            store
                .pulls
                .peek()
                .iter()
                .all(|p| p.phase == PullPhase::Done)
        })
    })
    .await;
    // The refresh after the pull lists the new image.
    until(&mut dom, "the refresh", |dom| {
        dom.in_runtime(|| {
            store
                .by_engine
                .peek()
                .get(&default_engine())
                .is_some_and(|images| images.len() == before + 1)
        })
    })
    .await;
    let entries = services
        .store
        .activity_query(&berth_store::ActivityFilter::default(), None, 10)
        .expect("activity");
    assert!(
        entries
            .iter()
            .any(|e| e.action == "image.pull" && e.subject_name == "alpine:3"),
        "{entries:?}"
    );
}

#[tokio::test]
async fn images_store_pull_refuses_a_bad_reference_and_an_unconnected_engine() {
    let (dom, store) = captured();
    let (services, _engine) = services_with_mock();
    let bad = dom.in_scope(ScopeId::APP, || {
        store.pull(&services, &default_engine(), "not valid", None)
    });
    assert!(bad.unwrap_err().contains("no spaces"));
    let missing = dom.in_scope(ScopeId::APP, || {
        store.pull(
            &services,
            &EngineId("nowhere".to_string()),
            "alpine:3",
            None,
        )
    });
    assert!(missing.unwrap_err().contains("not connected"));
    assert!(dom.in_runtime(|| store.pulls.peek().is_empty()));
}

#[tokio::test]
async fn images_store_a_second_pull_of_the_same_reference_does_not_start_another() {
    let (dom, store) = captured();
    let (services, _engine) = services_with_mock();
    dom.in_scope(ScopeId::APP, || {
        assert_eq!(
            store.pull(&services, &default_engine(), "alpine:3", None),
            Ok(())
        );
        // A still-running pull is left alone.
        assert_eq!(
            store.pull(&services, &default_engine(), "alpine:3", None),
            Ok(())
        );
    });
    assert_eq!(dom.in_runtime(|| store.pulls.peek().len()), 1);
    assert_eq!(dom.in_runtime(|| store.pulling()), 1);
}

#[tokio::test]
async fn images_store_cancel_pull_ends_it_as_cancelled() {
    let (mut dom, store) = captured();
    let (services, engine) = services_with_mock();
    let before = engine.images().await.expect("images").len();
    dom.in_scope(ScopeId::APP, || {
        store
            .pull(&services, &default_engine(), "berthfx/never:1", None)
            .expect("started");
        // Cancelled before the stream's first poll: nothing is pulled.
        store.cancel_pull(&(default_engine(), "berthfx/never:1".to_string()));
    });
    until(&mut dom, "the end of the pull", |dom| {
        dom.in_runtime(|| store.pulls.peek().iter().all(|p| !p.is_running()))
    })
    .await;
    let phase = dom.in_runtime(|| store.pulls.peek()[0].phase.clone());
    assert_eq!(phase, PullPhase::Cancelled);
    assert_eq!(engine.images().await.expect("images").len(), before);
}

#[tokio::test]
async fn images_store_dismiss_removes_a_finished_pull_but_not_a_running_one() {
    let (mut dom, store) = captured();
    let (services, _engine) = services_with_mock();
    dom.in_scope(ScopeId::APP, || {
        store
            .pull(&services, &default_engine(), "alpine:3", None)
            .expect("started");
        store.dismiss_pull(&(default_engine(), "alpine:3".to_string()));
    });
    assert_eq!(
        dom.in_runtime(|| store.pulls.peek().len()),
        1,
        "running stays"
    );
    until(&mut dom, "done", |dom| {
        dom.in_runtime(|| store.pulls.peek().iter().all(|p| !p.is_running()))
    })
    .await;
    dom.in_scope(ScopeId::APP, || {
        store.dismiss_pull(&(default_engine(), "alpine:3".to_string()));
    });
    assert!(dom.in_runtime(|| store.pulls.peek().is_empty()));
}

#[tokio::test]
async fn images_store_tag_gives_the_image_another_name_and_refreshes_the_list() {
    let (mut dom, store) = captured();
    let (services, engine) = services_with_mock();
    let target = engine
        .images()
        .await
        .expect("images")
        .into_iter()
        .find(|i| i.repo_tags.iter().any(|t| t == "postgres:17-alpine"))
        .expect("postgres");
    let heard = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = heard.clone();
    dom.in_scope(ScopeId::APP, || {
        store.tag(
            &services,
            &default_engine(),
            &target,
            "berthfx/pg",
            "mine",
            move |r| {
                *sink.borrow_mut() = Some(r);
            },
        );
    });
    until(&mut dom, "the tag result", |_| heard.borrow().is_some()).await;
    assert_eq!(*heard.borrow(), Some(Ok(())));
    until(&mut dom, "the refreshed list", |dom| {
        dom.in_runtime(|| {
            store
                .by_engine
                .peek()
                .get(&default_engine())
                .is_some_and(|images| {
                    images
                        .iter()
                        .any(|i| i.repo_tags.iter().any(|t| t == "berthfx/pg:mine"))
                })
        })
    })
    .await;
}

#[tokio::test]
async fn images_store_tag_reports_the_engines_refusal() {
    let (mut dom, store) = captured();
    let (services, _engine) = services_with_mock();
    let ghost = image(&["x:1"]);
    let heard = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = heard.clone();
    dom.in_scope(ScopeId::APP, || {
        store.tag(&services, &default_engine(), &ghost, "a", "b", move |r| {
            *sink.borrow_mut() = Some(r);
        });
    });
    until(&mut dom, "the tag result", |_| heard.borrow().is_some()).await;
    let result = heard.borrow().clone().expect("a result");
    assert!(result.is_err(), "{result:?}");
}

#[tokio::test]
async fn images_store_run_reports_what_create_said() {
    // The mock engine cannot create: the sheet must show that text, not hang.
    let (mut dom, store) = captured();
    let (services, _engine) = services_with_mock();
    let spec = base_spec(&image(&["nginx:alpine"]), "berthfx-run1");
    let heard = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = heard.clone();
    dom.in_scope(ScopeId::APP, || {
        store.run(&services, &default_engine(), spec, move |r| {
            *sink.borrow_mut() = Some(r);
        });
    });
    until(&mut dom, "the run result", |_| heard.borrow().is_some()).await;
    let result = heard.borrow().clone().expect("a result");
    assert!(result.is_err(), "{result:?}");
    let entries = services
        .store
        .activity_query(&berth_store::ActivityFilter::default(), None, 10)
        .expect("activity");
    let run = entries
        .iter()
        .find(|e| e.action == "image.run")
        .expect("a run entry");
    assert_eq!(run.subject_name, "berthfx-run1");
    assert!(
        matches!(run.outcome, Outcome::Failed { .. }),
        "{:?}",
        run.outcome
    );
}

#[test]
fn images_find_prefers_the_scope_engine_then_any_that_has_it() {
    let (dom, store) = captured();
    let other = EngineId("colima:work".to_string());
    dom.in_scope(ScopeId::APP, || {
        store.seed(&default_engine(), vec![image(&["a:1"])]);
        let mut there = image(&["a:1"]);
        there.engine = other.clone();
        store.seed(&other, vec![there]);
    });
    let id = "sha256:abcdef0123456789";
    let engine_of = |scope: &crate::state::EngineScope| {
        dom.in_runtime(|| store.find(id, scope)).map(|i| i.engine)
    };
    assert_eq!(
        engine_of(&crate::state::EngineScope::One(other.clone())),
        Some(other.clone())
    );
    // BTreeMap order: colima:default sorts before colima:work.
    assert_eq!(
        engine_of(&crate::state::EngineScope::All),
        Some(default_engine())
    );
    assert_eq!(
        dom.in_runtime(|| store.find("sha256:none", &crate::state::EngineScope::All)),
        None
    );
}

// ---------------------------------------------------------------------
// Delete (AD25a)
// ---------------------------------------------------------------------

fn captured_all() -> (VirtualDom, AppState, ImageStore) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let (state, store) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured the store");
    (dom, state, store)
}

fn load_flat_images(dom: &VirtualDom, mut state: AppState, images: Vec<Image>) {
    dom.in_runtime(|| state.images.set(images));
}

fn actions_service(
    services: &Services,
) -> (
    crate::services::actions::ActionsService,
    tokio::sync::mpsc::UnboundedReceiver<crate::services::actions::Finished>,
) {
    crate::services::actions::ActionsService::new(
        services.clients.clone(),
        services.store.clone(),
        Arc::new(berth_engine::MockCompose::bundled()),
        Arc::new(berth_engine::MockColima::bundled()),
        CancellationToken::new(),
    )
}

/// Wait for the one action in flight and apply its result, as the App root does.
async fn finish_one(
    dom: &VirtualDom,
    mut state: AppState,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::services::actions::Finished>,
) {
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("the action finishes")
        .expect("a result");
    dom.in_runtime(|| crate::services::actions::apply_finished(&mut state, done));
}

#[tokio::test]
async fn images_remove_deletes_clears_the_pending_state_and_records_it() {
    let (dom, state, _store) = captured_all();
    let (services, engine) = services_with_mock();
    let (actions, mut rx) = actions_service(&services);
    let images = engine.images().await.expect("images");
    load_flat_images(&dom, state, images.clone());
    let target = images
        .iter()
        .find(|i| i.used_by.is_empty() && !i.repo_tags.is_empty())
        .expect("an unused image")
        .clone();
    let mut state_copy = state;
    let reference = berth_core::ResourceRef::Image(target.id.clone());
    dom.in_runtime(|| {
        actions.remove(
            &mut state_copy,
            &default_engine(),
            &reference,
            &target.repo_tags[0],
            false,
        )
    })
    .expect("accepted");
    // Pending at once, "Deleting…".
    let key = crate::state::PendingKey {
        subject: Subject::Image(target.id.clone()),
        action: "image.delete",
    };
    let pending = dom.in_runtime(|| state.pending.peek().get(&key).cloned());
    assert_eq!(
        pending,
        Some(PendingState::Running {
            label: "Deleting\u{2026}".to_string()
        })
    );
    finish_one(&dom, state, &mut rx).await;
    assert!(dom.in_runtime(|| !state.pending.peek().contains_key(&key)));
    let left = engine.images().await.expect("images");
    assert!(left.iter().all(|i| i.id != target.id), "the engine lost it");
    let entries = services
        .store
        .activity_query(&berth_store::ActivityFilter::default(), None, 10)
        .expect("activity");
    assert!(
        entries.iter().any(|e| e.action == "image.delete"
            && e.subject_name == target.repo_tags[0]
            && matches!(e.outcome, Outcome::Ok)),
        "{entries:?}"
    );
}

#[tokio::test]
async fn images_remove_refuses_an_image_a_container_uses_and_sets_nothing() {
    let (dom, state, _store) = captured_all();
    let (services, engine) = services_with_mock();
    let (actions, _rx) = actions_service(&services);
    let images = engine.images().await.expect("images");
    load_flat_images(&dom, state, images.clone());
    let used = images
        .iter()
        .find(|i| i.used_by.len() == 2)
        .expect("an image of two containers")
        .clone();
    let mut state_copy = state;
    let refused = dom.in_runtime(|| {
        actions.remove(
            &mut state_copy,
            &default_engine(),
            &berth_core::ResourceRef::Image(used.id.clone()),
            "x",
            false,
        )
    });
    assert_eq!(refused, Err("Used by 2 containers".to_string()));
    assert!(dom.in_runtime(|| state.pending.peek().is_empty()));
    assert!(engine
        .images()
        .await
        .expect("images")
        .iter()
        .any(|i| i.id == used.id));
}

#[tokio::test]
async fn images_remove_failure_stays_on_the_row_with_the_engines_words() {
    let (dom, state, _store) = captured_all();
    let (services, _engine) = services_with_mock();
    let (actions, mut rx) = actions_service(&services);
    // The flat list knows an image the engine does not: a stale row.
    let mut ghost = image(&["berthfx/ghost:1"]);
    ghost.id = "sha256:ghost".to_string();
    load_flat_images(&dom, state, vec![ghost.clone()]);
    let mut state_copy = state;
    dom.in_runtime(|| {
        actions.remove(
            &mut state_copy,
            &default_engine(),
            &berth_core::ResourceRef::Image(ghost.id.clone()),
            "berthfx/ghost:1",
            false,
        )
    })
    .expect("accepted");
    finish_one(&dom, state, &mut rx).await;
    let key = crate::state::PendingKey {
        subject: Subject::Image(ghost.id.clone()),
        action: "image.delete",
    };
    assert!(
        matches!(
            dom.in_runtime(|| state.pending.peek().get(&key).cloned()),
            Some(PendingState::Failed { .. })
        ),
        "{:?}",
        dom.in_runtime(|| state.pending.peek().clone())
    );
    let entries = services
        .store
        .activity_query(&berth_store::ActivityFilter::default(), None, 10)
        .expect("activity");
    assert!(entries
        .iter()
        .any(|e| e.action == "image.delete" && matches!(e.outcome, Outcome::Failed { .. })));
}

#[tokio::test]
async fn images_confirm_run_sends_a_remove_image_model_to_the_actions_service() {
    let (dom, state, _store) = captured_all();
    let (services, engine) = services_with_mock();
    let images = engine.images().await.expect("images");
    load_flat_images(&dom, state, images.clone());
    let target = images
        .iter()
        .find(|i| i.used_by.is_empty() && !i.repo_tags.is_empty())
        .expect("an unused image")
        .clone();
    let model = crate::services::confirm::delete_image_model(&default_engine(), &target);
    let (actions, mut rx) = actions_service(&services);
    let mut state_copy = state;
    let launched =
        dom.in_runtime(|| crate::services::confirm::run(&mut state_copy, &actions, &model, false));
    assert!(launched.refused.is_empty(), "{:?}", launched.refused);
    assert_eq!(launched.keys.len(), 1);
    finish_one(&dom, state, &mut rx).await;
    assert!(engine
        .images()
        .await
        .expect("images")
        .iter()
        .all(|i| i.id != target.id));
}

#[test]
fn images_a_dialog_for_one_image_cannot_delete_another() {
    let (dom, state, _store) = captured_all();
    let (services, _engine) = services_with_mock();
    let (actions, _rx) = actions_service(&services);
    let a = image(&["a:1"]);
    let mut b = image(&["b:1"]);
    b.id = "sha256:other".to_string();
    load_flat_images(&dom, state, vec![a.clone(), b.clone()]);
    let mut model = crate::services::confirm::delete_image_model(&default_engine(), &a);
    model.targets = vec![Subject::Image(b.id.clone())];
    let mut state_copy = state;
    let launched =
        dom.in_runtime(|| crate::services::confirm::run(&mut state_copy, &actions, &model, false));
    assert!(launched.keys.is_empty());
    assert_eq!(launched.refused, ["This is not the image to delete"]);
}
