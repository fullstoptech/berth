//! P8-T4 tests for custom templates: saving a project's service as a template (from a compose file on
//! disk), listing it, adding it to another project and getting the same block back, secrets staying
//! out of the store, and the one summary behind Delete.

use std::path::PathBuf;

use dioxus::prelude::*;

use berth_core::{Project, ProjectId, TemplateKind};

use super::*;
use crate::sheets::add_service::model::build_plan;

const COMPOSE: &str = r#"services:
  cache:
    image: redis:7-alpine
    labels: [dev.berth.fixture=1]
    ports:
      - "127.0.0.1:56379:6379"
  db:
    image: postgres:17-alpine
    environment:
      POSTGRES_PASSWORD: hunter2-literal
    volumes:
      - pgdata:/var/lib/postgresql/data
  api:
    build: ./api
volumes:
  pgdata:
"#;

/// A project whose folder holds `compose`, in a temp dir that lives as long as the returned guard.
fn project_with(compose: &str, name: &str) -> (Project, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("compose.yaml"), compose).expect("write compose");
    let project = Project {
        id: ProjectId(name.to_string()),
        name: name.to_string(),
        folder: Some(dir.path().to_path_buf()),
        compose_files: vec![PathBuf::from("compose.yaml")],
        engine: berth_core::EngineId("colima:default".to_string()),
        source: berth_core::ProjectSource::Compose,
        services: Vec::new(),
        git: None,
        status: berth_core::ProjectStatus::Stopped,
        pinned: false,
    };
    (project, dir)
}

fn store() -> berth_store::Store {
    berth_store::Store::open_in_memory().expect("store")
}

fn service_names(text: &str) -> Vec<String> {
    berth_rules::compose_edit::service_names(text).expect("services")
}

#[test]
fn custom_templates_save_lists_the_service_under_custom_and_saving_again_replaces_it() {
    let (project, _dir) = project_with(COMPOSE, "berthfx");
    let store = store();
    let saved = save_from_service(&store, &project, "cache").expect("saved");
    assert_eq!(
        saved,
        Saved {
            name: "cache".to_string(),
            replaced: false
        }
    );
    assert_eq!(saved.message(), "Saved cache as a template.");

    let listed = all(Some(&store));
    let built_ins = berth_rules::templates::built_in().len();
    assert_eq!(listed.len(), built_ins + 1);
    let custom = listed.last().expect("custom last");
    assert_eq!(custom.kind, TemplateKind::Custom);
    assert_eq!(custom.id, "custom-berthfx-cache");
    assert_eq!(custom.image, "redis:7-alpine");
    assert!(
        custom.description.contains("berthfx"),
        "{}",
        custom.description
    );

    // The same service again is an update, not a second entry.
    let again = save_from_service(&store, &project, "cache").expect("saved again");
    assert!(again.replaced);
    assert_eq!(again.message(), "Updated the cache template.");
    assert_eq!(all(Some(&store)).len(), built_ins + 1);
}

#[test]
fn custom_templates_add_service_with_it_writes_the_block_that_was_saved() {
    let (source, _a) = project_with(COMPOSE, "berthfx");
    let store = store();
    save_from_service(&store, &source, "cache").expect("saved");
    let template = all(Some(&store)).pop().expect("the custom template");

    let (target, _b) = project_with("services:\n  web:\n    image: nginx\n", "other");
    let plan = build_plan(
        &target,
        &template,
        "cache",
        &std::collections::BTreeMap::new(),
    )
    .expect("plan");
    let written = plan.files[0].new_text.clone().expect("new compose text");
    assert_eq!(service_names(&written), ["web", "cache"], "{written}");
    // Same image, same labels, same port mapping as the saved service.
    for line in [
        "image: redis:7-alpine",
        "dev.berth.fixture=1",
        "127.0.0.1:56379:6379",
    ] {
        assert!(written.contains(line), "{line} missing: {written}");
    }
    assert!(
        matches!(plan.source, berth_core::PlanSource::Template(ref id) if id == "custom-berthfx-cache"),
        "{:?}",
        plan.source
    );
}

#[test]
fn custom_templates_secrets_stay_out_of_the_store() {
    let (project, _dir) = project_with(COMPOSE, "berthfx");
    let store = store();
    save_from_service(&store, &project, "db").expect("saved");
    let stored = format!("{:?}", store.custom_templates().expect("templates"));
    assert!(!stored.contains("hunter2-literal"), "{stored}");
    assert!(
        stored.contains("POSTGRES_PASSWORD:?set in .env"),
        "{stored}"
    );
}

#[test]
fn custom_templates_a_service_built_from_a_folder_is_refused_with_the_reason() {
    let (project, _dir) = project_with(COMPOSE, "berthfx");
    let store = store();
    let err = save_from_service(&store, &project, "api").expect_err("build");
    assert!(err.contains("built from a folder"), "{err}");
    assert!(store.custom_templates().expect("templates").is_empty());
}

#[test]
fn custom_templates_a_missing_file_or_service_says_so() {
    let (mut project, dir) = project_with(COMPOSE, "berthfx");
    let store = store();
    let err = save_from_service(&store, &project, "ghost").expect_err("no service");
    assert!(err.contains("not in this project's compose file"), "{err}");
    drop(dir);
    let err = save_from_service(&store, &project, "cache").expect_err("no file");
    assert!(
        err.contains("cannot find this project's compose file"),
        "{err}"
    );
    project.folder = None;
    let err = save_from_service(&store, &project, "cache").expect_err("no folder");
    assert!(err.contains("no folder"), "{err}");
}

#[test]
fn custom_templates_the_id_follows_the_project_and_the_service() {
    let a = custom_id(&ProjectId("Shop API".to_string()), "db");
    assert_eq!(a, "custom-shop-api-db");
    assert_ne!(a, custom_id(&ProjectId("Shop API".to_string()), "cache"));
    assert_ne!(a, custom_id(&ProjectId("other".to_string()), "db"));
}

#[test]
fn custom_templates_delete_is_one_summary_and_removes_only_that_template() {
    let (project, _dir) = project_with(COMPOSE, "berthfx");
    let store = store();
    save_from_service(&store, &project, "cache").expect("saved");
    save_from_service(&store, &project, "db").expect("saved");

    let effect = delete_effect("cache");
    assert_eq!(
        crate::services::confirm::button_label(&effect),
        "Delete template"
    );
    assert_eq!(
        crate::services::confirm::result_label(&effect, 1),
        "Deleted 1 template"
    );
    assert!(effect.lines.iter().any(|l| l.kind == EffectKind::Goes));
    assert!(effect.lines.iter().any(|l| l.kind == EffectKind::Stays));

    delete(&store, "custom-berthfx-cache").expect("deleted");
    let left: Vec<String> = store
        .custom_templates()
        .expect("templates")
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(left, ["custom-berthfx-db"]);
}

// ---------------------------------------------------------------------
// The UI path: a toast says how it went
// ---------------------------------------------------------------------

thread_local! {
    static SLOT: std::cell::RefCell<Option<crate::state::AppState>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = crate::state::use_app_state();
    use_hook(move || SLOT.with(|s| *s.borrow_mut() = Some(state)));
    rsx! { div {} }
}

async fn toast_after(
    project: Project,
    service: &str,
    store: std::sync::Arc<berth_store::Store>,
) -> crate::state::Toast {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let mut state = SLOT.with(|s| s.borrow_mut().take()).expect("state");
    let services = crate::services::Services {
        store,
        clients: crate::services::engines::client_map(),
        cancel: tokio_util::sync::CancellationToken::new(),
    };
    let service = service.to_string();
    dom.in_scope(ScopeId::APP, || {
        save(&mut state, &services, project, service)
    });
    let waited = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(t) = dom.in_runtime(|| state.toasts.peek().first().cloned()) {
                return t;
            }
            let _ = tokio::time::timeout(std::time::Duration::from_millis(50), dom.wait_for_work())
                .await;
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
        }
    })
    .await;
    waited.expect("a toast")
}

#[tokio::test]
async fn custom_templates_save_from_the_menu_says_what_happened_in_a_toast() {
    let (project, _dir) = project_with(COMPOSE, "berthfx");
    let store = std::sync::Arc::new(store());
    let toast = toast_after(project.clone(), "cache", store.clone()).await;
    assert_eq!(toast.text, "Saved cache as a template.");
    assert_eq!(toast.tone, crate::state::ToastTone::Success);
    assert_eq!(store.custom_templates().expect("templates").len(), 1);

    let toast = toast_after(project, "api", store.clone()).await;
    assert_eq!(toast.tone, crate::state::ToastTone::Danger);
    assert!(toast.text.contains("built from a folder"), "{}", toast.text);
    assert_eq!(store.custom_templates().expect("templates").len(), 1);
}

/// The done-when's hand check on the real fixture: saving `cache` of `fixtures/compose/berthfx` lists
/// it under Custom, and Add service with it writes the same block into another project.
#[test]
fn custom_templates_the_fixture_cache_service_comes_back_as_the_same_block() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/compose/berthfx/compose.yaml"
    ))
    .expect("fixture compose");
    let (source, _a) = project_with(&fixture, "berthfx");
    let store = store();
    save_from_service(&store, &source, "cache").expect("saved");
    let listed = all(Some(&store));
    let template = listed.last().expect("custom last");
    assert_eq!(template.kind, TemplateKind::Custom);

    let (target, _b) = project_with("services:\n  web:\n    image: nginx\n", "other");
    let plan = build_plan(
        &target,
        template,
        "cache",
        &std::collections::BTreeMap::new(),
    )
    .expect("plan");
    let written = plan.files[0].new_text.clone().expect("text");
    let block = |text: &str| {
        let doc: serde_json::Value =
            serde_json::to_value(berth_rules::compose_edit::service_names(text).expect("names"))
                .expect("json");
        doc
    };
    assert_eq!(block(&written), serde_json::json!(["web", "cache"]));
    for line in [
        "image: redis:7-alpine",
        "dev.berth.fixture=1",
        "127.0.0.1:56379:6379",
    ] {
        assert!(written.contains(line), "{line}: {written}");
    }
}
