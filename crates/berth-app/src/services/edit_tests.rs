//! P3-T7 service tests: what `run_edit` does against the mock engine, and
//! how the actions service applies an edit's outcome. The real recreate
//! (stop, rename, create, restore) is covered live in
//! `tests/edit_recreate_it.rs`; the mock engine cannot create containers.

use std::sync::Arc;

use berth_core::{ContainerId, EngineId, Limits, RestartPolicy, Subject};
use berth_engine::EngineClient;
use bytesize::ByteSize;
use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::services::actions::{apply_finished, ActionsService, ContainerTarget, Finished};
use crate::state::{use_app_state, AppState, PendingKey, PendingState, Selection};

fn engine_id() -> EngineId {
    EngineId("colima:default".to_string())
}

fn api() -> berth_core::Container {
    berth_engine::MockWorld::bundled()
        .containers
        .into_iter()
        .find(|c| c.name == "shop-api-api-1")
        .expect("shop-api-api-1 in the mock world")
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

fn harness() -> (VirtualDom, AppState) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    (dom, state)
}

fn memory_limit() -> EditOp {
    EditOp::ApplyLimits {
        limits: Limits {
            cpus: Some(0.5),
            memory: Some(ByteSize(256 << 20)),
            memory_swap: None,
        },
        restart: RestartPolicy::Always,
    }
}

#[tokio::test]
async fn edit_recreate_apply_limits_reaches_the_engine_and_says_so() {
    let engine = berth_engine::MockEngine::bundled(&engine_id());
    let c = api();
    let done = run_edit(
        &engine,
        &c.id,
        &c.name,
        memory_limit(),
        &CancellationToken::new(),
    )
    .await
    .expect("applied");
    assert_eq!(
        done.message.as_deref(),
        Some("Limits applied to shop-api-api-1")
    );
    assert_eq!(done.replaced_by, None);
    let after = engine.container(&c.id).await.expect("still there");
    assert_eq!(after.limits.memory, Some(ByteSize(268_435_456)));
    assert_eq!(after.limits.cpus, Some(0.5));
    assert_eq!(after.restart, RestartPolicy::Always);
}

fn limits_of(op: &EditOp) -> Limits {
    match op {
        EditOp::ApplyLimits { limits, .. } => limits.clone(),
        other => panic!("not a limits update: {other:?}"),
    }
}

#[test]
fn edit_recreate_a_new_memory_limit_without_swap_gets_twice_the_memory_as_swap() {
    // Docker refuses `-m 256m` on a container without a swap setting
    // ("update the memoryswap at the same time"); live on this engine.
    let old = berth_core::ContainerSpec::from_detail(
        &futures::executor::block_on(
            berth_engine::MockEngine::bundled(&engine_id()).container(&api().id),
        )
        .expect("detail"),
    );
    let mut new = old.clone();
    new.limits.memory = Some(ByteSize(256 << 20));
    let limits = limits_of(&EditOp::apply_limits(&old, &new));
    assert_eq!(limits.memory, Some(ByteSize(256 << 20)));
    assert_eq!(limits.memory_swap, Some(ByteSize(512 << 20)));

    // A swap figure the user gave is theirs.
    new.limits.memory_swap = Some(ByteSize(300 << 20));
    assert_eq!(
        limits_of(&EditOp::apply_limits(&old, &new)).memory_swap,
        Some(ByteSize(300 << 20))
    );

    // A memory limit that did not change keeps its (absent) swap: a CPU-only
    // edit must not turn an unlimited swap into 512 MB.
    let mut holding = old.clone();
    holding.limits.memory = Some(ByteSize(256 << 20));
    let mut cpu_only = holding.clone();
    cpu_only.limits.cpus = Some(0.5);
    let limits = limits_of(&EditOp::apply_limits(&holding, &cpu_only));
    assert_eq!(limits.memory_swap, None);
    assert_eq!(limits.cpus, Some(0.5));
}

#[tokio::test]
async fn edit_recreate_a_missing_container_fails_with_the_engine_text() {
    let engine = berth_engine::MockEngine::bundled(&engine_id());
    let gone = ContainerId("nope".to_string());
    let failed = run_edit(
        &engine,
        &gone,
        "nope",
        memory_limit(),
        &CancellationToken::new(),
    )
    .await
    .expect_err("no such container");
    assert!(failed.contains("nope"), "{failed}");
}

#[tokio::test]
async fn edit_recreate_a_failed_recreate_reports_the_engine_and_names_no_new_container() {
    let engine = berth_engine::MockEngine::bundled(&engine_id());
    let c = api();
    let detail = engine.container(&c.id).await.expect("detail");
    let spec = berth_core::ContainerSpec::from_detail(&detail);
    let failed = run_edit(
        &engine,
        &c.id,
        &c.name,
        EditOp::Recreate {
            spec: Box::new(spec),
            pull: false,
        },
        &CancellationToken::new(),
    )
    .await
    .expect_err("the mock cannot recreate");
    assert!(failed.contains("recreate"), "{failed}");
    // The container the failed recreate started from is still there.
    assert!(engine.container(&c.id).await.is_ok());
}

#[tokio::test]
async fn edit_recreate_recreate_latest_pulls_before_it_touches_the_container() {
    let engine = berth_engine::MockEngine::bundled(&engine_id());
    let mut c = api();
    // An image the mock engine has not got: the pull adds it.
    let reference = "berthfx/latest-probe:1";
    assert!(!engine
        .images()
        .await
        .expect("images")
        .iter()
        .any(|i| i.repo_tags.iter().any(|t| t == reference)));
    // The container's own image is what is pulled; point the row at the probe.
    c.image.reference = reference.to_string();
    let spec_engine = berth_engine::MockEngine::bundled(&engine_id());
    let mut detail = spec_engine.container(&c.id).await.expect("detail");
    detail.container.image.reference = reference.to_string();
    let spec = berth_core::ContainerSpec::from_detail(&detail);
    let cancel = CancellationToken::new();
    let result = super::pull(&engine, &spec, &cancel).await;
    assert_eq!(result, Ok(()));
    assert!(
        engine
            .images()
            .await
            .expect("images")
            .iter()
            .any(|i| i.repo_tags.iter().any(|t| t == reference)),
        "the pull ran to its end"
    );
}

#[tokio::test]
async fn edit_recreate_a_cancelled_edit_stops_at_once() {
    let engine = berth_engine::MockEngine::bundled(&engine_id());
    let c = api();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let spec =
        berth_core::ContainerSpec::from_detail(&engine.container(&c.id).await.expect("detail"));
    let op = EditOp::Recreate {
        spec: Box::new(spec),
        pull: true,
    };
    let result = run_edit(&engine, &c.id, &c.name, op, &cancel).await;
    assert_eq!(result, Err("cancelled".to_string()));
}

// ---------------------------------------------------------------------
// Through the actions service
// ---------------------------------------------------------------------

fn service() -> (
    ActionsService,
    tokio::sync::mpsc::UnboundedReceiver<Finished>,
) {
    let clients = crate::services::engines::client_map();
    let engine: Arc<dyn EngineClient> = Arc::new(berth_engine::MockEngine::bundled(&engine_id()));
    clients.lock().expect("clients").insert(engine_id(), engine);
    ActionsService::new(
        clients,
        Arc::new(berth_store::Store::open_in_memory().expect("store")),
        Arc::new(berth_engine::MockCompose::default()),
        Arc::new(berth_engine::MockColima::bundled()),
        CancellationToken::new(),
    )
}

#[tokio::test]
async fn edit_recreate_service_runs_apply_limits_with_a_pending_state_and_a_toast() {
    let (dom, mut state) = harness();
    let c = api();
    dom.in_runtime(|| state.containers.set(vec![c.clone()]));
    let (actions, mut finished) = service();
    let target = ContainerTarget::from(&c);
    dom.in_runtime(|| {
        actions
            .edit(&mut state, &target, memory_limit())
            .expect("accepted")
    });
    let key = PendingKey {
        subject: Subject::Container(c.id.clone()),
        action: "container.apply_limits",
    };
    assert_eq!(
        dom.in_runtime(|| state.pending.peek().get(&key).cloned()),
        Some(PendingState::Running {
            label: "Applying…".to_string()
        }),
        "pending is set before the engine answers"
    );
    // A second edit of the same container waits for the first.
    let second = dom.in_runtime(|| actions.edit(&mut state, &target, memory_limit()));
    assert!(second.is_err(), "still in progress");

    let done = tokio::time::timeout(std::time::Duration::from_secs(5), finished.recv())
        .await
        .expect("finishes")
        .expect("open");
    assert_eq!(done.result, Ok(()));
    dom.in_runtime(|| apply_finished(&mut state, done));
    assert!(dom.in_runtime(|| state.pending.peek().is_empty()));
    let toasts = dom.in_runtime(|| state.toasts.peek().clone());
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0].text, "Limits applied to shop-api-api-1");
}

#[tokio::test]
async fn edit_recreate_service_keeps_a_failed_recreate_as_a_failed_state() {
    let (dom, mut state) = harness();
    let c = api();
    dom.in_runtime(|| state.containers.set(vec![c.clone()]));
    let (actions, mut finished) = service();
    let target = ContainerTarget::from(&c);
    let spec = berth_core::ContainerSpec::from_detail(
        &futures::executor::block_on(
            berth_engine::MockEngine::bundled(&engine_id()).container(&c.id),
        )
        .expect("detail"),
    );
    dom.in_runtime(|| {
        actions
            .edit(
                &mut state,
                &target,
                EditOp::Recreate {
                    spec: Box::new(spec),
                    pull: true,
                },
            )
            .expect("accepted")
    });
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), finished.recv())
        .await
        .expect("finishes")
        .expect("open");
    assert!(done.result.is_err(), "the mock cannot recreate");
    dom.in_runtime(|| apply_finished(&mut state, done));
    let key = PendingKey {
        subject: Subject::Container(c.id.clone()),
        action: "container.recreate",
    };
    assert!(matches!(
        dom.in_runtime(|| state.pending.peek().get(&key).cloned()),
        Some(PendingState::Failed { .. })
    ));
    assert!(
        dom.in_runtime(|| state.toasts.peek().is_empty()),
        "no success toast"
    );
}

#[test]
fn edit_recreate_the_selection_follows_the_new_container_after_a_recreate() {
    let (dom, mut state) = harness();
    let old = ContainerId("old".to_string());
    let new = ContainerId("new".to_string());
    let finished = |replaced_by: Option<ContainerId>| Finished {
        key: PendingKey {
            subject: Subject::Container(old.clone()),
            action: "container.recreate",
        },
        result: Ok(()),
        replaced_by,
        message: None,
    };
    dom.in_runtime(|| {
        state
            .selection
            .set(Selection::Containers(vec![old.clone()]))
    });
    dom.in_runtime(|| apply_finished(&mut state, finished(Some(new.clone()))));
    assert_eq!(
        dom.in_runtime(|| state.selection.peek().clone()),
        Selection::Containers(vec![new.clone()])
    );
    // Someone else's selection is left alone.
    let other = ContainerId("other".to_string());
    dom.in_runtime(|| {
        state
            .selection
            .set(Selection::Containers(vec![other.clone()]))
    });
    dom.in_runtime(|| apply_finished(&mut state, finished(Some(new))));
    assert_eq!(
        dom.in_runtime(|| state.selection.peek().clone()),
        Selection::Containers(vec![other])
    );
}

#[test]
fn edit_recreate_every_op_has_its_own_pending_action() {
    let ids = [memory_limit().action_id(), EditOp::Duplicate.action_id()];
    assert_eq!(ids, ["container.apply_limits", "container.duplicate"]);
    for id in ids {
        assert_ne!(
            crate::services::actions::pending_label(id),
            "Working…",
            "{id}"
        );
    }
}

// ---------------------------------------------------------------------
// ⌘E and "Recreate with latest image"
// ---------------------------------------------------------------------

#[test]
fn edit_recreate_cmd_e_opens_the_sheet_of_the_one_selected_container() {
    use crate::state::Overlay;
    let (dom, mut state) = harness();
    let c = api();
    assert_eq!(
        crate::keys::map_key(
            "e",
            crate::keys::Modifiers::META,
            crate::keys::KeyContext::default()
        ),
        Some("container.edit")
    );
    // Nothing selected: nothing opens.
    dom.in_runtime(|| crate::keys::dispatch(&mut state, "container.edit"));
    assert!(dom.in_runtime(|| state.overlays.peek().is_empty()));
    dom.in_runtime(|| {
        state
            .selection
            .set(Selection::Containers(vec![c.id.clone()]))
    });
    dom.in_runtime(|| crate::keys::dispatch(&mut state, "container.edit"));
    assert_eq!(
        dom.in_runtime(|| state.overlays.peek().clone()),
        vec![Overlay::EditRecreate(c.id.clone())]
    );
    // Pressed again, or over another overlay: still one sheet.
    dom.in_runtime(|| crate::keys::dispatch(&mut state, "container.edit"));
    assert_eq!(dom.in_runtime(|| state.overlays.peek().len()), 1);
    // Several selected: no sheet.
    dom.in_runtime(|| {
        state.overlays.write().clear();
        state.selection.set(Selection::Containers(vec![
            c.id.clone(),
            ContainerId("other".to_string()),
        ]));
        crate::keys::dispatch(&mut state, "container.edit");
    });
    assert!(dom.in_runtime(|| state.overlays.peek().is_empty()));
}

#[tokio::test]
async fn edit_recreate_latest_image_dialog_counts_its_one_target_and_starts_the_pull_and_recreate()
{
    use crate::services::confirm;
    let (dom, mut state) = harness();
    let c = api();
    dom.in_runtime(|| state.containers.set(vec![c.clone()]));
    let detail = futures::executor::block_on(
        berth_engine::MockEngine::bundled(&engine_id()).container(&c.id),
    )
    .expect("detail");
    let model = dom
        .in_runtime(|| {
            confirm::recreate_latest_model(
                &state,
                &c.id,
                berth_core::ContainerSpec::from_detail(&detail),
            )
        })
        .expect("a dialog for a known container");
    // One action: pull, then recreate, with the spec in it (D18).
    assert!(matches!(
        &model.action,
        berth_core::EngineAction::Recreate { pull: true, spec, id, .. }
            if spec.name == c.name && *id == c.id
    ));
    assert_eq!(model.effect.count as usize, model.targets.len());
    assert_eq!(model.targets, vec![Subject::Container(c.id.clone())]);
    assert_eq!(confirm::button_label(&model.effect), "Recreate container");
    assert!(model.title.contains("latest image"), "{}", model.title);
    assert!(model.effect.lines[0].text.contains("pulled again"));

    let (actions, mut finished) = service();
    let launched = dom.in_runtime(|| confirm::run(&mut state, &actions, &model, false));
    assert_eq!(launched.refused, Vec::<String>::new());
    assert_eq!(
        launched.keys,
        vec![PendingKey {
            subject: Subject::Container(c.id.clone()),
            action: "container.recreate"
        }]
    );
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), finished.recv())
        .await
        .expect("finishes")
        .expect("open");
    assert_eq!(done.key, launched.keys[0]);
    // A target that vanished is refused, not guessed at.
    dom.in_runtime(|| state.containers.set(Vec::new()));
    let gone = dom.in_runtime(|| confirm::run(&mut state, &actions, &model, false));
    assert!(gone.keys.is_empty());
    assert_eq!(
        gone.refused,
        vec!["This container no longer exists".to_string()]
    );
}
