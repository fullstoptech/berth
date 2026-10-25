//! ColumnBrowser tests (P5-T4): markers, counts, the virtual window and the
//! failure states, rendered with dioxus-ssr.

use std::path::{Path, PathBuf};

use berth_rules::detect::FolderMarkers;

use super::*;
use crate::services::folders::{Column, Details};

fn entry(parent: &str, name: &str, details: Option<Details>) -> Entry {
    Entry {
        name: name.to_string(),
        path: Path::new(parent).join(name),
        hidden: name.starts_with('.'),
        details,
    }
}

fn items(n: usize) -> Option<Details> {
    Some(Details {
        markers: FolderMarkers::default(),
        items: Some(n),
    })
}

fn marked(compose: bool, dockerfile: bool) -> Option<Details> {
    Some(Details {
        markers: FolderMarkers {
            compose,
            dockerfile,
        },
        items: Some(2),
    })
}

fn browser_of(columns: Vec<Column>) -> Browser {
    Browser {
        columns,
        filter: String::new(),
        show_hidden: false,
    }
}

fn ready(folder: &str, entries: Vec<Entry>, selected: Option<&str>) -> Column {
    Column {
        folder: PathBuf::from(folder),
        state: ColumnState::Ready(entries),
        selected: selected.map(PathBuf::from),
    }
}

#[derive(Props, Clone, PartialEq)]
struct Harness {
    browser: Browser,
    known: Known,
}

#[component]
fn Host(props: Harness) -> Element {
    let browser = use_signal(|| props.browser.clone());
    let known = use_memo(move || props.known.clone());
    rsx! {
        ColumnBrowser {
            browser,
            known,
            on_select: move |_| {},
            on_open: move |_| {},
            on_open_settings: move |()| {},
            on_system_dialog: move |_| {},
            on_other: move |()| {},
        }
    }
}

fn render(browser: Browser, known: Known) -> String {
    let mut dom = VirtualDom::new_with_props(Host, Harness { browser, known });
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

fn known_of(folder: &str) -> Known {
    let engine = berth_core::EngineId("colima:default".to_string());
    let project = berth_core::Project {
        id: berth_core::ProjectId::for_folder(Path::new(folder), &engine),
        name: "known".to_string(),
        folder: Some(PathBuf::from(folder)),
        compose_files: Vec::new(),
        engine,
        source: berth_core::ProjectSource::Compose,
        services: Vec::new(),
        git: None,
        status: berth_core::ProjectStatus::AllRunning,
        pinned: false,
    };
    Known::from_projects(&[project])
}

#[test]
fn column_browser_rows_show_markers_counts_and_the_known_marker() {
    let entries = vec![
        entry("/d", "analytics", items(7)),
        entry("/d", "landing-v2", marked(true, false)),
        entry("/d", "new-app", marked(false, true)),
        entry("/d", "scratch", items(2)),
        entry("/d", "unread", None),
    ];
    let html = render(
        browser_of(vec![ready("/d", entries, Some("/d/new-app"))]),
        known_of("/d/analytics"),
    );
    // The known folder reads "In Berth" even though it has a count.
    assert!(html.contains(">In Berth<"), "{html}");
    assert!(
        !html.contains("7 items"),
        "a known folder shows the marker, not the count: {html}"
    );
    assert!(html.contains(">Compose<"), "{html}");
    assert!(html.contains(">Dockerfile<"), "{html}");
    assert!(html.contains(">2 items<"), "{html}");
    // A row not yet annotated shows nothing on its right.
    let unread = html.find("unread").expect("row");
    let after = &html[unread..];
    let row_end = after.find("cb-chevron").expect("chevron");
    assert!(
        !after[..row_end].contains("cb-meta"),
        "{}",
        &after[..row_end]
    );
    // The highlighted row of the last column is the focused one.
    assert!(
        html.contains("class=\"cb-row cb-row-selected cb-row-focus\""),
        "{html}"
    );
}

#[test]
fn column_browser_a_highlight_in_an_earlier_column_is_not_the_focused_one() {
    let first = ready("/", vec![entry("/", "d", items(1))], Some("/d"));
    let second = ready("/d", vec![entry("/d", "x", items(1))], None);
    let html = render(browser_of(vec![first, second]), Known::default());
    assert!(html.contains("class=\"cb-row cb-row-selected\""), "{html}");
    assert!(
        !html.contains("class=\"cb-row cb-row-selected cb-row-focus\""),
        "{html}"
    );
}

#[test]
fn column_browser_hidden_folders_wait_for_the_switch_and_the_filter_narrows_the_last_column() {
    let entries = vec![
        entry("/d", ".cache", items(1)),
        entry("/d", "alpha", items(1)),
        entry("/d", "beta", items(1)),
    ];
    let mut browser = browser_of(vec![ready("/d", entries, None)]);
    let html = render(browser.clone(), Known::default());
    assert!(!html.contains(".cache"), "{html}");
    assert!(html.contains("alpha") && html.contains("beta"));

    browser.show_hidden = true;
    assert!(render(browser.clone(), Known::default()).contains(".cache"));

    browser.show_hidden = false;
    browser.filter = "bet".to_string();
    let html = render(browser.clone(), Known::default());
    assert!(html.contains("beta") && !html.contains("alpha"), "{html}");
    browser.filter = "zzz".to_string();
    assert!(
        render(browser, Known::default()).contains("matches"),
        "an empty result says what was filtered"
    );
}

#[test]
fn column_browser_a_large_folder_renders_only_a_window_of_rows() {
    let entries: Vec<Entry> = (0..5000)
        .map(|i| entry("/big", &format!("folder-{i:04}"), items(i % 7)))
        .collect();
    let html = render(
        browser_of(vec![ready("/big", entries, None)]),
        Known::default(),
    );
    let rows = html.matches("class=\"cb-row").count();
    assert!(rows > 0, "{html}");
    assert!(
        rows <= 540 / ROW_HEIGHT_BASE_PX as usize + 2 * DEFAULT_OVERSCAN + 1,
        "{rows} rows in the DOM"
    );
    assert!(
        html.contains("aria-rowcount=\"5000\""),
        "the list knows its length"
    );
    assert!(html.contains("folder-0000"), "the first rows are there");
    assert!(!html.contains("folder-4999"), "the last row is not");
    // The spacer below stands in for the rest.
    let rest = (5000 - rows) as u64 * u64::from(ROW_HEIGHT_BASE_PX);
    assert!(html.contains(&format!("height: {rest}px;")), "{html}");
}

#[test]
fn column_browser_a_column_being_read_shows_a_skeleton_and_a_slow_note() {
    let loading = |slow| Column {
        folder: PathBuf::from("/Volumes/Backup"),
        state: ColumnState::Loading { slow },
        selected: None,
    };
    let html = render(browser_of(vec![loading(false)]), Known::default());
    assert!(html.contains("aria-busy=\"true\""), "{html}");
    assert!(html.contains("Reading Backup"), "{html}");
    assert!(!html.contains("slow to answer"));
    let html = render(browser_of(vec![loading(true)]), Known::default());
    assert!(html.contains("Backup is slow to answer"), "{html}");
}

#[test]
fn column_browser_empty_folder_says_so() {
    let html = render(
        browser_of(vec![ready("/d/empty", Vec::new(), None)]),
        Known::default(),
    );
    assert!(html.contains("No folders in empty"), "{html}");
}

#[test]
fn column_browser_no_access_has_the_macos_words_and_both_actions() {
    let denied = Column {
        folder: PathBuf::from("/Users/me/Documents"),
        state: ColumnState::Failed(FolderError::NoAccess(PathBuf::from("/Users/me/Documents"))),
        selected: None,
    };
    let html = render(browser_of(vec![denied]), Known::default());
    for text in [
        "Berth can&#39;t read Documents",
        "macOS asks you to allow access.",
        "Privacy &#38; Security",
        "No need to restart Berth after allowing.",
        "Open System Settings",
        "Choose another folder",
        "Choose Documents with the system dialog",
        "Grants access in one step",
    ] {
        assert!(html.contains(text), "{text}: {html}");
    }
}

#[test]
fn column_browser_not_found_offers_no_system_settings() {
    let gone = Column {
        folder: PathBuf::from("/Users/me/gone"),
        state: ColumnState::Failed(FolderError::NotFound(PathBuf::from("/Users/me/gone"))),
        selected: None,
    };
    let html = render(browser_of(vec![gone]), Known::default());
    assert!(html.contains("gone is not there"), "{html}");
    assert!(!html.contains("Open System Settings"), "{html}");
    assert!(html.contains("Choose another folder"), "{html}");
}
