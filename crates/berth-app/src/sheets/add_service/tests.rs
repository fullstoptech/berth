//! P5-T9 tests for the Add service sheet: the rows and their warnings, the name and the
//! `ServiceExists` way out, the plan a real folder gets, the masked password, and the scenes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use berth_core::{Container, ContainerState, PlanSource, Project, Template};

use super::model::{
    base_name, blocker, build_plan, eligible_projects, image_template, input_rows, name_error,
    pending_plan, plan_inputs, suggested_values, unique_name, AddError, RowKind,
};
use super::{submit, PASSWORD_MASK};
use crate::scenes::{self, SCENE_PASSWORD};
use crate::services::plans::PlanStore;

fn template(id: &str) -> Template {
    berth_rules::templates::built_in()
        .into_iter()
        .find(|t| t.id == id)
        .unwrap_or_else(|| panic!("{id} template"))
}

fn container(name: &str, host_port: u16, state: ContainerState) -> Container {
    let mut c = berth_engine::MockWorld::bundled()
        .containers
        .into_iter()
        .next()
        .expect("a mock container");
    c.name = name.to_string();
    c.state = state;
    c.ports = vec![berth_core::PortMapping {
        host_ip: None,
        host_port: Some(host_port),
        container_port: 80,
        proto: berth_core::Proto::Tcp,
    }];
    c
}

const COMPOSE: &str = "\
# the shop
services:
  web:
    image: nginx:alpine   # keep this comment
  redis:
    image: redis:7-alpine
";

/// A project whose folder is a temp dir holding `COMPOSE` and the given `.env`.
fn project_in(dir: &tempfile::TempDir, env: Option<&str>) -> Project {
    std::fs::write(dir.path().join("compose.yaml"), COMPOSE).expect("compose");
    if let Some(env) = env {
        std::fs::write(dir.path().join(".env"), env).expect("env");
    }
    let mut p = berth_engine::MockWorld::bundled()
        .projects
        .into_iter()
        .next()
        .expect("a mock project");
    p.folder = Some(dir.path().to_path_buf());
    p.compose_files = vec![PathBuf::from("compose.yaml")];
    p.services.retain(|s| s.name == "web" || s.name == "redis");
    p
}

fn typed(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// An obviously fake password for tests that pass one in.
fn fake_password() -> String {
    "q".repeat(24)
}

fn new_text<'a>(plan: &'a berth_core::GenerationPlan, path: &str) -> &'a str {
    plan.files
        .iter()
        .find(|f| f.path == std::path::Path::new(path))
        .and_then(|f| f.new_text.as_deref())
        .unwrap_or_else(|| panic!("{path} is in the plan"))
}

// ---------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------

#[test]
fn add_service_a_taken_port_moves_to_the_next_free_one() {
    // `suggest` keeps a free port and moves only a taken one: the test uses a taken port.
    assert_eq!(berth_rules::ports::suggest(6379, &[]), 6379);
    assert_eq!(berth_rules::ports::suggest(6379, &[6379]), 6380);
    let redis = template("redis");
    assert_eq!(
        suggested_values(&redis, &[])
            .get("PORT")
            .map(String::as_str),
        Some("6379")
    );
    assert_eq!(
        suggested_values(&redis, &[6379])
            .get("PORT")
            .map(String::as_str),
        Some("6380")
    );
    // The fixture's cache holds 56379: asking for that port moves to 56380.
    assert_eq!(berth_rules::ports::suggest(56379, &[56379]), 56380);
}

#[test]
fn add_service_suggested_ports_of_one_template_never_repeat() {
    let rabbit = template("rabbitmq");
    let s = suggested_values(&rabbit, &[5672, 15672]);
    assert_eq!(s.get("PORT").map(String::as_str), Some("5673"));
    assert_eq!(s.get("MANAGEMENT_PORT").map(String::as_str), Some("15673"));
    assert_eq!(s.get("VERSION").map(String::as_str), Some("4"));
    assert_eq!(s.get("VOLUME").map(String::as_str), Some("rabbitmqdata"));
    assert!(
        !s.contains_key("PASSWORD"),
        "the password is never a suggestion"
    );
}

#[test]
fn add_service_a_port_a_running_container_holds_warns_and_names_it() {
    let pg = template("postgres");
    let containers = [container("pg-scratch", 5432, ContainerState::Running)];
    let taken = crate::services::ports::taken_ports(&containers);

    // The field starts at the free port; nothing to warn about yet.
    let rows = input_rows(&pg, &BTreeMap::new(), &taken, &containers);
    let port = rows.iter().find(|r| r.key == "PORT").expect("port row");
    assert_eq!(port.initial, "5433");
    assert_eq!(port.warning, None);

    // Typing the taken one over it warns, by name, and is not an error.
    let rows = input_rows(&pg, &typed(&[("PORT", "5432")]), &taken, &containers);
    let port = rows.iter().find(|r| r.key == "PORT").expect("port row");
    assert_eq!(port.warning.as_deref(), Some("5432 is used by pg-scratch"));
    assert_eq!(port.error, None);
    assert_eq!(
        port.initial, "5433",
        "the start value never follows the typing"
    );

    // A stopped container holds nothing.
    let stopped = [container("pg-old", 5432, ContainerState::Exited)];
    let rows = input_rows(&pg, &typed(&[("PORT", "5432")]), &[], &stopped);
    assert_eq!(
        rows.iter().find(|r| r.key == "PORT").expect("port").warning,
        None
    );
}

#[test]
fn add_service_text_that_is_not_a_port_or_ports_used_twice_are_errors() {
    let rabbit = template("rabbitmq");
    for bad in ["abc", "0", "70000", "-1", ""] {
        let rows = input_rows(&rabbit, &typed(&[("PORT", bad)]), &[], &[]);
        let row = rows.iter().find(|r| r.key == "PORT").expect("port");
        assert_eq!(
            row.error.as_deref(),
            Some("Enter a port from 1 to 65535"),
            "{bad:?}"
        );
    }
    let rows = input_rows(
        &rabbit,
        &typed(&[("PORT", "6000"), ("MANAGEMENT_PORT", "6000")]),
        &[],
        &[],
    );
    for key in ["PORT", "MANAGEMENT_PORT"] {
        let row = rows.iter().find(|r| r.key == key).expect("row");
        assert_eq!(
            row.error.as_deref(),
            Some("6000 is used twice here"),
            "{key}"
        );
    }
}

#[test]
fn add_service_a_volume_name_compose_would_refuse_is_an_error() {
    let pg = template("postgres");
    let rows = input_rows(&pg, &typed(&[("VOLUME", "../etc")]), &[], &[]);
    let row = rows.iter().find(|r| r.key == "VOLUME").expect("volume");
    assert!(row.error.is_some());
    assert_eq!(row.kind, RowKind::Volume);
    let rows = input_rows(&pg, &typed(&[("VOLUME", "shop_pg-data")]), &[], &[]);
    assert_eq!(
        rows.iter()
            .find(|r| r.key == "VOLUME")
            .expect("volume")
            .error,
        None
    );
}

#[test]
fn add_service_rows_follow_the_templates_inputs_in_order() {
    let rabbit = template("rabbitmq");
    let rows = input_rows(&rabbit, &BTreeMap::new(), &[], &[]);
    let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(
        keys,
        ["PORT", "MANAGEMENT_PORT", "PASSWORD", "VOLUME", "VERSION"]
    );
    let version = rows.iter().find(|r| r.key == "VERSION").expect("version");
    assert_eq!(
        version.kind,
        RowKind::Version {
            choices: vec!["4".to_string(), "3".to_string()]
        }
    );
    assert_eq!(
        rows.iter()
            .find(|r| r.key == "PASSWORD")
            .expect("password")
            .kind,
        RowKind::Password
    );
}

// ---------------------------------------------------------------------
// The service name
// ---------------------------------------------------------------------

#[test]
fn add_service_default_name_is_the_template_id_stepped_past_existing_services() {
    let redis = template("redis");
    assert_eq!(base_name(&redis), "redis");
    assert_eq!(unique_name("redis", &[]), "redis");
    assert_eq!(unique_name("redis", &["redis".to_string()]), "redis-2");
    assert_eq!(
        unique_name("redis", &["redis".to_string(), "redis-2".to_string()]),
        "redis-3"
    );
    // S3 is "S3 (SeaweedFS)": its id is already a name.
    assert_eq!(base_name(&template("s3")), "s3");
    // A custom template with an id that is not a name gets its name made into one.
    let mut custom = template("redis");
    custom.kind = berth_core::TemplateKind::Custom;
    custom.id = "9b1d-uuid/with space".to_string();
    custom.name = "Postgres with pgvector".to_string();
    assert_eq!(base_name(&custom), "postgres-with-pgvector");
}

#[test]
fn add_service_name_error_names_the_service_that_is_in_the_way() {
    let existing = vec!["web".to_string(), "redis".to_string()];
    assert_eq!(name_error("cache", &existing, None), None);
    assert_eq!(
        name_error("redis", &existing, None).as_deref(),
        Some("A service called redis is already in this project. Pick another name.")
    );
    assert!(name_error("", &existing, None).is_some());
    assert!(name_error("my service", &existing, None).is_some());
    assert!(name_error("-lead", &existing, None).is_some());
    // The compose file said so, though the project list did not.
    assert!(name_error("cache", &existing, Some("cache")).is_some());
    assert_eq!(name_error("cache-2", &existing, Some("cache")), None);
}

#[test]
fn add_service_service_exists_is_cleared_by_renaming_the_service() {
    let dir = tempfile::tempdir().expect("dir");
    let project = project_in(&dir, None);
    let redis = template("redis");
    let inputs = plan_inputs(&redis, &BTreeMap::new(), &[], &fake_password());

    // The compose file has `redis` already: the plan says whose name it is.
    let err = build_plan(&project, &redis, "redis", &inputs).expect_err("exists");
    assert_eq!(err, AddError::NameTaken("redis".to_string()));
    // The sheet turns that into the name field's error, and the button waits.
    let existing = vec!["web".to_string()];
    let problem = name_error("redis", &existing, Some("redis"));
    assert!(problem.is_some());
    assert_eq!(blocker(Some(&project), problem.as_deref(), &[]), problem);

    // Another name goes through, next to the old service, and the old one is untouched.
    let plan = build_plan(&project, &redis, "redis-2", &inputs).expect("renamed");
    let compose = new_text(&plan, "compose.yaml");
    assert!(
        compose.contains("  redis-2:\n    image: redis:7.4-alpine"),
        "{compose}"
    );
    assert!(
        compose.contains("image: redis:7-alpine"),
        "the old service stays: {compose}"
    );
    assert!(compose.contains("# keep this comment"), "{compose}");
    // The plan starts only the new service.
    assert_eq!(
        plan.after_write,
        Some(berth_core::ComposeCommand::Up {
            services: vec!["redis-2".to_string()],
            build: false
        })
    );
}

#[test]
fn add_service_a_service_in_the_file_but_not_in_the_list_is_found_when_the_plan_is_built() {
    // The project list has no `web` (it is stopped), the file does.
    let dir = tempfile::tempdir().expect("dir");
    let mut project = project_in(&dir, None);
    project.services.clear();
    let adminer = template("adminer");
    let inputs = plan_inputs(&adminer, &BTreeMap::new(), &[], &fake_password());
    assert_eq!(
        name_error("web", &[], None),
        None,
        "the list does not know it"
    );
    assert_eq!(
        build_plan(&project, &adminer, "web", &inputs).expect_err("exists"),
        AddError::NameTaken("web".to_string())
    );
}

// ---------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------

#[test]
fn add_service_plan_keeps_the_password_in_env_and_out_of_the_compose_file() {
    let dir = tempfile::tempdir().expect("dir");
    let project = project_in(&dir, Some("A=1\n"));
    let pg = template("postgres");
    let password = fake_password();
    let inputs = plan_inputs(
        &pg,
        &typed(&[("PORT", "56450"), ("VERSION", "16")]),
        &[],
        &password,
    );
    let plan = build_plan(&project, &pg, "postgres", &inputs).expect("plan");

    let compose = new_text(&plan, "compose.yaml");
    assert!(compose.contains("\"56450:5432\""), "{compose}");
    assert!(compose.contains("image: postgres:16-alpine"), "{compose}");
    assert!(
        !compose.contains(&password),
        "the password never reaches compose.yaml: {compose}"
    );
    assert!(compose.contains("${POSTGRES_PASSWORD:?"), "{compose}");
    let env = new_text(&plan, ".env");
    assert!(env.starts_with("A=1\n"), "the old lines stay: {env}");
    assert!(
        env.contains(&format!("POSTGRES_PASSWORD={password}")),
        "{env}"
    );
    assert_eq!(plan.source, PlanSource::Template("postgres".to_string()));
    // Building a plan writes nothing.
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".env")).expect("env"),
        "A=1\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("compose.yaml")).expect("compose"),
        COMPOSE
    );
}

#[test]
fn add_service_the_pending_plan_starts_the_new_service_and_rebuilds_from_disk() {
    let dir = tempfile::tempdir().expect("dir");
    let project = project_in(&dir, None);
    let redis = template("redis");
    let inputs = plan_inputs(&redis, &BTreeMap::new(), &[], &fake_password());
    let pending = pending_plan(&project, &redis, "cache", &inputs).expect("pending");
    assert_eq!(pending.title, project.name);
    let run = pending.run.clone().expect("a run for the project");
    assert_eq!(run.project, project.id);

    // The compose file changes before Write: Regenerate sees the change and the guard moves.
    let regenerate = pending.regenerate.clone().expect("regenerate");
    let first_hash = pending.plan.files[0].old_sha256.clone();
    std::fs::write(
        dir.path().join("compose.yaml"),
        format!("{COMPOSE}  extra:\n    image: alpine:3\n"),
    )
    .expect("edit");
    let again = regenerate().expect("regenerated");
    assert_ne!(again.files[0].old_sha256, first_hash);
    assert!(new_text(&again, "compose.yaml").contains("  extra:"));

    // And a service added by hand under the same name turns Regenerate into the rename message.
    std::fs::write(
        dir.path().join("compose.yaml"),
        format!("{COMPOSE}  cache:\n    image: alpine:3\n"),
    )
    .expect("edit");
    assert!(matches!(
        regenerate(),
        Err(berth_rules::RulesError::ServiceExists(name)) if name == "cache"
    ));
}

#[test]
fn add_service_a_project_without_a_compose_file_on_this_mac_says_so() {
    let dir = tempfile::tempdir().expect("dir");
    let mut project = project_in(&dir, None);
    std::fs::remove_file(dir.path().join("compose.yaml")).expect("remove");
    let err =
        build_plan(&project, &template("redis"), "redis", &BTreeMap::new()).expect_err("no file");
    assert_eq!(
        err,
        AddError::Failed("Berth cannot find this project's compose file on this Mac.".to_string())
    );
    project.folder = None;
    assert!(matches!(
        build_plan(&project, &template("redis"), "redis", &BTreeMap::new()),
        Err(AddError::Failed(m)) if m.contains("no folder")
    ));
}

#[tokio::test]
async fn add_service_submit_parks_the_plan_for_review_changes_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("dir");
    let project = project_in(&dir, None);
    let store = PlanStore::default();
    let redis = template("redis");
    let inputs = plan_inputs(&redis, &typed(&[("PORT", "56451")]), &[], &fake_password());

    let taken = submit(
        &store,
        project.clone(),
        redis.clone(),
        "web".into(),
        inputs.clone(),
    )
    .await;
    assert_eq!(taken, Err(AddError::NameTaken("web".to_string())));
    assert!(store.is_empty(), "a refused name parks nothing");

    let id = submit(&store, project.clone(), redis, "cache".into(), inputs)
        .await
        .expect("parked");
    let pending = store.get(&id).expect("the plan is in the store");
    assert!(new_text(&pending.plan, "compose.yaml").contains("\"56451:6379\""));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("compose.yaml")).expect("compose"),
        COMPOSE,
        "only Review changes writes"
    );
}

// ---------------------------------------------------------------------
// Image seeds, projects, the blocker
// ---------------------------------------------------------------------

#[test]
fn add_service_an_image_becomes_a_one_service_template() {
    let t = image_template(" nginx:1.27-alpine ").expect("template");
    assert_eq!(t.image, "nginx:1.27-alpine");
    assert_eq!(base_name(&t), "nginx");
    assert_eq!(
        base_name(&image_template("ghcr.io/acme/web-app:2@sha256:abc").expect("t")),
        "web-app"
    );
    assert!(t.inputs.is_empty());
    for bad in ["", "a: b", "x\ny", "${HOME}", "-lead", "a b", "web\"app"] {
        assert!(image_template(bad).is_err(), "{bad:?}");
    }

    let dir = tempfile::tempdir().expect("dir");
    let project = project_in(&dir, None);
    let plan = build_plan(&project, &t, "nginx", &BTreeMap::new()).expect("plan");
    let compose = new_text(&plan, "compose.yaml");
    assert!(
        compose.contains("  nginx:\n    image: nginx:1.27-alpine"),
        "{compose}"
    );
    assert!(compose.contains("restart: unless-stopped"), "{compose}");
    assert!(
        !plan
            .files
            .iter()
            .any(|f| f.path == std::path::Path::new(".env")),
        "an image has no secrets to add"
    );
}

#[test]
fn add_service_only_projects_with_a_folder_and_a_compose_file_are_offered() {
    let world = berth_engine::MockWorld::bundled();
    let offered = eligible_projects(&world.projects);
    assert!(!offered.is_empty());
    assert!(offered
        .iter()
        .all(|p| p.folder.is_some() && !p.compose_files.is_empty()));
    // docker-manger has a folder and no compose file; it is left out.
    assert!(!offered.iter().any(|p| p.name == "docker-manger"));
    let names: Vec<String> = offered.iter().map(|p| p.name.to_lowercase()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "by name");
}

#[test]
fn add_service_the_blocker_asks_for_a_project_first_then_the_name_then_the_inputs() {
    let dir = tempfile::tempdir().expect("dir");
    let project = project_in(&dir, None);
    let rabbit = template("rabbitmq");
    let bad_port = input_rows(&rabbit, &typed(&[("PORT", "x")]), &[], &[]);
    assert_eq!(
        blocker(None, Some("name"), &bad_port).as_deref(),
        Some("Choose a project first")
    );
    assert_eq!(
        blocker(Some(&project), Some("name problem"), &bad_port).as_deref(),
        Some("name problem")
    );
    assert_eq!(
        blocker(Some(&project), None, &bad_port).as_deref(),
        Some("Enter a port from 1 to 65535")
    );
    let ok = input_rows(&rabbit, &BTreeMap::new(), &[], &[]);
    assert_eq!(blocker(Some(&project), None, &ok), None);
}

// ---------------------------------------------------------------------
// The sheet
// ---------------------------------------------------------------------

fn scene(frame: &str) -> String {
    scenes::render_scene_html(frame).unwrap_or_else(|| panic!("{frame} renders"))
}

fn button_tag<'a>(html: &'a str, label: &str) -> &'a str {
    let at = html
        .find(&format!(">{label}<"))
        .unwrap_or_else(|| panic!("{label} button in {html}"));
    &html[html[..at].rfind("<button").expect("tag")..at]
}

#[test]
fn add_service_sheet_masks_the_generated_password_everywhere() {
    for frame in [
        "Add service / postgres / 1440",
        "Add service / rabbitmq / 1440",
        "Add service / port taken / 1440",
    ] {
        let html = scene(frame);
        assert!(html.contains("Generated password, hidden"), "{frame}");
        assert!(html.contains(PASSWORD_MASK), "{frame}");
        assert!(
            !html.contains(SCENE_PASSWORD),
            "{frame}: the sheet drew the password"
        );
        assert!(
            html.contains("It goes in .env, not in compose.yaml."),
            "{frame}"
        );
    }
    // Redis has no password input, so no row.
    assert!(!scene("Add service / pick project / 1440").contains("Generated password"));
}

#[test]
fn add_service_sheet_without_a_project_asks_for_one_and_waits() {
    let html = scene("Add service / pick project / 1440");
    assert!(html.contains("aria-label=\"Add service\""), "{html}");
    assert!(html.contains("aria-label=\"Project\""), "{html}");
    assert!(html.contains(">Choose a project<"), "{html}");
    for name in ["shop-api", "analytics", "berth-site"] {
        assert!(
            html.contains(&format!(">{name}</option>")),
            "{name}: {html}"
        );
    }
    assert!(
        !html.contains(">docker-manger</option>"),
        "a project with no compose file is not offered: {html}"
    );
    let tag = button_tag(&html, "Review changes");
    assert!(tag.contains("disabled"), "{tag}");
    assert!(tag.contains("Choose a project first"), "{tag}");
}

#[test]
fn add_service_sheet_with_a_project_suggests_a_free_port_and_enables_review() {
    let html = scene("Add service / postgres / 1440");
    assert!(
        html.contains(">shop-api<"),
        "the seeded project is named: {html}"
    );
    assert!(
        !html.contains("aria-label=\"Project\""),
        "no picker: {html}"
    );
    // pg-scratch holds 5432, so the field starts at 5433 and says nothing.
    assert!(html.contains("value=\"5433\""), "{html}");
    assert!(!html.contains("is used by"), "{html}");
    for label in ["Host port", "Version", "Data volume", "Service name"] {
        assert!(
            html.contains(&format!("aria-label=\"{label}\"")),
            "{label}: {html}"
        );
    }
    assert!(html.contains("value=\"postgres\""), "{html}");
    let tag = button_tag(&html, "Review changes");
    assert!(!tag.contains("disabled"), "{tag}");
}

#[test]
fn add_service_sheet_names_the_container_that_holds_a_typed_port() {
    let html = scene("Add service / port taken / 1440");
    assert!(html.contains("5432 is used by pg-scratch"), "{html}");
    // A warning, not an error: Review changes stays available.
    assert!(!button_tag(&html, "Review changes").contains("disabled"));
}

#[test]
fn add_service_sheet_blocks_a_service_name_the_project_already_has() {
    let html = scene("Add service / name exists / 1440");
    assert!(
        html.contains("A service called redis is already in this project. Pick another name."),
        "{html}"
    );
    let tag = button_tag(&html, "Review changes");
    assert!(tag.contains("disabled"), "{tag}");
    // Without a typed name the sheet starts from a free one.
    let html = scene("Add service / postgres / 1440");
    assert!(!html.contains("already in this project"));
    assert!(base_name(&template("redis")) == "redis");
}

#[test]
fn add_service_sheet_from_an_image_shows_the_image_and_no_template_picker() {
    let html = scene("Add service / from image / 1440");
    assert!(html.contains("nginx:1.27-alpine"), "{html}");
    assert!(!html.contains("aria-label=\"Template\""), "{html}");
    assert!(html.contains("value=\"nginx\""), "{html}");
    assert!(html.contains("aria-label=\"Project\""), "{html}");
}

#[test]
fn add_service_sheet_with_a_template_offers_the_others() {
    let html = scene("Add service / pick project / 1440");
    assert!(html.contains("aria-label=\"Template\""), "{html}");
    assert!(html.contains(">Redis \u{b7} Cache</option>"), "{html}");
    assert!(html.contains("Redis 7 without a password"), "{html}");
}

// ---------------------------------------------------------------------
// From Find images
// ---------------------------------------------------------------------

#[test]
fn add_service_find_images_add_to_project_is_enabled_and_opens_the_sheet_on_the_image() {
    use dioxus::prelude::*;

    let html = scene("Images / find / 1440");
    let tag = button_tag(&html, "Add to project\u{2026}");
    assert!(!tag.contains("disabled"), "{tag}");
    assert!(!html.contains("Arrives with Templates"), "{html}");

    thread_local! {
        static SLOT: std::cell::RefCell<Option<crate::state::AppState>> = const { std::cell::RefCell::new(None) };
    }
    #[component]
    fn Harness() -> Element {
        let state = crate::state::use_app_state();
        use_hook(move || SLOT.with(|s| *s.borrow_mut() = Some(state)));
        rsx! { div {} }
    }
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let mut state = SLOT.with(|s| s.borrow_mut().take()).expect("state");
    dom.in_runtime(|| {
        crate::screens::find_images::add_to_project(&mut state, "postgres:17-alpine");
    });
    let overlays = dom.in_runtime(|| state.overlays.peek().clone());
    assert_eq!(
        overlays,
        [crate::state::Overlay::AddService(
            crate::state::AddServiceSeed {
                project: None,
                template: None,
                image: Some("postgres:17-alpine".to_string()),
            }
        )]
    );
}
