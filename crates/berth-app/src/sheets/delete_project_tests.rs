//! P5-T6 tests for the Delete project sheet: the effect text under each
//! combination of the two checkboxes, the name that unlocks the button,
//! and what Delete sends to Compose. Compose is the recording mock; no
//! container, volume or image of any engine is touched.

use std::sync::Arc;

use berth_core::{ComposeCommand, EffectKind, Project};

use super::*;
use crate::scenes;
use crate::services::actions::ActionsService;
use crate::services::confirm::{result_label, ProjectHoldings};
use crate::state::{use_app_state, Overlay, PendingState};

fn world() -> berth_engine::MockWorld {
    berth_engine::MockWorld::bundled()
}

fn project_named(world: &berth_engine::MockWorld, name: &str) -> Project {
    world
        .projects
        .iter()
        .find(|p| p.name == name)
        .cloned()
        .expect("mock project")
}

fn holdings_of(world: &berth_engine::MockWorld, project: &Project) -> ProjectHoldings {
    let volumes: Vec<berth_core::Volume> = world
        .volumes_by_engine
        .values()
        .flatten()
        .cloned()
        .collect();
    project_holdings(project, &world.containers, &volumes)
}

fn kinds(effect: &berth_core::EffectSummary) -> Vec<EffectKind> {
    effect.lines.iter().map(|l| l.kind).collect()
}

#[test]
fn project_delete_effect_with_both_boxes_off_removes_containers_and_keeps_the_rest() {
    let world = world();
    let project = project_named(&world, "shop-api");
    let holdings = holdings_of(&world, &project);
    let effect = project_delete_effect(
        &project,
        &holdings,
        Some("~/Dev/clients/acme/shop-api"),
        DeleteOptions::default(),
    );
    assert_eq!(
        kinds(&effect),
        [
            EffectKind::Goes,
            EffectKind::Stays,
            EffectKind::Stays,
            EffectKind::Stays
        ]
    );
    assert!(
        effect.lines[0].text.starts_with("5 containers ("),
        "{effect:?}"
    );
    assert!(effect.lines[0].text.contains("shop-api-api-1"));
    assert!(effect.lines[0]
        .text
        .ends_with("and the project's networks."));
    assert!(effect.lines[1].text.contains("shop-api_pgdata"));
    assert!(effect.lines[1].text.contains("with their data"));
    assert!(effect.lines[2].text.contains("postgres:17-alpine"));
    assert!(effect.lines[3].text.contains("~/Dev/clients/acme/shop-api"));
    assert_eq!(effect.typed_name, None, "nothing irreplaceable goes");
    assert_eq!(effect.bytes, None);
    assert_eq!(confirm::button_label(&effect), "Delete project");
    assert_eq!(result_label(&effect, 1), "Deleted 1 project");
}

#[test]
fn project_delete_effect_with_volumes_ticked_removes_them_and_asks_for_the_name() {
    let world = world();
    let project = project_named(&world, "shop-api");
    let holdings = holdings_of(&world, &project);
    let effect = project_delete_effect(
        &project,
        &holdings,
        Some("~/p"),
        DeleteOptions {
            volumes: true,
            remove_images: false,
        },
    );
    assert_eq!(
        kinds(&effect),
        [
            EffectKind::Goes,
            EffectKind::Goes,
            EffectKind::Stays,
            EffectKind::Stays
        ]
    );
    assert!(effect.lines[1].text.contains("shop-api_pgdata"));
    assert_eq!(effect.typed_name.as_deref(), Some("shop-api"));
    assert!(confirm::button_label(&effect).starts_with("Delete project"));
}

#[test]
fn project_delete_effect_with_images_ticked_removes_images_and_keeps_volumes() {
    let world = world();
    let project = project_named(&world, "shop-api");
    let holdings = holdings_of(&world, &project);
    let effect = project_delete_effect(
        &project,
        &holdings,
        Some("~/p"),
        DeleteOptions {
            volumes: false,
            remove_images: true,
        },
    );
    assert_eq!(
        kinds(&effect),
        [
            EffectKind::Goes,
            EffectKind::Stays,
            EffectKind::Goes,
            EffectKind::Stays
        ]
    );
    assert!(
        effect.lines[2].text.contains("again when needed"),
        "{effect:?}"
    );
    assert_eq!(effect.typed_name, None, "images can be pulled again");
}

#[test]
fn project_delete_effect_with_both_ticked_removes_both_and_still_keeps_the_folder() {
    let world = world();
    let project = project_named(&world, "shop-api");
    let holdings = holdings_of(&world, &project);
    let effect = project_delete_effect(
        &project,
        &holdings,
        Some("~/Dev/clients/acme/shop-api"),
        DeleteOptions {
            volumes: true,
            remove_images: true,
        },
    );
    assert_eq!(
        kinds(&effect),
        [
            EffectKind::Goes,
            EffectKind::Goes,
            EffectKind::Goes,
            EffectKind::Stays
        ]
    );
    let folder = effect.lines.last().expect("folder line");
    assert_eq!(folder.kind, EffectKind::Stays);
    assert!(folder.text.contains("~/Dev/clients/acme/shop-api"));
    assert_eq!(effect.typed_name.as_deref(), Some("shop-api"));
}

#[test]
fn project_delete_folder_is_kept_whatever_is_ticked() {
    let world = world();
    let project = project_named(&world, "shop-api");
    let holdings = holdings_of(&world, &project);
    for volumes in [false, true] {
        for remove_images in [false, true] {
            let effect = project_delete_effect(
                &project,
                &holdings,
                Some("~/p"),
                DeleteOptions {
                    volumes,
                    remove_images,
                },
            );
            assert!(
                !effect
                    .lines
                    .iter()
                    .any(|l| l.kind == EffectKind::Goes && l.text.contains("~/p")),
                "the folder never goes: {effect:?}"
            );
            assert!(effect
                .lines
                .iter()
                .any(|l| l.kind == EffectKind::Stays && l.text.contains("~/p")));
        }
    }
}

#[test]
fn project_delete_without_volumes_asks_for_no_name_even_when_ticked() {
    let world = world();
    // A project that owns no volume (the mock has one or more).
    let project = world
        .projects
        .iter()
        .find(|p| !p.services.is_empty() && holdings_of(&world, p).volumes.is_empty())
        .cloned()
        .expect("a mock project without volumes");
    let holdings = holdings_of(&world, &project);
    let effect = project_delete_effect(
        &project,
        &holdings,
        None,
        DeleteOptions {
            volumes: true,
            remove_images: false,
        },
    );
    assert_eq!(effect.typed_name, None);
    assert!(!effect.lines.iter().any(|l| l.text.contains("volume")));
    let [volumes_label, _] = delete_option_labels(&holdings);
    assert!(volumes_label.contains("has none"), "{volumes_label}");
    assert!(effect
        .lines
        .last()
        .is_some_and(|l| l.text.contains("nothing on this Mac is touched")));
}

#[test]
fn project_delete_blocker_says_the_engine_reason_first_then_the_name_to_type() {
    let world = world();
    let project = project_named(&world, "shop-api");
    let holdings = holdings_of(&world, &project);
    let both = project_delete_effect(
        &project,
        &holdings,
        None,
        DeleteOptions {
            volumes: true,
            remove_images: false,
        },
    );
    assert_eq!(
        delete_blocker(&both, "", None).as_deref(),
        Some("Type shop-api to confirm")
    );
    assert_eq!(
        delete_blocker(&both, "shop", None).as_deref(),
        Some("Type shop-api to confirm")
    );
    assert_eq!(delete_blocker(&both, "shop-api", None), None);
    assert_eq!(
        delete_blocker(&both, "shop-api", Some("Engine is stopped".into())).as_deref(),
        Some("Engine is stopped")
    );
    let plain = project_delete_effect(&project, &holdings, None, DeleteOptions::default());
    assert_eq!(delete_blocker(&plain, "", None), None);
}

#[test]
fn project_delete_option_maps_to_compose_down_with_the_same_flags() {
    for volumes in [false, true] {
        for remove_images in [false, true] {
            let op = delete_op(DeleteOptions {
                volumes,
                remove_images,
            });
            assert_eq!(
                op.command(),
                ComposeCommand::Down {
                    volumes,
                    remove_images
                }
            );
            assert_eq!(op.action_id(), "project.delete");
        }
    }
}

#[test]
fn project_delete_scene_draws_the_sheet_with_both_boxes_off_and_no_name_field() {
    let html = scenes::render_scene_html("Project / delete / 1440").expect("scene");
    assert!(
        html.contains("aria-label=\"Delete project shop-api?\""),
        "{html}"
    );
    assert!(html.contains("Delete project shop-api?"));
    assert!(html.contains("Also delete its 2 volumes and the data in them"));
    assert!(html.contains("Also remove the images its services use"));
    assert!(
        !html.contains("aria-label=\"Type shop-api to confirm\""),
        "the name is asked only once volumes are ticked"
    );
    // Both of the dialog's checkboxes start unchecked.
    let dialog = &html[html.find("class=\"confirm-dialog\"").expect("the dialog")..];
    assert_eq!(dialog.matches("role=\"checkbox\"").count(), 2);
    assert_eq!(dialog.matches("aria-checked=\"true\"").count(), 0);
    let (before, _) = html
        .rsplit_once("class=\"btn-label\">Delete project<")
        .expect("the sheet's button");
    let button = before.rsplit("<button").next().expect("button");
    assert!(!button.contains("disabled"), "{button}");
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

struct Rig {
    dom: VirtualDom,
    state: AppState,
    service: ActionsService,
    compose: Arc<berth_engine::MockCompose>,
    finished: tokio::sync::mpsc::UnboundedReceiver<crate::services::actions::Finished>,
    project: Project,
}

fn rig() -> Rig {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    let world = world();
    let project = project_named(&world, "shop-api");
    dom.in_runtime(|| {
        let mut state = state;
        state.engines.set(world.engines.clone());
        state.containers.set(world.containers.clone());
        state.projects.set(world.projects.clone());
    });
    let clients = crate::services::engines::client_map();
    let engine_id = berth_core::EngineId("colima:default".to_string());
    clients.lock().expect("clients").insert(
        engine_id.clone(),
        Arc::new(berth_engine::MockEngine::bundled(&engine_id))
            as Arc<dyn berth_engine::EngineClient>,
    );
    let compose = Arc::new(berth_engine::MockCompose::bundled());
    let (service, finished) = ActionsService::new(
        clients,
        Arc::new(berth_store::Store::open_in_memory().expect("store")),
        Arc::clone(&compose) as Arc<dyn berth_engine::ComposeRunner>,
        Arc::new(berth_engine::MockColima::bundled()),
        tokio_util::sync::CancellationToken::new(),
    );
    Rig {
        dom,
        state,
        service,
        compose,
        finished,
        project,
    }
}

#[tokio::test]
async fn project_delete_confirm_runs_down_with_exactly_the_ticked_options_and_closes_the_sheet() {
    for volumes in [false, true] {
        for remove_images in [false, true] {
            let mut rig = rig();
            let id = rig.project.id.clone();
            let options = DeleteOptions {
                volumes,
                remove_images,
            };
            let launched = rig.dom.in_runtime(|| {
                let mut state = rig.state;
                state
                    .overlays
                    .write()
                    .push(Overlay::DeleteProject(id.clone()));
                let launched = confirm_delete(&mut state, Some(&rig.service), &id, options);
                assert!(state.overlays.peek().is_empty(), "the sheet closes");
                launched
            });
            assert_eq!(launched.keys, [delete_key(&id)]);
            assert!(launched.refused.is_empty());
            let running = rig.dom.in_runtime(|| {
                matches!(
                    rig.state.pending.peek().get(&delete_key(&id)),
                    Some(PendingState::Running { .. })
                )
            });
            assert!(running, "Deleting\u{2026} shows while it runs");

            let done = tokio::time::timeout(std::time::Duration::from_secs(5), rig.finished.recv())
                .await
                .expect("the delete finishes")
                .expect("channel open");
            assert_eq!(done.result, Ok(()));
            let recorded = rig.compose.recorded();
            assert_eq!(recorded.len(), 1, "{recorded:?}");
            assert_eq!(
                recorded[0].1,
                ComposeCommand::Down {
                    volumes,
                    remove_images
                }
            );
            assert_eq!(recorded[0].0.project_name, "shop-api");
        }
    }
}

#[tokio::test]
async fn project_delete_confirm_without_a_service_runs_nothing_and_leaves_no_command() {
    let rig = rig();
    let id = rig.project.id.clone();
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        state
            .overlays
            .write()
            .push(Overlay::DeleteProject(id.clone()));
        let _ = confirm_delete(&mut state, None, &id, DeleteOptions::default());
        assert!(state.overlays.peek().is_empty());
    });
    assert!(rig.compose.recorded().is_empty());
}

#[test]
fn project_delete_holdings_find_a_live_volume_by_its_compose_label() {
    let world = world();
    let project = project_named(&world, "shop-api");
    let volume = |name: &str, label: Option<&str>| {
        let mut v = world
            .volumes_by_engine
            .values()
            .flatten()
            .next()
            .cloned()
            .expect("a mock volume");
        v.name = name.to_string();
        v.project = None;
        v.engine = project.engine.clone();
        v.labels.clear();
        if let Some(label) = label {
            v.labels
                .insert("com.docker.compose.project".to_string(), label.to_string());
        }
        v
    };
    let volumes = [
        volume("shop-api_data", Some("shop-api")),
        volume("other_data", Some("other")),
        volume("loose", None),
    ];
    let holdings = project_holdings(&project, &world.containers, &volumes);
    let names: Vec<&str> = holdings.volumes.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["shop-api_data"]);
}
