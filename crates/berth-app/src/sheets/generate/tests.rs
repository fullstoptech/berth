//! P5-T3 tests for the Generate sheet: what the choices turn into, the port rows with their
//! warnings, the plan a real fixture folder gets, and the sheet's states.

use std::path::{Path, PathBuf};

use berth_core::{Container, ContainerState, Detection, ServiceKind};

use super::model::{
    build_plan, has_compose, options, port_rows, replan, stack_line, with_choices, Choices,
};
use super::{blocker, GENERATE_CSS};
use crate::scenes::{self, next_detection};

fn detection() -> Detection {
    next_detection()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/folders")
        .join(name)
}

/// A copy of a fixture folder that tests may write into.
fn copy_of(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("dir");
    for entry in std::fs::read_dir(fixture(name)).expect("fixture").flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), dir.path().join(entry.file_name())).expect("copy");
        }
    }
    dir
}

fn container(name: &str, host_port: u16) -> Container {
    let mut c = berth_engine::MockWorld::bundled()
        .containers
        .into_iter()
        .next()
        .expect("a mock container");
    c.name = name.to_string();
    c.state = ContainerState::Running;
    c.ports = vec![berth_core::PortMapping {
        host_ip: None,
        host_port: Some(host_port),
        container_port: 80,
        proto: berth_core::Proto::Tcp,
    }];
    c
}

// ---------------------------------------------------------------------
// Choices and options
// ---------------------------------------------------------------------

#[test]
fn generate_choices_start_from_what_was_found() {
    let c = Choices::from_detection(&detection(), &[]);
    assert_eq!(c.database, Some(ServiceKind::Postgres));
    assert_eq!(c.extras, [ServiceKind::Redis]);
    assert_eq!(c.node_version.as_deref(), Some("22"));
    assert_eq!(c.services(), [ServiceKind::Postgres, ServiceKind::Redis]);
}

#[test]
fn generate_extras_keep_the_sheets_order_when_ticked_in_any_order() {
    let mut c = Choices::default();
    c.set_extra(ServiceKind::Mailpit, true);
    c.set_extra(ServiceKind::Redis, true);
    assert_eq!(c.extras, [ServiceKind::Redis, ServiceKind::Mailpit]);
    c.set_extra(ServiceKind::Redis, false);
    assert_eq!(c.extras, [ServiceKind::Mailpit]);
}

#[test]
fn generate_choosing_another_database_changes_the_services_the_options_offer() {
    let d = detection();
    let mut c = Choices::from_detection(&d, &[]);
    c.database = Some(ServiceKind::Mysql);
    let services = with_choices(&d, &c).services_needed;
    assert!(services.iter().any(|s| s.kind == ServiceKind::Mysql));
    assert!(!services.iter().any(|s| s.kind == ServiceKind::Postgres));
    // A service the detection found keeps its reason; a new one says it was chosen here.
    let redis = services
        .iter()
        .find(|s| s.kind == ServiceKind::Redis)
        .expect("redis");
    assert_eq!(redis.reason, "from REDIS_URL");
    assert_eq!(
        services
            .iter()
            .find(|s| s.kind == ServiceKind::Mysql)
            .expect("mysql")
            .reason,
        "chosen here"
    );
    let opts = options(&d, &c, &[]).expect("options");
    assert!(opts.services.contains(&ServiceKind::Mysql));
}

#[test]
fn generate_port_rows_offer_a_free_port_and_say_who_holds_the_taken_ones() {
    let d = detection();
    let c = Choices::from_detection(&d, &[]);
    let rows = port_rows(&d, &c, &[3000, 5432], &[container("pg-scratch", 5432)]);
    let by_key = |key: &str| {
        rows.iter()
            .find(|r| r.key == key)
            .unwrap_or_else(|| panic!("{key}"))
    };
    assert_eq!(rows[0].key, "app", "the app comes first");
    assert_eq!(by_key("app").value, "3001");
    assert_eq!(by_key("db").value, "5433");
    assert_eq!(by_key("db").label, "PostgreSQL port");
    assert!(rows
        .iter()
        .all(|r| r.error.is_none() && r.warning.is_none()));

    // Typing a port a container holds is allowed but warned about.
    let mut typed = c.clone();
    typed.ports.insert("db".to_string(), "5432".to_string());
    let rows = port_rows(&d, &typed, &[3000, 5432], &[container("pg-scratch", 5432)]);
    let db = rows.iter().find(|r| r.key == "db").expect("db");
    // The field keeps starting from the suggestion: what is typed is never fed back into it.
    assert_eq!(db.value, "5433");
    assert_eq!(db.warning.as_deref(), Some("5432 is used by pg-scratch"));
    assert_eq!(db.error, None);
}

#[test]
fn generate_port_rows_flag_text_that_is_not_a_port_and_ports_used_twice() {
    let d = detection();
    let mut c = Choices::from_detection(&d, &[]);
    c.ports.insert("app".to_string(), "abc".to_string());
    c.ports.insert("db".to_string(), "6379".to_string());
    c.ports.insert("redis".to_string(), "6379".to_string());
    let rows = port_rows(&d, &c, &[], &[]);
    let row = |key: &str| rows.iter().find(|r| r.key == key).expect("row");
    assert_eq!(
        row("app").error.as_deref(),
        Some("Enter a port from 1 to 65535")
    );
    assert_eq!(row("db").error.as_deref(), Some("6379 is used twice here"));
    assert_eq!(
        row("redis").error.as_deref(),
        Some("6379 is used twice here")
    );
    for bad in ["0", "70000", "-1", ""] {
        let mut c = c.clone();
        c.ports.insert("app".to_string(), bad.to_string());
        let rows = port_rows(&d, &c, &[], &[]);
        assert!(rows[0].error.is_some(), "{bad:?} is not a port");
    }
}

#[test]
fn generate_options_apply_typed_ports_and_refuse_a_bad_node_version() {
    let d = detection();
    let mut c = Choices::from_detection(&d, &[]);
    c.ports.insert("app".to_string(), "4000".to_string());
    c.node_version = Some(" 20 ".to_string());
    let opts = options(&d, &c, &[]).expect("options");
    assert_eq!(opts.ports.get("app"), Some(&4000));
    assert_eq!(opts.node_version.as_deref(), Some("20"));

    c.node_version = Some("lts".to_string());
    assert_eq!(
        options(&d, &c, &[]).expect_err("version"),
        "lts is not a Node version"
    );
    c.node_version = Some("22".to_string());
    c.ports.insert("app".to_string(), "x".to_string());
    assert_eq!(
        options(&d, &c, &[]).expect_err("port"),
        "x is not a port number"
    );
    c.ports.insert("app".to_string(), "5433".to_string());
    c.ports.insert("db".to_string(), "5433".to_string());
    assert_eq!(
        options(&d, &c, &[]).expect_err("twice"),
        "Port 5433 is used twice"
    );
}

// ---------------------------------------------------------------------
// What was found
// ---------------------------------------------------------------------

#[test]
fn generate_stack_line_reads_like_the_frame() {
    assert_eq!(
        stack_line(&detection()),
        "Next.js 15 \u{b7} pnpm \u{b7} Node 22"
    );
    let mut d = detection();
    d.stack.clear();
    assert_eq!(stack_line(&d), "A Dockerfile");
    d.found.clear();
    assert_eq!(stack_line(&d), "No known stack");
}

#[test]
fn generate_blocker_names_a_folder_that_has_compose_or_is_not_recognised() {
    assert_eq!(blocker(&detection()), None);
    let mut d = detection();
    d.found.push(berth_core::FoundFile {
        kind: berth_core::FoundKind::Compose,
        path: PathBuf::from("compose.yaml"),
    });
    assert!(has_compose(&d));
    assert!(blocker(&d)
        .expect("blocked")
        .contains("already has a compose file"));
    let mut d = detection();
    d.stack.clear();
    d.found.clear();
    assert!(blocker(&d).expect("blocked").contains("does not recognise"));
}

// ---------------------------------------------------------------------
// Plans from real folders
// ---------------------------------------------------------------------

#[test]
fn generate_plan_for_a_folder_with_a_dockerfile_adds_compose_and_env_keys_only() {
    let dir = copy_of("node-next-pnpm");
    let d = berth_rules::detect::detect(dir.path()).expect("detect");
    let choices = Choices::from_detection(&d, &[]);
    let opts = options(&d, &choices, &[]).expect("options");
    let plan = build_plan(&d, &opts).expect("plan");
    let paths: Vec<_> = plan
        .files
        .iter()
        .map(|f| f.path.display().to_string())
        .collect();
    assert!(paths.contains(&"compose.yaml".to_string()), "{paths:?}");
    assert!(
        !paths.contains(&"Dockerfile".to_string()),
        "the folder's Dockerfile is reused: {paths:?}"
    );
    assert_eq!(plan.root, dir.path());
    assert!(plan.after_write.is_some());
    // Nothing was written to the folder by building the plan.
    assert!(!dir.path().join("compose.yaml").exists());
}

#[test]
fn generate_replan_reads_the_folder_again_and_sees_what_changed() {
    let dir = copy_of("node-next-pnpm");
    let d = berth_rules::detect::detect(dir.path()).expect("detect");
    let choices = Choices::from_detection(&d, &[]);
    let first = replan(dir.path(), &choices, &[]).expect("first");
    let env_before = first
        .files
        .iter()
        .find(|f| f.path == Path::new(".env"))
        .and_then(|f| f.old_sha256.clone());
    std::fs::write(
        dir.path().join(".env"),
        "# edited by hand\nDATABASE_URL=postgres://x\n",
    )
    .expect("edit");
    let second = replan(dir.path(), &choices, &[]).expect("second");
    let env_after = second
        .files
        .iter()
        .find(|f| f.path == Path::new(".env"))
        .and_then(|f| f.old_sha256.clone());
    assert!(env_before.is_some() && env_after.is_some());
    assert_ne!(
        env_before, env_after,
        "the plan guards the .env it read last"
    );
}

#[test]
fn generate_plan_for_a_folder_that_already_has_compose_has_nothing_to_write() {
    let dir = copy_of("compose-present");
    let d = berth_rules::detect::detect(dir.path()).expect("detect");
    assert!(blocker(&d).is_some());
    let opts = options(&d, &Choices::from_detection(&d, &[]), &[]).expect("options");
    let plan = build_plan(&d, &opts).expect("plan");
    assert!(plan.files.is_empty());
    assert!(plan
        .notes
        .iter()
        .any(|n| n.contains("already has a compose file")));
}

// ---------------------------------------------------------------------
// The sheet
// ---------------------------------------------------------------------

fn scene(frame: &str) -> String {
    scenes::render_scene_html(frame).unwrap_or_else(|| panic!("{frame} renders"))
}

#[test]
fn generate_sheet_shows_what_was_found_and_the_options() {
    let html = scene("Generate / sheet / 1440");
    assert!(GENERATE_CSS.contains(".gen-sheet"));
    assert!(html.contains("Generate files for new-app"), "{html}");
    assert!(
        html.contains("Next.js 15 \u{b7} pnpm \u{b7} Node 22"),
        "{html}"
    );
    assert!(
        html.contains("PostgreSQL (from DATABASE_URL), Redis (from REDIS_URL)"),
        "{html}"
    );
    for label in [
        "Database",
        "App port",
        "PostgreSQL port",
        "Redis port",
        "Node version",
    ] {
        assert!(
            html.contains(&format!("aria-label=\"{label}\"")),
            "{label}: {html}"
        );
    }
    assert!(
        html.contains(">Review changes<") && html.contains(">Cancel<"),
        "{html}"
    );
    // The harness choice only appears when a harness is detected, and none is in this scene.
    assert!(!html.contains("Generate with"), "{html}");
}

#[test]
fn generate_sheet_for_a_folder_with_compose_says_so_and_disables_review() {
    let html = scene("Generate / compose exists / 1440");
    assert!(html.contains("already has a compose file"), "{html}");
    let at = html.find(">Review changes<").expect("button");
    let tag = &html[html[..at].rfind("<button").expect("tag")..at];
    assert!(tag.contains("disabled"), "{tag}");
}

// ---------------------------------------------------------------------
// The Detected sheet's frame layout
// ---------------------------------------------------------------------

use super::model::{harness_note, reuses_dockerfile, service_rows, set_service, template_name};

#[test]
fn generate_service_rows_name_the_version_the_port_and_the_password() {
    let d = detection();
    let c = Choices::from_detection(&d, &[]);
    let rows = service_rows(&d, &c, &[5432], &[container("pg-scratch", 5432)]);

    assert_eq!(rows.len(), 2);
    let db = &rows[0];
    assert_eq!(db.kind, ServiceKind::Postgres);
    assert_eq!(db.name, "Postgres 17");
    assert!(db.checked);
    assert_eq!(db.port.as_ref().map(|p| p.value.as_str()), Some("5433"));
    assert_eq!(
        db.note,
        "(5432 is used by pg-scratch) \u{b7} password generated into .env"
    );
    assert_eq!(
        db.hint.as_deref(),
        Some("Uncheck if DATABASE_URL points at a hosted database")
    );
    let redis = &rows[1];
    assert_eq!(redis.name, "Redis 7.4");
    // Nothing holds 6379, and Redis has no password to generate.
    assert_eq!(redis.note, "");
}

#[test]
fn generate_unticking_a_service_hides_its_port_and_leaves_it_out_of_the_plan() {
    let d = detection();
    let mut c = Choices::from_detection(&d, &[]);
    set_service(&mut c, ServiceKind::Postgres, false);

    let rows = service_rows(&d, &c, &[], &[]);
    assert!(!rows[0].checked, "the row stays so it can be ticked again");
    assert!(rows[0].port.is_none() && rows[0].note.is_empty());
    assert!(rows[1].checked);
    let opts = options(&d, &c, &[]).expect("options");
    assert_eq!(opts.services, [ServiceKind::Redis]);
    assert!(!opts.ports.contains_key("db"));

    set_service(&mut c, ServiceKind::Postgres, true);
    assert_eq!(c.database, Some(ServiceKind::Postgres));
}

#[test]
fn generate_ticking_a_database_replaces_the_other_one() {
    let mut d = detection();
    d.services_needed.push(berth_core::ServiceGuess {
        kind: ServiceKind::Mysql,
        reason: "from mysql2 in package.json".to_string(),
    });
    let mut c = Choices::from_detection(&d, &[]);
    set_service(&mut c, ServiceKind::Mysql, true);

    let rows = service_rows(&d, &c, &[], &[]);
    let ticked: Vec<ServiceKind> = rows.iter().filter(|r| r.checked).map(|r| r.kind).collect();
    assert_eq!(ticked, [ServiceKind::Redis, ServiceKind::Mysql]);
    assert!(!rows[0].checked, "Postgres gave way to MySQL");
    // A service found from a dependency has no key to name.
    assert_eq!(rows[2].hint, None);
}

#[test]
fn generate_the_dockerfile_line_and_the_template_card_follow_the_folder() {
    let mut d = detection();
    assert!(reuses_dockerfile(&d));
    assert_eq!(template_name(&d), "Berth's Node template");
    assert_eq!(
        harness_note(&d),
        "Sends file names, package.json and the Dockerfile. Never .env values."
    );

    d.found
        .retain(|f| !matches!(f.kind, berth_core::FoundKind::Dockerfile { .. }));
    d.stack = vec![berth_core::StackGuess::Go];
    assert!(!reuses_dockerfile(&d));
    assert_eq!(template_name(&d), "Berth's Go template");
    assert_eq!(
        harness_note(&d),
        "Sends file names and the project manifest. Never .env values."
    );
}

#[test]
fn generate_the_dockerfile_line_is_true_exactly_when_the_plan_reuses_the_dockerfile() {
    // `build_plan` writes no Dockerfile for a folder that has one, and a Dockerfile of its own
    // for one that does not; the sheet's line must say the same.
    let with = copy_of("node-next-pnpm");
    let d = berth_rules::detect::detect(with.path()).expect("detect");
    let opts = options(&d, &Choices::from_detection(&d, &[]), &[]).expect("options");
    let plan = build_plan(&d, &opts).expect("plan");
    let writes_dockerfile = plan
        .files
        .iter()
        .any(|f| f.path == Path::new("Dockerfile") && f.old_sha256.is_none());
    assert_eq!(reuses_dockerfile(&d), !writes_dockerfile);

    std::fs::write(
        with.path().join("Dockerfile"),
        "FROM node:22-alpine\nEXPOSE 3000\n",
    )
    .expect("dockerfile");
    let d = berth_rules::detect::detect(with.path()).expect("detect");
    let opts = options(&d, &Choices::from_detection(&d, &[]), &[]).expect("options");
    let plan = build_plan(&d, &opts).expect("plan");
    assert!(reuses_dockerfile(&d));
    assert!(!plan.files.iter().any(|f| f.path == Path::new("Dockerfile")));
}
