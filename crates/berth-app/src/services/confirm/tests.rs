//! P2-T8 tests for the confirm models: the delete dialog says what the
//! frame says, the button and result repeat the same count, a running
//! container becomes "Stop and delete", a typed name locks the button, and
//! a confirmed model reaches the actions service.

use std::sync::Arc;

use dioxus::prelude::*;

use berth_core::{
    Container, ContainerAction, ContainerId, EffectKind, EffectSummary, EngineAction, EngineId,
    Volume,
};

use super::*;
use crate::state::{use_app_state, Overlay, Selection};

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn default_engine() -> EngineId {
    EngineId("colima:default".to_string())
}

fn container(name: &str) -> Container {
    world()
        .containers
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} in the mock world"))
}

fn volumes() -> Vec<Volume> {
    world()
        .volumes_by_engine
        .values()
        .flatten()
        .cloned()
        .collect()
}

fn texts(effect: &EffectSummary, kind: EffectKind) -> Vec<&str> {
    effect
        .lines
        .iter()
        .filter(|l| l.kind == kind)
        .map(|l| l.text.as_str())
        .collect()
}

#[test]
fn confirm_delete_of_the_db_says_what_the_frame_says() {
    let db = container("shop-api-db-1");
    let effect = delete_effect(&[&db], &volumes());
    let said: Vec<&str> = effect.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        said.join(" "),
        "Its writable layer is deleted. Volume shop-api_pgdata (1.2 GB) is kept."
    );
    assert_eq!(texts(&effect, EffectKind::Goes).len(), 1);
    assert_eq!(texts(&effect, EffectKind::Stays).len(), 1);
    // An exited container is not stopped first.
    assert!(texts(&effect, EffectKind::Changes).is_empty());
    assert_eq!(button_label(&effect), "Delete container");
    assert_eq!(result_label(&effect, 1), "Deleted 1 container");
}

#[test]
fn confirm_a_running_container_gets_stop_and_delete() {
    let api = container("shop-api-api-1");
    let effect = delete_effect(&[&api], &volumes());
    assert_eq!(
        texts(&effect, EffectKind::Changes),
        ["shop-api-api-1 is stopped first."]
    );
    assert_eq!(button_label(&effect), "Stop and delete");
    // The result still counts what was deleted.
    assert_eq!(result_label(&effect, 1), "Deleted 1 container");
}

#[test]
fn confirm_bulk_delete_repeats_one_count_in_button_and_result() {
    let names = [
        "shop-api-api-1",
        "shop-api-worker-1",
        "shop-api-redis-1",
        "shop-api-db-1",
    ];
    let found: Vec<Container> = names.iter().map(|n| container(n)).collect();
    let refs: Vec<&Container> = found.iter().collect();
    let effect = delete_effect(&refs, &volumes());
    assert_eq!(effect.count, 4);
    assert_eq!(button_label(&effect), "Stop and delete 4 containers");
    assert_eq!(result_label(&effect, 4), "Deleted 4 containers");
    // Three failed to delete? The result says what went through.
    assert_eq!(result_label(&effect, 3), "Deleted 3 containers");
    assert_eq!(
        texts(&effect, EffectKind::Changes),
        ["3 running containers are stopped first."]
    );
    assert_eq!(
        texts(&effect, EffectKind::Goes),
        ["Their writable layers are deleted."]
    );

    // Only stopped containers: a plain "Delete N containers".
    let stopped = [container("shop-api-db-1"), container("analytics-grafana-1")];
    let refs: Vec<&Container> = stopped.iter().collect();
    let effect = delete_effect(&refs, &volumes());
    assert_eq!(button_label(&effect), "Delete 2 containers");
}

#[test]
fn confirm_anonymous_volumes_are_not_listed_as_kept() {
    let mut db = container("shop-api-db-1");
    let anonymous = "a".repeat(64);
    db.mounts.push(berth_core::Mount {
        kind: berth_core::MountKind::Volume,
        source: String::new(),
        destination: "/tmp".to_string(),
        read_only: false,
        volume_name: Some(anonymous.clone()),
    });
    assert!(is_anonymous_volume(&anonymous));
    assert!(!is_anonymous_volume("shop-api_pgdata"));
    let effect = delete_effect(&[&db], &volumes());
    let kept = texts(&effect, EffectKind::Stays);
    assert_eq!(kept, ["Volume shop-api_pgdata (1.2 GB) is kept."]);
}

#[test]
fn confirm_delete_checkbox_names_the_named_volumes_it_leaves_alone() {
    let named = |db: &mut Container, name: &str| {
        db.mounts.push(berth_core::Mount {
            kind: berth_core::MountKind::Volume,
            source: String::new(),
            destination: "/x".to_string(),
            read_only: false,
            volume_name: Some(name.to_string()),
        });
    };
    let db = container("shop-api-db-1");
    assert_eq!(
        delete_checkbox_label(&[&db]),
        "Also delete anonymous volumes (not shop-api_pgdata)"
    );
    // Nothing named to keep: no parenthesis. Anonymous volumes are not
    // "kept" names (they are what the box deletes).
    let mut plain = container("shop-api-api-1");
    plain.mounts.clear();
    assert_eq!(
        delete_checkbox_label(&[&plain]),
        "Also delete anonymous volumes"
    );
    named(&mut plain, &"b".repeat(64));
    assert_eq!(
        delete_checkbox_label(&[&plain]),
        "Also delete anonymous volumes"
    );
    // Several names are listed; past three they are counted.
    let mut many = container("shop-api-db-1");
    named(&mut many, "v2");
    assert_eq!(
        delete_checkbox_label(&[&many]),
        "Also delete anonymous volumes (not shop-api_pgdata, v2)"
    );
    for name in ["v3", "v4"] {
        named(&mut many, name);
    }
    assert_eq!(
        delete_checkbox_label(&[&many]),
        "Also delete anonymous volumes (not 4 named volumes)"
    );
}

#[test]
fn confirm_many_kept_volumes_are_summed_up() {
    let mut db = container("shop-api-db-1");
    for name in ["v1", "v2", "v3", "v4"] {
        db.mounts.push(berth_core::Mount {
            kind: berth_core::MountKind::Volume,
            source: String::new(),
            destination: format!("/{name}"),
            read_only: false,
            volume_name: Some(name.to_string()),
        });
    }
    let effect = delete_effect(&[&db], &volumes());
    assert_eq!(
        texts(&effect, EffectKind::Stays),
        ["5 volumes are kept (1.2 GB)."]
    );
}

#[test]
fn confirm_kill_and_engine_summaries_keep_their_words() {
    let api = container("shop-api-api-1");
    let kill = kill_effect(&api);
    assert_eq!(kill.verb, "Kill");
    assert_eq!(button_label(&kill), "Kill container");
    assert_eq!(
        texts(&kill, EffectKind::Changes),
        ["shop-api-api-1 stops at once without a clean shutdown"]
    );
    let stop = engine_effect(true, 15);
    assert_eq!(button_label(&stop), "Stop engine");
    assert_eq!(
        texts(&stop, EffectKind::Changes),
        ["15 running containers stop while the engine is stopped"]
    );
    let restart = engine_effect(false, 1);
    assert_eq!(
        texts(&restart, EffectKind::Changes),
        ["1 running container stop while the engine restarts"]
    );
}

#[test]
fn confirm_typed_name_unlocks_only_on_an_exact_match() {
    let mut effect = delete_effect(&[&container("shop-api-db-1")], &volumes());
    assert!(crate::ui::confirm_dialog::name_matches(&effect, ""));
    effect.typed_name = Some("shop-api-db-1".to_string());
    assert!(!crate::ui::confirm_dialog::name_matches(&effect, ""));
    assert!(!crate::ui::confirm_dialog::name_matches(
        &effect,
        "shop-api-db"
    ));
    assert!(crate::ui::confirm_dialog::name_matches(
        &effect,
        "shop-api-db-1"
    ));
}

fn render(model: &ConfirmModel) -> String {
    #[component]
    fn Host(model: ConfirmModel) -> Element {
        rsx! {
            crate::ui::ConfirmDialog {
                title: model.title.clone(),
                effect: model.effect.clone(),
                checkbox: model.checkbox.clone(),
                oncheck: |_| {},
                oncancel: |()| {},
                onconfirm: |()| {},
            }
        }
    }
    let mut dom = VirtualDom::new_with_props(
        Host,
        HostProps {
            model: model.clone(),
        },
    );
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

fn strip_styles(html: &str) -> String {
    let mut out = html.to_string();
    while let (Some(a), Some(b)) = (out.find("<style>"), out.find("</style>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</style>".len(), "");
    }
    out
}

fn sample_model(effect: EffectSummary) -> ConfirmModel {
    ConfirmModel {
        title: "Delete volume shop-api_pgdata?".to_string(),
        effect,
        action: EngineAction::Container {
            engine: default_engine(),
            id: ContainerId("x".to_string()),
            action: ContainerAction::Remove {
                force: false,
                volumes: false,
            },
        },
        checkbox: None,
        targets: vec![berth_core::Subject::Container(ContainerId("x".to_string()))],
    }
}

#[test]
fn confirm_dialog_with_a_typed_name_disables_the_button_with_the_reason() {
    let mut effect = delete_effect(&[&container("shop-api-db-1")], &volumes());
    effect.typed_name = Some("shop-api_pgdata".to_string());
    let html = strip_styles(&render(&sample_model(effect)));
    assert!(html.contains("Type shop-api_pgdata to confirm"), "{html}");
    let button = html
        .split("<button")
        .find(|b| b.contains("btn-danger"))
        .expect("danger button");
    assert!(button.contains("disabled"), "{button}");
    assert!(
        button.contains("data-tooltip=\"Type shop-api_pgdata to confirm\""),
        "{button}"
    );
    // The field is uncontrolled: no `value` attribute bound to a signal.
    let input = html
        .split("<input")
        .nth(1)
        .expect("typed-name input")
        .split('>')
        .next()
        .unwrap_or_default();
    assert!(!input.contains(" value="), "{input}");
}

#[test]
fn confirm_dialog_without_a_typed_name_has_no_field_and_an_enabled_button() {
    let effect = delete_effect(&[&container("shop-api-db-1")], &volumes());
    let html = strip_styles(&render(&sample_model(effect)));
    assert!(!html.contains("<input"), "{html}");
    let button = html
        .split("<button")
        .find(|b| b.contains("btn-danger"))
        .expect("danger button");
    assert!(!button.contains("disabled"), "{button}");
    assert!(button.contains("Delete container"), "{button}");
}

#[test]
fn confirm_delete_scene_renders_the_frame_dialog() {
    let html = strip_styles(
        &crate::scenes::render_scene_html("Containers / confirm delete / 1440")
            .expect("scene exists"),
    );
    assert!(html.contains("Delete container shop-api-db-1?"), "{html}");
    assert!(html.contains("Its writable layer is deleted."), "{html}");
    assert!(
        html.contains("Volume shop-api_pgdata (1.2 GB) is kept."),
        "{html}"
    );
    assert!(html.contains("Removes"));
    assert!(html.contains("Keeps"));
    assert!(
        html.contains("Also delete anonymous volumes (not shop-api_pgdata)"),
        "{html}"
    );
    assert!(html.contains("aria-checked=\"false\""), "{html}");
    assert!(html.contains("Cancel"));
    assert!(html.contains("Delete container"));
    assert!(html.contains("role=\"dialog\""));
}

// ---------------------------------------------------------------------
// With signals: models from state, keys, running
// ---------------------------------------------------------------------

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

struct Rig {
    dom: VirtualDom,
    state: AppState,
    store: Arc<berth_store::Store>,
    engine: Arc<berth_engine::MockEngine>,
    service: ActionsService,
    finished: tokio::sync::mpsc::UnboundedReceiver<crate::services::actions::Finished>,
}

fn rig() -> Rig {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    let world = world();
    let clients = crate::services::engines::client_map();
    let engine = Arc::new(berth_engine::MockEngine::bundled(&default_engine()));
    clients.lock().expect("clients").insert(
        default_engine(),
        Arc::clone(&engine) as Arc<dyn berth_engine::EngineClient>,
    );
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let (service, finished) = ActionsService::new(
        clients,
        Arc::clone(&store),
        Arc::new(berth_engine::MockCompose::bundled()),
        Arc::new(berth_engine::MockColima::bundled()),
        tokio_util::sync::CancellationToken::new(),
    );
    dom.in_runtime(|| {
        let mut state = state;
        state.engines.set(world.engines.clone());
        state
            .containers
            .set(world.containers_for(&default_engine()));
        state.volumes.set(volumes());
        state.section.set(crate::state::Section::Containers);
    });
    Rig {
        dom,
        state,
        store,
        engine,
        service,
        finished,
    }
}

impl Rig {
    fn id(&self, name: &str) -> ContainerId {
        self.dom.in_runtime(|| {
            self.state
                .containers
                .peek()
                .iter()
                .find(|c| c.name == name)
                .map(|c| c.id.clone())
                .unwrap_or_else(|| panic!("{name} in state"))
        })
    }

    fn select(&self, names: &[&str]) {
        let ids: Vec<ContainerId> = names.iter().map(|n| self.id(n)).collect();
        self.dom.in_runtime(|| {
            let mut state = self.state;
            state.selection.set(Selection::Containers(ids));
        });
    }

    fn overlays(&self) -> Vec<Overlay> {
        self.dom.in_runtime(|| self.state.overlays.peek().clone())
    }

    fn press_backspace(&self) {
        self.dom.in_runtime(|| {
            let mut state = self.state;
            crate::keys::dispatch(&mut state, "container.delete");
        });
    }
}

#[test]
fn confirm_backspace_opens_the_delete_dialog_for_the_selection() {
    let rig = rig();
    rig.select(&["shop-api-db-1"]);
    rig.press_backspace();
    match rig.overlays().as_slice() {
        [Overlay::Confirm(model)] => {
            assert_eq!(model.title, "Delete container shop-api-db-1?");
            assert_eq!(
                model.checkbox,
                Some((
                    "Also delete anonymous volumes (not shop-api_pgdata)".to_string(),
                    false
                ))
            );
            assert!(matches!(
                &model.action,
                EngineAction::Container {
                    action: ContainerAction::Remove {
                        force: false,
                        volumes: false
                    },
                    ..
                }
            ));
        }
        other => panic!("expected one confirm overlay, got {other:?}"),
    }
}

#[test]
fn confirm_backspace_with_several_selected_opens_one_bulk_dialog() {
    let rig = rig();
    rig.select(&["shop-api-api-1", "shop-api-redis-1", "shop-api-db-1"]);
    rig.press_backspace();
    match rig.overlays().as_slice() {
        [Overlay::Confirm(model)] => {
            assert_eq!(model.title, "Delete 3 containers?");
            assert_eq!(model.effect.count, 3);
            // No Prune: the model is a container action on the first one.
            assert!(
                matches!(
                    &model.action,
                    EngineAction::Container {
                        action: ContainerAction::Remove { .. },
                        ..
                    }
                ),
                "{:?}",
                model.action
            );
        }
        other => panic!("expected one confirm overlay, got {other:?}"),
    }
}

#[test]
fn confirm_backspace_does_nothing_without_a_selection_or_over_another_overlay() {
    let rig = rig();
    rig.press_backspace();
    assert!(rig.overlays().is_empty());
    rig.select(&["shop-api-db-1"]);
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        state.overlays.write().push(Overlay::Palette);
    });
    rig.press_backspace();
    assert_eq!(rig.overlays(), [Overlay::Palette]);
}

#[test]
fn confirm_checkbox_decides_whether_anonymous_volumes_go() {
    let action = ContainerAction::Remove {
        force: true,
        volumes: false,
    };
    assert_eq!(
        with_volumes(&action, true),
        ContainerAction::Remove {
            force: true,
            volumes: true
        }
    );
    assert_eq!(with_volumes(&action, false), action);
    assert_eq!(
        with_volumes(&ContainerAction::Kill { signal: None }, true),
        ContainerAction::Kill { signal: None }
    );
}

#[tokio::test]
async fn confirm_run_deletes_through_the_actions_service_and_the_result_repeats_the_count() {
    let mut rig = rig();
    rig.select(&["eager_hopper"]);
    rig.press_backspace();
    let Some(Overlay::Confirm(model)) = rig.overlays().into_iter().next() else {
        panic!("a confirm overlay");
    };
    let mut state = rig.state;
    let launched = rig
        .dom
        .in_runtime(|| run(&mut state, &rig.service, &model, false));
    assert_eq!(launched.refused, Vec::<String>::new());
    assert_eq!(launched.keys.len(), 1);
    assert_eq!(launched.keys[0].action, "container.delete");
    // Optimistic: pending before the engine answers (D34).
    let pending = rig.dom.in_runtime(|| state.pending.peek().clone());
    assert_eq!(finished_count(&pending, &launched.keys), None);
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), rig.finished.recv())
        .await
        .expect("the delete finishes")
        .expect("channel open");
    assert_eq!(done.result, Ok(()));
    rig.dom
        .in_runtime(|| crate::services::actions::apply_finished(&mut state, done));
    let pending = rig.dom.in_runtime(|| state.pending.peek().clone());
    let done = finished_count(&pending, &launched.keys).expect("nothing runs now");
    assert_eq!(done, 1);
    assert_eq!(
        result_label(&model.effect, done),
        "Deleted 1 container",
        "the result repeats the dialog's count"
    );
    assert_eq!(button_label(&model.effect), "Delete container");
}

#[tokio::test]
async fn confirm_bulk_delete_runs_one_container_action_per_selected_container() {
    use berth_engine::EngineClient as _;
    let mut rig = rig();
    // A running one (needs force), an exited one and a third.
    let names = ["shop-api-api-1", "shop-api-db-1", "eager_hopper"];
    rig.select(&names);
    rig.press_backspace();
    let Some(Overlay::Confirm(model)) = rig.overlays().into_iter().next() else {
        panic!("a confirm overlay");
    };
    assert_eq!(model.effect.count, 3);
    let mut state = rig.state;
    let launched = rig
        .dom
        .in_runtime(|| run(&mut state, &rig.service, &model, false));
    assert_eq!(launched.refused, Vec::<String>::new());
    // One pending key per container, all container.delete, all distinct.
    assert_eq!(launched.keys.len(), 3);
    assert!(launched.keys.iter().all(|k| k.action == "container.delete"));
    let subjects: Vec<_> = launched.keys.iter().map(|k| k.subject.clone()).collect();
    for name in names {
        let id = rig.id(name);
        assert!(
            subjects.contains(&berth_core::Subject::Container(id)),
            "{name} has its own pending key"
        );
    }
    let pending = rig.dom.in_runtime(|| state.pending.peek().clone());
    assert_eq!(finished_count(&pending, &launched.keys), None);
    for _ in 0..3 {
        let done = tokio::time::timeout(std::time::Duration::from_secs(5), rig.finished.recv())
            .await
            .expect("each delete finishes")
            .expect("channel open");
        assert_eq!(done.result, Ok(()));
        rig.dom
            .in_runtime(|| crate::services::actions::apply_finished(&mut state, done));
    }
    let pending = rig.dom.in_runtime(|| state.pending.peek().clone());
    let done = finished_count(&pending, &launched.keys).expect("nothing runs now");
    assert_eq!(done, 3);
    assert_eq!(result_label(&model.effect, done), "Deleted 3 containers");
    // The engine no longer has them, and each has its own activity entry.
    let left: Vec<String> = rig
        .engine
        .containers()
        .await
        .expect("containers")
        .into_iter()
        .map(|c| c.name)
        .collect();
    for name in names {
        assert!(!left.iter().any(|n| n == name), "{name} still exists");
    }
    let entries = rig
        .store
        .activity_query(&berth_store::ActivityFilter::default(), None, 10)
        .expect("activity");
    let deleted: Vec<&str> = entries
        .iter()
        .filter(|e| e.action == "container.delete")
        .map(|e| e.subject_name.as_str())
        .collect();
    assert_eq!(deleted.len(), 3, "{deleted:?}");
    for name in names {
        assert!(deleted.contains(&name), "{name} in {deleted:?}");
    }
}

#[tokio::test]
async fn confirm_bulk_delete_skips_a_container_that_vanished_behind_the_dialog() {
    let mut rig = rig();
    rig.select(&["eager_hopper", "shop-api-db-1"]);
    rig.press_backspace();
    let Some(Overlay::Confirm(model)) = rig.overlays().into_iter().next() else {
        panic!("a confirm overlay");
    };
    let gone = rig.id("shop-api-db-1");
    let mut state = rig.state;
    rig.dom.in_runtime(|| {
        state.containers.write().retain(|c| c.id != gone);
    });
    let launched = rig
        .dom
        .in_runtime(|| run(&mut state, &rig.service, &model, false));
    assert_eq!(launched.keys.len(), 1);
    assert_eq!(
        launched.refused,
        ["This container no longer exists"],
        "the vanished target is a refused line"
    );
    let done = rig.finished.recv().await.expect("one result");
    rig.dom
        .in_runtime(|| crate::services::actions::apply_finished(&mut state, done));
    let pending = rig.dom.in_runtime(|| state.pending.peek().clone());
    // The result says what went through, not the dialog's 2.
    let n = finished_count(&pending, &launched.keys).expect("done");
    assert_eq!(result_label(&model.effect, n), "Deleted 1 container");
}

fn container_subjects(rig: &Rig, names: &[&str]) -> Vec<berth_core::Subject> {
    names
        .iter()
        .map(|n| berth_core::Subject::Container(rig.id(n)))
        .collect()
}

#[test]
fn confirm_every_builder_fills_targets_that_match_its_count() {
    let rig = rig();
    // Delete, one and several.
    for names in [
        &["shop-api-db-1"][..],
        &["shop-api-api-1", "shop-api-redis-1", "shop-api-db-1"][..],
    ] {
        let ids: Vec<ContainerId> = names.iter().map(|n| rig.id(n)).collect();
        let model = rig
            .dom
            .in_runtime(|| delete_model(&rig.state, &ids).expect("delete model"));
        assert_eq!(model.effect.count as usize, model.targets.len());
        assert_eq!(model.targets, container_subjects(&rig, names));
    }
    // Kill.
    let id = rig.id("shop-api-api-1");
    let kill = rig
        .dom
        .in_runtime(|| kill_model(&rig.state, &id).expect("kill model"));
    assert_eq!(kill.effect.count as usize, kill.targets.len());
    assert_eq!(kill.targets, [berth_core::Subject::Container(id)]);
    // Engine stop and restart.
    for op in [berth_core::ColimaOp::Stop, berth_core::ColimaOp::Restart] {
        let model = engine_model(&default_engine(), "default", "default".to_string(), op, 3);
        assert_eq!(model.effect.count as usize, model.targets.len());
        assert_eq!(
            model.targets,
            [berth_core::Subject::Engine(default_engine())]
        );
    }
}

#[test]
fn confirm_refuses_targets_that_are_not_the_ones_its_action_names() {
    let mut rig = rig();
    let api = rig.id("shop-api-api-1");
    let db = rig.id("shop-api-db-1");
    let mut state = rig.state;
    // A kill dialog for one container, its target swapped for another.
    let mut kill = rig
        .dom
        .in_runtime(|| kill_model(&rig.state, &api).expect("kill model"));
    kill.targets = vec![berth_core::Subject::Container(db)];
    let launched = rig
        .dom
        .in_runtime(|| run(&mut state, &rig.service, &kill, false));
    assert!(launched.keys.is_empty(), "{:?}", launched.keys);
    assert_eq!(launched.refused, ["This is not the container to act on"]);
    // An engine stop whose target is some other engine.
    let mut stop = engine_model(
        &default_engine(),
        "default",
        "default".to_string(),
        berth_core::ColimaOp::Stop,
        3,
    );
    stop.targets = vec![berth_core::Subject::Engine(berth_core::EngineId(
        "colima:other".to_string(),
    ))];
    let launched = rig
        .dom
        .in_runtime(|| run(&mut state, &rig.service, &stop, false));
    assert!(launched.keys.is_empty(), "{:?}", launched.keys);
    assert_eq!(launched.refused.len(), 1);
    rig.state = state;
}

#[tokio::test]
async fn confirm_bulk_delete_acts_on_its_own_targets_when_the_selection_changes() {
    let mut rig = rig();
    rig.select(&["shop-api-api-1", "shop-api-db-1"]);
    rig.press_backspace();
    let Some(Overlay::Confirm(model)) = rig.overlays().into_iter().next() else {
        panic!("a confirm overlay");
    };
    // After the dialog opened, the selection moves to something else.
    rig.select(&["eager_hopper"]);
    let mut state = rig.state;
    let launched = rig
        .dom
        .in_runtime(|| run(&mut state, &rig.service, &model, false));
    assert_eq!(launched.refused, Vec::<String>::new());
    let subjects: Vec<_> = launched.keys.iter().map(|k| k.subject.clone()).collect();
    assert_eq!(
        subjects,
        container_subjects(&rig, &["shop-api-api-1", "shop-api-db-1"]),
        "the dialog acts on the containers it counted"
    );
    for _ in 0..2 {
        let done = tokio::time::timeout(std::time::Duration::from_secs(5), rig.finished.recv())
            .await
            .expect("each delete finishes")
            .expect("channel open");
        assert_eq!(done.result, Ok(()));
        rig.dom
            .in_runtime(|| crate::services::actions::apply_finished(&mut state, done));
    }
    use berth_engine::EngineClient as _;
    let left: Vec<String> = rig
        .engine
        .containers()
        .await
        .expect("containers")
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert!(!left.iter().any(|n| n == "shop-api-api-1"));
    assert!(!left.iter().any(|n| n == "shop-api-db-1"));
    assert!(
        left.iter().any(|n| n == "eager_hopper"),
        "the newly selected container is untouched"
    );
}

#[test]
fn confirm_selection_cannot_move_while_a_dialog_is_up() {
    let rig = rig();
    rig.select(&["shop-api-api-1", "shop-api-db-1"]);
    rig.press_backspace();
    let before = rig.dom.in_runtime(|| rig.state.selection.peek().clone());
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        crate::keys::dispatch(&mut state, "global.select_next");
    });
    let after = rig.dom.in_runtime(|| rig.state.selection.peek().clone());
    assert_eq!(before, after);
}

#[test]
fn confirm_finished_count_waits_for_running_and_skips_failures() {
    use crate::state::PendingState;
    let key = |n: &str| PendingKey {
        subject: berth_core::Subject::Container(ContainerId(n.to_string())),
        action: "container.delete",
    };
    let keys = vec![key("a"), key("b"), key("c")];
    let mut pending = std::collections::BTreeMap::new();
    pending.insert(
        key("a"),
        PendingState::Running {
            label: "Deleting…".to_string(),
        },
    );
    assert_eq!(finished_count(&pending, &keys), None);
    pending.insert(
        key("a"),
        PendingState::Failed {
            message: "boom".to_string(),
        },
    );
    assert_eq!(finished_count(&pending, &keys), Some(2));
}
