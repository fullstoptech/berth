//! P4-T4 tests for the Create volume sheet: the scene draws the form, the
//! engine choice and the "name taken" rule follow the lists, and a create
//! through the actions service makes the volume and says so.

use std::sync::Arc;

use super::*;
use crate::scenes;
use crate::services::actions::ActionsService;
use crate::state::{use_app_state, EngineScope};

fn default_engine() -> EngineId {
    EngineId("colima:default".to_string())
}

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

fn rig() -> (VirtualDom, AppState) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    let world = berth_engine::MockWorld::bundled();
    dom.in_runtime(|| {
        let mut state = state;
        state.engines.set(world.engines.clone());
        state.containers.set(world.containers.clone());
        state.volumes.set(
            world
                .volumes_by_engine
                .values()
                .flatten()
                .cloned()
                .collect(),
        );
    });
    (dom, state)
}

#[test]
fn volumes_create_scene_draws_name_driver_options_and_a_locked_button() {
    let html = scenes::render_scene_html("Volumes / create / 1440").expect("scene");
    assert!(html.contains("role=\"dialog\""));
    assert!(html.contains("aria-label=\"Create volume\""), "{html}");
    for label in ["Name", "Driver", "Options"] {
        assert!(html.contains(&format!(">{label}<")), "{label}");
    }
    assert!(html.contains("aria-label=\"Volume name\""));
    assert!(html.contains("Cancel"));
    // With no name the button is locked and says why.
    let (before, _) = html
        .rsplit_once("class=\"btn-label\">Create volume<")
        .expect("the sheet's button");
    let create = before.rsplit("<button").next().expect("the primary button");
    assert!(create.contains("disabled"), "{create}");
    assert!(
        create.contains("data-tooltip=\"Give the volume a name\""),
        "{create}"
    );
    // One engine runs, so there is nothing to choose.
    assert!(!html.contains("aria-label=\"Engine\""));
}

#[test]
fn volumes_create_offers_only_running_engines_in_scope() {
    let (dom, state) = rig();
    let engines = dom.in_runtime(|| state.engines.peek().clone());
    let all = candidates(&engines, &EngineScope::All);
    assert_eq!(all, [(default_engine(), "default".to_string())]);
    let work = candidates(
        &engines,
        &EngineScope::One(EngineId("colima:work".to_string())),
    );
    assert!(work.is_empty(), "a stopped engine cannot take a volume");
}

#[test]
fn volumes_create_name_taken_follows_the_engines_own_list() {
    let (dom, state) = rig();
    dom.in_runtime(|| {
        assert!(name_taken(&state, "stale-cache", &default_engine()));
        assert!(name_taken(&state, " stale-cache ", &default_engine()));
        assert!(!name_taken(&state, "berthfx-vol", &default_engine()));
        // crm_pgdata is on the work engine, so the name is free here.
        assert!(!name_taken(&state, "crm_pgdata", &default_engine()));
        assert!(name_taken(
            &state,
            "crm_pgdata",
            &EngineId("colima:work".to_string())
        ));
    });
}

#[tokio::test]
async fn volumes_create_through_the_actions_service_makes_the_volume() {
    let (dom, state) = rig();
    let clients = crate::services::engines::client_map();
    let engine = Arc::new(berth_engine::MockEngine::bundled(&default_engine()));
    clients.lock().expect("clients").insert(
        default_engine(),
        Arc::clone(&engine) as Arc<dyn berth_engine::EngineClient>,
    );
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let (service, mut finished) = ActionsService::new(
        clients,
        Arc::clone(&store),
        Arc::new(berth_engine::MockCompose::bundled()),
        Arc::new(berth_engine::MockColima::bundled()),
        tokio_util::sync::CancellationToken::new(),
    );
    let spec = volumes::create_spec("berthfx-vol", "local", "").expect("spec");
    dom.in_runtime(|| {
        let mut state = state;
        service
            .volume(&mut state, &default_engine(), "berthfx-vol", VolumeOp::Create(spec))
            .expect("starts");
        let key = PendingKey {
            subject: Subject::Volume("berthfx-vol".to_string()),
            action: volumes::CREATE,
        };
        assert!(
            matches!(state.pending.peek().get(&key), Some(PendingState::Running { label }) if label == "Creating\u{2026}")
        );
    });
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), finished.recv())
        .await
        .expect("finishes")
        .expect("open");
    assert_eq!(done.result, Ok(()));
    assert_eq!(done.message.as_deref(), Some("Created 1 volume"));
    let names: Vec<String> = berth_engine::EngineClient::volumes(engine.as_ref())
        .await
        .expect("volumes")
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert!(names.contains(&"berthfx-vol".to_string()));
}

#[tokio::test]
async fn volumes_create_a_taken_name_fails_with_the_engines_words_and_keeps_the_failure() {
    let (dom, state) = rig();
    let clients = crate::services::engines::client_map();
    clients.lock().expect("clients").insert(
        default_engine(),
        Arc::new(berth_engine::MockEngine::bundled(&default_engine()))
            as Arc<dyn berth_engine::EngineClient>,
    );
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let (service, mut finished) = ActionsService::new(
        clients,
        store,
        Arc::new(berth_engine::MockCompose::bundled()),
        Arc::new(berth_engine::MockColima::bundled()),
        tokio_util::sync::CancellationToken::new(),
    );
    let spec = volumes::create_spec("stale-cache", "local", "").expect("spec");
    dom.in_runtime(|| {
        let mut state = state;
        service
            .volume(
                &mut state,
                &default_engine(),
                "stale-cache",
                VolumeOp::Create(spec),
            )
            .expect("starts");
    });
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), finished.recv())
        .await
        .expect("finishes")
        .expect("open");
    dom.in_runtime(|| {
        let mut state = state;
        crate::services::actions::apply_finished(&mut state, done);
        let pending = state.pending.peek();
        let failed = pending
            .values()
            .find_map(|v| match v {
                PendingState::Failed { message } => Some(message.clone()),
                PendingState::Running { .. } => None,
            })
            .expect("the failure stays for the sheet to show");
        assert!(failed.contains("already exists"), "{failed}");
    });
}
