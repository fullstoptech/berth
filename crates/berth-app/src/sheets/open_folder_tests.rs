//! Open folder sheet tests (P5-T4): the two frames as scenes, the pure
//! helpers, and the states the sheet adds around the columns.

use std::path::{Path, PathBuf};

use super::*;
use crate::scenes::render_scene_html;

const BROWSER: &str = "Open folder / browser / 1440";
const NO_ACCESS: &str = "Open folder / browser / no access / 1440";

fn place(name: &str, path: &str, kind: PlaceKind) -> Place {
    Place {
        name: name.to_string(),
        path: PathBuf::from(path),
        kind,
    }
}

fn known_at(folder: &str, name: &str) -> Known {
    let engine = berth_core::EngineId("colima:default".to_string());
    Known::from_projects(&[berth_core::Project {
        id: berth_core::ProjectId::for_folder(Path::new(folder), &engine),
        name: name.to_string(),
        folder: Some(PathBuf::from(folder)),
        compose_files: Vec::new(),
        engine,
        source: berth_core::ProjectSource::Compose,
        services: Vec::new(),
        git: None,
        status: berth_core::ProjectStatus::AllRunning,
        pinned: false,
    }])
}

// --- The browser frame ---

#[test]
fn open_folder_browser_scene_shows_places_columns_markers_and_the_preview() {
    let html = render_scene_html(BROWSER).expect("scene renders");
    for text in [
        "aria-label=\"Open folder\"",
        ">Places<",
        ">Recent<",
        ">Home<",
        ">Dev<",
        ">Macintosh HD<",
        ">shop-api<",
        "Filter this folder",
        ">Hidden files<",
        // Columns: the home, Dev, side.
        "data-columns=\"3\"",
        ">landing-v2<",
        ">Compose<",
        ">Dockerfile<",
        ">In Berth<",
        ">4 items<",
        // The preview of new-app.
        "Found",
        "FROM node:22-alpine",
        "6 keys, values stay hidden",
        "not found",
        "Node 22 \u{b7} pnpm \u{b7} Next.js 15",
        "Services needed",
        "Postgres",
        "(from DATABASE_URL)",
        "Redis",
        "Use system dialog\u{2026}",
        ">Cancel<",
        ">Open new-app<",
    ] {
        assert!(html.contains(text), "{text}: {html}");
    }
    assert!(
        !html.contains("btn-primary size-md\" disabled"),
        "Open is enabled: {html}"
    );
}

#[test]
fn open_folder_browser_scene_marks_known_projects_in_berth_not_their_item_counts() {
    let html = render_scene_html(BROWSER).expect("scene renders");
    // analytics (7 items) and docs-site (9 items) are projects in the mock world.
    assert_eq!(html.matches(">In Berth<").count(), 2, "{html}");
    for name in ["analytics", "docs-site"] {
        let at = html
            .find(&format!("<span class=\"cb-name\">{name}</span>"))
            .expect("row");
        let row = &html[at..at + 120];
        assert!(
            row.contains(">In Berth<") && !row.contains("items"),
            "{row}"
        );
    }
}

#[test]
fn open_folder_browser_scene_path_bar_shows_the_folder_being_looked_at() {
    let html = render_scene_html(BROWSER).expect("scene renders");
    assert!(html.contains("of-path-input"), "{html}");
    assert!(html.contains("~/Dev/side/"), "{html}");
}

// --- The no-access frame ---

#[test]
fn open_folder_no_access_scene_maps_eperm_to_the_message_and_the_settings_action() {
    let html = render_scene_html(NO_ACCESS).expect("scene renders");
    for text in [
        "Berth can&#39;t read Documents",
        "Open System Settings",
        "Choose another folder",
        "Choose Documents with the system dialog",
        "Select a folder to see what Berth finds",
        "~/Documents",
    ] {
        assert!(html.contains(text), "{text}: {html}");
    }
    // The primary action has nothing it can open, and says so.
    assert!(html.contains(">Open folder<"), "{html}");
    assert!(
        html.contains("data-tooltip=\"Berth can&#39;t read this folder\""),
        "{html}"
    );
    // The columns are replaced by the state.
    assert!(!html.contains("data-columns"), "{html}");
}

// --- The hidden files shortcut ---

#[component]
fn ToggleHost(provided: bool) -> Element {
    let mut state = crate::state::use_app_state();
    let view = provided.then(provide_folder_view);
    let seen = |view: &Option<FolderView>| view.map(|v| *v.show_hidden.peek());
    let before = seen(&view);
    toggle_hidden(&mut state, "folders.toggle_hidden");
    let once = seen(&view);
    toggle_hidden(&mut state, "folders.toggle_hidden");
    let twice = seen(&view);
    rsx! { span { "{before:?} {once:?} {twice:?}" } }
}

fn toggled(provided: bool) -> String {
    let mut dom = VirtualDom::new_with_props(ToggleHost, ToggleHostProps { provided });
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

#[test]
fn open_folder_shortcut_toggles_hidden_folders_and_does_nothing_without_the_sheet_state() {
    assert!(
        toggled(true).contains("Some(false) Some(true) Some(false)"),
        "{}",
        toggled(true)
    );
    // No view provided: the keys reach nothing and nothing breaks.
    assert!(
        toggled(false).contains("None None None"),
        "{}",
        toggled(false)
    );
}

// --- Helpers ---

#[test]
fn open_folder_primary_says_open_go_to_or_why_not() {
    let none = Known::default();
    let folder = Path::new("/Users/me/Dev/side/new-app");
    assert_eq!(
        primary_label(Some(folder), &none),
        ("Open new-app".to_string(), None)
    );
    assert_eq!(
        primary_label(
            Some(folder),
            &known_at("/Users/me/Dev/side/new-app", "new-app")
        ),
        ("Go to new-app".to_string(), None)
    );
    // A project may carry a name other than its folder's.
    assert_eq!(
        primary_label(
            Some(folder),
            &known_at("/Users/me/Dev/side/new-app", "shop-api")
        )
        .0,
        "Go to shop-api"
    );
    let (label, why) = primary_label(None, &none);
    assert_eq!(label, "Open folder");
    assert_eq!(why.as_deref(), Some("Berth can't read this folder"));
}

#[test]
fn open_folder_the_place_that_holds_the_folder_is_the_longest_match() {
    let places = vec![
        place("Home", "/Users/me", PlaceKind::Home),
        place("Dev", "/Users/me/Dev", PlaceKind::Favourite),
        place("Documents", "/Users/me/Documents", PlaceKind::Standard),
        place("Backup", "/Volumes/Backup", PlaceKind::Volume),
    ];
    assert_eq!(
        active_place(&places, Path::new("/Users/me/Dev/side")),
        Some(1)
    );
    assert_eq!(active_place(&places, Path::new("/Users/me/Dev")), Some(1));
    assert_eq!(active_place(&places, Path::new("/Users/me/Music")), Some(0));
    assert_eq!(
        active_place(&places, Path::new("/Users/me/Documents")),
        Some(2)
    );
    assert_eq!(
        active_place(&places, Path::new("/Volumes/Backup/x")),
        Some(3)
    );
    assert_eq!(active_place(&places, Path::new("/opt")), None);
    // `/Users/me/Developer` is not inside `/Users/me/Dev`.
    assert_eq!(
        active_place(&places, Path::new("/Users/me/Developer")),
        Some(0)
    );
}
