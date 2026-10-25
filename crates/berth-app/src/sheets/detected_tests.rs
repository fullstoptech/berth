//! Tests of the Detected sheet (P5-T5): the frame, the compose-present state, and what the real
//! fixture folders say.

use std::path::PathBuf;

use super::*;
use crate::scenes::render_scene_html;

const FRAME: &str = "New project / detected / 1440";
const COMPOSE: &str = "Detected / compose present / 1440";

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/folders")
        .join(name)
}

/// The sheet's own markup: the Projects list behind it has Start project buttons too.
fn sheet(html: &str) -> &str {
    let at = html.find("class=\"dt-sheet\"").expect("the sheet is drawn");
    &html[at..]
}

#[test]
fn detected_frame_lists_what_berth_found_in_the_folder() {
    let html = render_scene_html(FRAME).expect("scene renders");
    for text in [
        "aria-label=\"new-app detected\"",
        ">new-app<",
        "~/Dev/side/new-app",
        ">Found<",
        "Dockerfile",
        "FROM node:22-alpine",
        "6 keys, values stay hidden",
        "not found",
        ">Stack<",
        "Node 22 \u{b7} pnpm \u{b7} Next.js 15",
        ">Services needed<",
        ">Postgres<",
        "(from DATABASE_URL)",
        ">Redis<",
        "(from ioredis in package.json)",
        "Nothing is written until you review the changes.",
        "Generate compose.yaml",
    ] {
        assert!(html.contains(text), "missing {text:?}\n{html}");
    }
    // A folder without Compose is generated for, not started.
    assert!(!sheet(&html).contains(">Start project<"), "{html}");
}

#[test]
fn detected_never_prints_an_env_value() {
    let html = render_scene_html(FRAME).expect("scene renders");
    assert!(html.contains("values stay hidden"));
    assert!(!html.contains("DATABASE_URL="), "{html}");
}

#[test]
fn detected_folder_with_compose_offers_start_project_and_leaves_the_file_alone() {
    let html = render_scene_html(COMPOSE).expect("scene renders");
    assert!(sheet(&html).contains(">Start project<"), "{html}");
    assert!(
        html.contains("Berth does not change compose.yaml."),
        "{html}"
    );
    assert!(!html.contains("Generate compose.yaml"), "{html}");
    assert!(!html.contains(">Options<"), "{html}");
}

#[test]
fn detected_found_rows_follow_the_detection() {
    let d = crate::scenes::next_detection();
    let rows = found_list(&d);
    assert_eq!(
        rows,
        [
            ("Dockerfile", true, "FROM node:22-alpine".to_string()),
            (".env", true, "3 keys, values stay hidden".to_string()),
            ("compose.yaml", false, "not found".to_string()),
        ]
    );
}

#[test]
fn detected_stack_text_names_the_framework_version_and_says_when_there_is_none() {
    let mut d = crate::scenes::next_detection();
    assert_eq!(stack_text(&d), "Node 22 \u{b7} pnpm \u{b7} Next.js 15");
    d.stack.clear();
    assert_eq!(stack_text(&d), "No known stack");
}

#[test]
fn detected_waiting_note_counts_the_folders_behind() {
    assert_eq!(waiting_note(0), None);
    assert_eq!(
        waiting_note(1).as_deref(),
        Some("1 more folder is waiting.")
    );
    assert_eq!(
        waiting_note(3).as_deref(),
        Some("3 more folders are waiting.")
    );
}

#[test]
fn detected_reads_the_node_fixture_as_the_frame_does() {
    let d = berth_rules::detect::detect(&fixture("node-next-pnpm")).expect("detect");
    assert!(stack_text(&d).contains("Next.js"), "{}", stack_text(&d));
    let rows = found_list(&d);
    assert!(rows.iter().any(|(f, yes, _)| *f == "Dockerfile" && !yes) || rows.len() == 3);
    assert!(
        d.services_needed
            .iter()
            .any(|s| service_label(s.kind) == "Postgres" && s.reason.contains("DATABASE_URL")),
        "{:?}",
        d.services_needed
    );
    assert!(compose_files(&d).is_empty());
}

#[test]
fn detected_compose_fixture_names_its_compose_file_for_start_project() {
    let d = berth_rules::detect::detect(&fixture("compose-present")).expect("detect");
    let files = compose_files(&d);
    assert!(!files.is_empty());
    for file in &files {
        assert!(
            file.components().count() == 1,
            "{file:?} is relative to the folder"
        );
    }
}

#[test]
fn detected_static_site_fixture_is_generated_for() {
    let d = berth_rules::detect::detect(&fixture("static-site")).expect("detect");
    assert!(compose_files(&d).is_empty());
    assert!(crate::sheets::generate::blocker(&d).is_none());
}
