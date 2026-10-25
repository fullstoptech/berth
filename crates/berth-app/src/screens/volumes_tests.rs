//! P4-T4 tests for the Volumes screen: the scene renders the table the
//! task lists, the delete dialog comes from the one confirm builder with a
//! typed name and `EngineAction::Remove`, a confirmed dialog reaches the
//! engine, and a volume in use cannot be deleted.

use std::sync::Arc;

use berth_core::{EngineAction, EngineId, ResourceRef};

use super::*;
use crate::scenes;
use crate::services::actions::ActionsService;
use crate::state::use_app_state;

fn default_engine() -> EngineId {
    EngineId("colima:default".to_string())
}

fn scene(frame: &str) -> String {
    scenes::render_scene_html(frame).expect("the scene exists")
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

fn markup(frame: &str) -> String {
    strip_styles(&scene(frame))
}

/// The `<div class="vl-row ...">` chunk of one volume.
fn row_of<'a>(html: &'a str, name: &str) -> &'a str {
    let key = format!("title=\"{name}\"");
    let at = html.find(&key).unwrap_or_else(|| panic!("row of {name}"));
    let start = html[..at].rfind("class=\"vl-row").expect("row start");
    let end = html[at..].find("vt-row").map_or(html.len(), |e| at + e);
    &html[start..end]
}

#[test]
fn volumes_list_scene_renders_the_header_and_the_twelve_rows() {
    let html = markup("Volumes / list / 1440");
    for needle in [
        "<h1 class=\"vl-title\">Volumes</h1>",
        "badge badge-count\">12<",
        "Filter volumes",
        ">Unused<",
        ">Create volume<",
        ">Name<",
        ">Project<",
        ">Size<",
        ">Used by<",
        ">Driver<",
        ">Created<",
    ] {
        assert!(html.contains(needle), "missing {needle}");
    }
    assert_eq!(html.matches("class=\"vl-row").count(), 12);
    // Sizes the frames' data carries; the used-by cell names the container.
    let pg = row_of(&html, "shop-api_pgdata");
    assert!(pg.contains(">1.2 GB<"), "{pg}");
    assert!(pg.contains("shop-api-db-1"), "{pg}");
    assert!(pg.contains(">shop-api<"), "{pg}");
    // Another engine's volumes are not on this engine's list.
    assert!(!html.contains("crm_pgdata"));
}

#[test]
fn volumes_list_scene_disables_delete_with_the_reason_while_a_container_uses_the_volume() {
    let html = markup("Volumes / list / 1440");
    let used = row_of(&html, "shop-api_pgdata");
    let delete = used
        .split("aria-label=\"Delete shop-api_pgdata\"")
        .nth(1)
        .expect("delete button");
    assert!(
        delete.contains("data-tooltip=\"Used by 1 container\""),
        "{delete}"
    );
    assert!(delete.contains("disabled"), "{delete}");

    let unused = row_of(&html, "stale-cache");
    let delete = unused
        .split("aria-label=\"Delete stale-cache\"")
        .nth(1)
        .expect("delete button");
    let button = delete.split('>').next().expect("attributes");
    assert!(!button.contains("disabled"), "{button}");
}

#[test]
fn volumes_list_columns_carry_the_classes_the_tiers_hide() {
    let html = markup("Volumes / list / 1440");
    for class in [
        "vl-project-head",
        "vl-driver-head",
        "vl-created-head",
        "vl-project",
        "vl-driver",
        "vl-created",
    ] {
        assert!(html.contains(class), "{class}");
    }
    // The generated tiers drop them in that order.
    assert!(VOLUMES_CSS.contains("@container volumes (max-width:"));
    let first_hide = VOLUMES_CSS
        .find("display: none")
        .expect("a tier hides a column");
    let driver = VOLUMES_CSS.find(".vl-driver").expect("driver");
    let created = VOLUMES_CSS.find(".vl-created").expect("created");
    assert!(driver < created, "Driver goes before Created");
    assert!(first_hide > 0);
}

#[test]
fn volumes_list_scene_shows_the_selected_row() {
    let html = markup("Volumes / selected / 1440");
    assert!(
        row_of(&html, "shop-api_pgdata").starts_with("class=\"vl-row vl-selected\""),
        "{}",
        row_of(&html, "shop-api_pgdata")
    );
    assert!(row_of(&html, "stale-cache").starts_with("class=\"vl-row\""));
}

#[test]
fn volumes_empty_copy_says_what_to_do() {
    let (title, _, action) = empty_copy("", UsageFilter::All);
    assert_eq!(title, "No volumes yet");
    assert_eq!(action, Some("Create volume"));
    let (title, body, action) = empty_copy("pg", UsageFilter::All);
    assert_eq!(title, "No volumes match");
    assert!(body.contains("\u{201c}pg\u{201d}"));
    assert_eq!(action, Some("Clear filter"));
    let (title, _, action) = empty_copy("", UsageFilter::Unused);
    assert_eq!(title, "Every volume is in use");
    assert_eq!(action, Some("Show all volumes"));
}

#[test]
fn volumes_failed_actions_become_banners_naming_the_volume() {
    let mut pending = BTreeMap::new();
    pending.insert(
        PendingKey {
            subject: Subject::Volume("a-vol".to_string()),
            action: "volume.delete",
        },
        PendingState::Failed {
            message: "volume is in use".to_string(),
        },
    );
    pending.insert(
        PendingKey {
            subject: Subject::Volume("b-vol".to_string()),
            action: "volume.delete",
        },
        PendingState::Running {
            label: "Deleting\u{2026}".to_string(),
        },
    );
    let banners = failure_banners(&pending);
    assert_eq!(banners.len(), 1);
    assert_eq!(banners[0].1, "Could not delete a-vol: volume is in use");
}

// ---------------------------------------------------------------------
// With signals and a mock engine
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
    engine: Arc<berth_engine::MockEngine>,
    /// The second engine, `colima:work`.
    work: Arc<berth_engine::MockEngine>,
    service: ActionsService,
    finished: tokio::sync::mpsc::UnboundedReceiver<crate::services::actions::Finished>,
}

fn rig() -> Rig {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    let world = berth_engine::MockWorld::bundled();
    let clients = crate::services::engines::client_map();
    let engine = Arc::new(berth_engine::MockEngine::bundled(&default_engine()));
    clients.lock().expect("clients").insert(
        default_engine(),
        Arc::clone(&engine) as Arc<dyn berth_engine::EngineClient>,
    );
    let work_id = EngineId("colima:work".to_string());
    let work = Arc::new(berth_engine::MockEngine::bundled(&work_id));
    clients.lock().expect("clients").insert(
        work_id,
        Arc::clone(&work) as Arc<dyn berth_engine::EngineClient>,
    );
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let (service, finished) = ActionsService::new(
        clients,
        store,
        Arc::new(berth_engine::MockCompose::bundled()),
        Arc::new(berth_engine::MockColima::bundled()),
        tokio_util::sync::CancellationToken::new(),
    );
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
        state.section.set(Section::Volumes);
    });
    Rig {
        dom,
        state,
        engine,
        work,
        service,
        finished,
    }
}

impl Rig {
    fn select(&self, name: &str) {
        self.dom.in_runtime(|| {
            let mut state = self.state;
            state.selection.set(Selection::Volume(name.to_string()));
        });
    }

    fn model(&self, name: &str) -> Option<crate::state::ConfirmModel> {
        self.dom
            .in_runtime(|| confirm::delete_volume_model(&self.state, name))
    }

    fn overlays(&self) -> Vec<Overlay> {
        self.dom.in_runtime(|| self.state.overlays.peek().clone())
    }
}

#[test]
fn volumes_delete_dialog_names_the_volume_asks_for_its_name_and_removes_it() {
    let rig = rig();
    let model = rig
        .model("stale-cache")
        .expect("an unused volume can be deleted");
    assert_eq!(model.title, "Delete volume stale-cache?");
    assert_eq!(model.effect.typed_name.as_deref(), Some("stale-cache"));
    assert_eq!(model.effect.count, 1);
    assert_eq!(model.effect.bytes, Some(bytesize::ByteSize::mb(800)));
    assert_eq!(confirm::button_label(&model.effect), "Delete volume");
    assert_eq!(confirm::result_label(&model.effect, 1), "Deleted 1 volume");
    assert_eq!(
        model.action,
        EngineAction::Remove {
            engine: default_engine(),
            target: ResourceRef::Volume("stale-cache".to_string()),
            force: false,
        }
    );
    assert_eq!(model.targets, [Subject::Volume("stale-cache".to_string())]);
    // The button stays locked until the exact name is typed.
    assert!(!crate::ui::confirm_dialog::name_matches(&model.effect, ""));
    assert!(!crate::ui::confirm_dialog::name_matches(
        &model.effect,
        "stale"
    ));
    assert!(crate::ui::confirm_dialog::name_matches(
        &model.effect,
        "stale-cache"
    ));
}

#[test]
fn volumes_a_volume_in_use_has_no_delete_dialog() {
    let rig = rig();
    assert!(rig.model("shop-api_pgdata").is_none());
    assert!(rig.model("no-such-volume").is_none());
    // Nor does the key open one.
    rig.select("shop-api_pgdata");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        crate::keys::dispatch(&mut state, "volume.delete");
    });
    assert!(rig.overlays().is_empty());
}

#[test]
fn volumes_backspace_on_an_unused_volume_opens_the_dialog_only_on_this_screen() {
    let rig = rig();
    rig.select("tmp-export");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        state.section.set(Section::Containers);
        crate::keys::dispatch(&mut state, "volume.delete");
    });
    assert!(rig.overlays().is_empty(), "not on the Volumes screen");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        state.section.set(Section::Volumes);
        crate::keys::dispatch(&mut state, "volume.delete");
    });
    match rig.overlays().as_slice() {
        [Overlay::Confirm(model)] => assert_eq!(model.title, "Delete volume tmp-export?"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn volumes_with_two_engines_running_an_unused_volume_on_the_second_is_deleted_there() {
    let mut rig = rig();
    let work = EngineId("colima:work".to_string());
    // Both engines run, and the second has an unused volume of its own.
    let made = berth_engine::EngineClient::create_volume(
        rig.work.as_ref(),
        &berth_core::VolumeSpec {
            name: "berthfx-work-scratch".to_string(),
            driver: "local".to_string(),
            driver_opts: std::collections::BTreeMap::new(),
            labels: std::collections::BTreeMap::new(),
        },
    )
    .await
    .expect("created on the second engine");
    assert_eq!(made.engine, work, "the engine stamps the volume");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        for engine in state.engines.write().iter_mut() {
            engine.state = berth_core::EngineState::Running;
        }
        state.volumes.write().push(made.clone());
    });
    let model = rig.model("berthfx-work-scratch").expect("dialog");
    assert!(matches!(
        &model.action,
        EngineAction::Remove { engine, .. } if *engine == work
    ));
    let launched = rig.dom.in_runtime(|| {
        let mut state = rig.state;
        confirm::run(&mut state, &rig.service, &model, false)
    });
    assert_eq!(launched.refused, Vec::<String>::new());
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), rig.finished.recv())
        .await
        .expect("finishes")
        .expect("open");
    assert_eq!(done.result, Ok(()));
    let on_work: Vec<String> = berth_engine::EngineClient::volumes(rig.work.as_ref())
        .await
        .expect("volumes")
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert!(
        !on_work.contains(&"berthfx-work-scratch".to_string()),
        "{on_work:?}"
    );
    let on_default: Vec<String> = berth_engine::EngineClient::volumes(rig.engine.as_ref())
        .await
        .expect("volumes")
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert!(
        on_default.contains(&"stale-cache".to_string()),
        "the first engine kept its volumes"
    );
}

#[tokio::test]
async fn volumes_a_confirmed_delete_reaches_the_engine_and_the_result_repeats_the_dialog() {
    let mut rig = rig();
    let model = rig.model("old-uploads").expect("dialog");
    let launched = rig.dom.in_runtime(|| {
        let mut state = rig.state;
        confirm::run(&mut state, &rig.service, &model, false)
    });
    assert_eq!(launched.refused, Vec::<String>::new());
    assert_eq!(launched.keys.len(), 1);
    let pending = rig.dom.in_runtime(|| rig.state.pending.peek().clone());
    assert!(
        matches!(pending.get(&launched.keys[0]), Some(PendingState::Running { label }) if label == "Deleting\u{2026}"),
        "{pending:?}"
    );
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), rig.finished.recv())
        .await
        .expect("finishes")
        .expect("open");
    assert_eq!(done.result, Ok(()));
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        crate::services::actions::apply_finished(&mut state, done);
    });
    let names: Vec<String> = berth_engine::EngineClient::volumes(rig.engine.as_ref())
        .await
        .expect("volumes")
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert!(!names.contains(&"old-uploads".to_string()));
    assert!(
        names.contains(&"stale-cache".to_string()),
        "only that one went"
    );
    let pending = rig.dom.in_runtime(|| rig.state.pending.peek().clone());
    let done = confirm::finished_count(&pending, &launched.keys).expect("none running");
    assert_eq!(
        confirm::result_label(&model.effect, done),
        "Deleted 1 volume"
    );
}

#[test]
fn volumes_a_volume_dialog_refuses_a_target_that_is_not_a_volume() {
    let rig = rig();
    let mut model = rig.model("old-uploads").expect("dialog");
    model.targets = vec![Subject::Container(berth_core::ContainerId(
        "c0ffee".to_string(),
    ))];
    let launched = rig.dom.in_runtime(|| {
        let mut state = rig.state;
        confirm::run(&mut state, &rig.service, &model, false)
    });
    assert!(launched.keys.is_empty());
    assert_eq!(launched.refused, ["This is not the volume to delete"]);
}

#[tokio::test]
async fn volumes_a_dialog_for_one_volume_cannot_delete_another() {
    let rig = rig();
    let mut model = rig.model("old-uploads").expect("dialog");
    model.targets = vec![Subject::Volume("tmp-export".to_string())];
    let launched = rig.dom.in_runtime(|| {
        let mut state = rig.state;
        confirm::run(&mut state, &rig.service, &model, false)
    });
    assert!(launched.keys.is_empty(), "{:?}", launched.keys);
    assert_eq!(launched.refused, ["This is not the volume to delete"]);
    assert!(rig.dom.in_runtime(|| rig.state.pending.peek().is_empty()));
    let names: Vec<String> = berth_engine::EngineClient::volumes(rig.engine.as_ref())
        .await
        .expect("list")
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert!(names.contains(&"tmp-export".to_string()), "{names:?}");
    assert!(names.contains(&"old-uploads".to_string()), "{names:?}");
}

#[test]
fn volumes_create_button_opens_the_sheet_once() {
    let rig = rig();
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        open_create(&mut state);
        open_create(&mut state);
    });
    assert_eq!(rig.overlays(), [Overlay::CreateVolume]);
}
