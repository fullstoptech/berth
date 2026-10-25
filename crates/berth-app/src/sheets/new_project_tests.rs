//! Tests of the New project sheet (P5-T5).

use super::*;
use crate::scenes::render_scene_html;

#[test]
fn new_project_sheet_offers_three_ways_and_disables_git() {
    let html = render_scene_html("New project / sheet / 1440").expect("scene renders");
    for text in [
        "aria-label=\"New project\"",
        ">Open folder<",
        ">From editor<",
        ">From Git<",
        "Cloning from Git arrives with the Git support (P8-T5).",
    ] {
        assert!(html.contains(text), "missing {text:?}\n{html}");
    }
    // Exactly the Git choice is disabled.
    assert_eq!(
        html.matches("np-choice\" type=\"button\" disabled").count(),
        1,
        "{html}"
    );
}

#[test]
fn new_project_editor_page_hosts_the_bundle_and_waits_for_a_folder() {
    let html = render_scene_html("New project / editor / 1440").expect("scene renders");
    assert!(html.contains("id=\"berth-editor-new-project\""), "{html}");
    assert!(html.contains("Choose folder"), "{html}");
    assert!(html.contains("No folder chosen"), "{html}");
    assert!(html.contains("Choose the folder to save it in."), "{html}");
    assert!(
        html.contains("Nothing is written until you review the changes."),
        "{html}"
    );
}

#[test]
fn new_project_plan_creates_compose_yaml_and_starts_the_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plan = compose_plan(dir.path(), "services:\n  web:\n    image: nginx\n").expect("plan");
    assert_eq!(plan.root, dir.path());
    assert_eq!(plan.source, PlanSource::Editor);
    assert_eq!(plan.files.len(), 1);
    assert_eq!(plan.files[0].path, Path::new("compose.yaml"));
    assert_eq!(plan.files[0].change, berth_core::FileChange::Create);
    assert_eq!(
        plan.after_write,
        Some(ComposeCommand::Up {
            services: Vec::new(),
            build: false
        })
    );
    // Planning writes nothing.
    assert!(!dir.path().join("compose.yaml").exists());
}

#[test]
fn new_project_plan_writes_exactly_the_typed_text_once_applied() {
    let dir = tempfile::tempdir().expect("tempdir");
    let typed = "services:\n  web:\n    image: nginx:alpine   \n\n\n";
    let plan = compose_plan(dir.path(), typed).expect("plan");
    berth_rules::write::apply(&plan).expect("write");
    let written = std::fs::read_to_string(dir.path().join("compose.yaml")).expect("file");
    assert_eq!(written, "services:\n  web:\n    image: nginx:alpine\n");
}

#[test]
fn new_project_plan_refuses_an_empty_file_and_a_folder_that_has_compose() {
    let dir = tempfile::tempdir().expect("tempdir");
    let empty = compose_plan(dir.path(), "  \n\n").expect_err("empty");
    assert!(empty.contains("empty"), "{empty}");

    for name in [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yaml",
        "docker-compose.yml",
    ] {
        let other = tempfile::tempdir().expect("tempdir");
        std::fs::write(other.path().join(name), "services: {}\n").expect("write");
        let err = compose_plan(other.path(), "services: {}\n").expect_err(name);
        assert!(err.contains(name), "{err}");
        // The file that was there is untouched.
        assert_eq!(
            std::fs::read_to_string(other.path().join(name)).expect("read"),
            "services: {}\n"
        );
    }
}

#[test]
fn new_project_review_is_blocked_until_there_is_a_folder_and_text() {
    let folder = Path::new("/tmp/x");
    assert_eq!(
        review_blocker(None, STARTER, None).as_deref(),
        Some("Choose the folder to save it in.")
    );
    assert_eq!(
        review_blocker(Some(folder), "  ", None).as_deref(),
        Some("Write the compose file first.")
    );
    assert_eq!(review_blocker(Some(folder), STARTER, None), None);
    // The mock cannot write: its reason wins.
    assert_eq!(
        review_blocker(Some(folder), STARTER, Some("mock".to_string())).as_deref(),
        Some("mock")
    );
}

#[test]
fn new_project_the_editor_script_listens_to_its_own_editor_only() {
    let options = EditorOptions {
        lang: Lang::Yaml,
        value: STARTER.to_string(),
        read_only: false,
        theme: Theme::Light,
    };
    let script = editor_script(&options);
    assert!(script.contains("\"new-project\""));
    assert!(script.contains("message.id === id"));
    assert!(script.contains("berth.editor.create(id, options)"));
    assert!(script.contains("\"lang\":\"yaml\""));
}

#[test]
fn new_project_starter_is_valid_compose_for_the_plan() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plan = compose_plan(dir.path(), STARTER).expect("plan");
    let added: Vec<&str> = plan.files[0]
        .hunks
        .iter()
        .flat_map(|h| h.lines.iter().map(|l| l.text.as_str()))
        .collect();
    assert_eq!(added.first().copied(), Some("services:"));
}
