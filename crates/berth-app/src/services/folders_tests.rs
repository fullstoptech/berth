//! Tests of the folder service (P5-T4). Every folder read here is a temp
//! dir the test made.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use berth_core::{
    EngineId, Project, ProjectId, ProjectSource, ProjectStatus, ServiceKind, StackGuess,
};

use super::*;

fn dir(root: &Path, rel: &str) -> PathBuf {
    let path = root.join(rel);
    std::fs::create_dir_all(&path).expect("mkdir");
    path
}

fn file(root: &Path, rel: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(path, "x: 1\n").expect("write");
}

fn names(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|e| e.name.as_str()).collect()
}

fn engine() -> EngineId {
    EngineId("colima:default".to_string())
}

fn project(folder: &Path, name: &str) -> Project {
    let engine = engine();
    Project {
        id: ProjectId::for_folder(folder, &engine),
        name: name.to_string(),
        folder: Some(folder.to_path_buf()),
        compose_files: Vec::new(),
        engine,
        source: ProjectSource::Compose,
        services: Vec::new(),
        status: ProjectStatus::AllRunning,
        git: None,
        pinned: false,
    }
}

/// Makes a folder unreadable for the length of a test and puts the mode back
/// on drop, so the temp dir can be removed even when an assertion fails.
struct Locked(PathBuf);

impl Locked {
    fn new(path: &Path) -> Self {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        Self(path.to_path_buf())
    }
}

impl Drop for Locked {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

// --- Errors ---

#[test]
fn folders_unreadable_folder_reads_as_no_access() {
    let tmp = tempfile::tempdir().expect("tmp");
    let locked = dir(tmp.path(), "Documents");
    let guard = Locked::new(&locked);
    let err = read_folder(&locked).expect_err("a folder with mode 000 cannot be listed");
    drop(guard);
    assert_eq!(err, FolderError::NoAccess(locked));
    assert_eq!(err.headline(), "Berth can't read Documents");
    assert!(err.is_no_access());
}

#[test]
fn folders_eperm_from_macos_privacy_maps_to_no_access() {
    // macOS answers a folder without a Files and Folders grant with EPERM (1),
    // which is not the mode-bit EACCES (13) the temp folder gives.
    let path = Path::new("/Users/me/Documents");
    for code in [1, 13] {
        let err = FolderError::from_io(path, &io::Error::from_raw_os_error(code));
        assert_eq!(
            err,
            FolderError::NoAccess(path.to_path_buf()),
            "errno {code}"
        );
    }
    let missing = FolderError::from_io(path, &io::Error::from_raw_os_error(2));
    assert_eq!(missing, FolderError::NotFound(path.to_path_buf()));
    let other = FolderError::from_io(path, &io::Error::from_raw_os_error(5));
    assert!(matches!(other, FolderError::Failed(..)), "{other:?}");
}

#[test]
fn folders_missing_folder_and_a_file_are_told_apart() {
    let tmp = tempfile::tempdir().expect("tmp");
    file(tmp.path(), "plain.txt");
    assert!(matches!(
        read_folder(&tmp.path().join("nope")),
        Err(FolderError::NotFound(_))
    ));
    assert!(matches!(
        read_folder(&tmp.path().join("plain.txt")),
        Err(FolderError::NotAFolder(_))
    ));
    assert_eq!(classify_path(&tmp.path().join("nope")), PathState::Missing);
    assert_eq!(
        classify_path(&tmp.path().join("plain.txt")),
        PathState::NotAFolder
    );
    assert_eq!(classify_path(tmp.path()), PathState::Folder);
}

// --- Listing: one level, folders, sorted ---

#[test]
fn folders_lists_one_level_of_folders_sorted_ignoring_case() {
    let tmp = tempfile::tempdir().expect("tmp");
    for name in ["beta", "Alpha", "alpha2", ".hidden", "zeta", "Zebra"] {
        dir(tmp.path(), name);
    }
    file(tmp.path(), "notes.txt");
    file(tmp.path(), "beta/deep/inside.txt");
    let entries = read_folder(tmp.path()).expect("list");
    assert_eq!(
        names(&entries),
        [".hidden", "Alpha", "alpha2", "beta", "Zebra", "zeta"],
        "files are not listed, children of children are not read"
    );
    assert!(entries[0].hidden && !entries[1].hidden);
    assert!(entries.iter().all(|e| e.details.is_none()));
}

#[test]
fn folders_listing_does_not_open_what_is_below() {
    let tmp = tempfile::tempdir().expect("tmp");
    let child = dir(tmp.path(), "private");
    dir(&child, "inner");
    let guard = Locked::new(&child);
    let entries = read_folder(tmp.path());
    drop(guard);
    let entries = entries.expect("a locked child does not fail the parent");
    assert_eq!(names(&entries), ["private"]);
}

#[test]
fn folders_symlink_to_a_folder_is_listed_a_symlink_to_a_file_is_not() {
    let tmp = tempfile::tempdir().expect("tmp");
    let target = dir(tmp.path(), "real");
    file(tmp.path(), "plain.txt");
    std::os::unix::fs::symlink(&target, tmp.path().join("link")).expect("symlink");
    std::os::unix::fs::symlink(tmp.path().join("plain.txt"), tmp.path().join("filelink"))
        .expect("symlink");
    let entries = read_folder(tmp.path()).expect("list");
    assert_eq!(names(&entries), ["link", "real"]);
}

// --- Markers and counts ---

#[test]
fn folders_annotate_reads_markers_and_counts_visible_entries() {
    let tmp = tempfile::tempdir().expect("tmp");
    let home = tmp.path();
    let app = dir(home, "app");
    file(&app, "compose.yaml");
    file(&app, "Dockerfile");
    file(&app, ".env");
    dir(&app, "src");
    let docker_only = dir(home, "svc");
    file(&docker_only, "Dockerfile");
    let bare = dir(home, "bare");

    let a = annotate(&app, home);
    assert_eq!(
        a.markers,
        FolderMarkers {
            compose: true,
            dockerfile: true
        }
    );
    assert_eq!(a.items, Some(3), ".env is hidden and not counted");
    assert!(!annotate(&docker_only, home).markers.compose);
    assert!(annotate(&docker_only, home).markers.dockerfile);
    assert_eq!(
        annotate(&bare, home),
        Details {
            markers: FolderMarkers::default(),
            items: Some(0)
        }
    );
}

#[test]
fn folders_privacy_protected_children_of_home_are_not_read_to_draw_a_column() {
    let tmp = tempfile::tempdir().expect("tmp");
    let home = tmp.path();
    let docs = dir(home, "Documents");
    file(&docs, "compose.yaml");
    assert_eq!(annotate(&docs, home), Details::default());
    // Only the three prompting folders of the home itself.
    assert!(is_privacy_protected(&home.join("Desktop"), home));
    assert!(is_privacy_protected(&home.join("Downloads"), home));
    assert!(!is_privacy_protected(&home.join("Dev"), home));
    assert!(!is_privacy_protected(&home.join("Dev/Documents"), home));
}

#[test]
fn folders_item_count_stops_at_the_cap() {
    let tmp = tempfile::tempdir().expect("tmp");
    for i in 0..30 {
        dir(tmp.path(), &format!("d{i}"));
    }
    assert_eq!(count_items(tmp.path()), Some(30));
    assert_eq!(count_items(&tmp.path().join("missing")), None);
}

#[test]
fn folders_marker_priority_is_known_then_compose_then_dockerfile() {
    let both = FolderMarkers {
        compose: true,
        dockerfile: true,
    };
    let docker = FolderMarkers {
        compose: false,
        dockerfile: true,
    };
    assert_eq!(marker_of(both, true), Some(Marker::InBerth));
    assert_eq!(marker_of(both, false), Some(Marker::Compose));
    assert_eq!(marker_of(docker, false), Some(Marker::Dockerfile));
    assert_eq!(marker_of(FolderMarkers::default(), false), None);
    assert_eq!(Marker::InBerth.label(), "In Berth");
}

#[test]
fn folders_known_projects_are_found_by_folder() {
    let folder = Path::new("/Users/me/Dev/analytics");
    let known = Known::from_projects(&[
        project(folder, "analytics"),
        Project {
            folder: None,
            ..project(Path::new("/x"), "remote")
        },
    ]);
    assert!(known.contains(folder));
    assert_eq!(
        known.get(folder).map(|k| k.name.as_str()),
        Some("analytics")
    );
    assert!(!known.contains(Path::new("/Users/me/Dev/other")));
}

#[test]
fn folders_annotate_all_fills_every_chunk_and_stops_when_cancelled() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut paths = Vec::new();
    for i in 0..(ANNOTATE_CHUNK * 2 + 5) {
        let d = dir(tmp.path(), &format!("d{i:04}"));
        if i == 3 {
            file(&d, "compose.yaml");
        }
        paths.push(d);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    let mut got = Vec::new();
    rt.block_on(annotate_all(
        paths.clone(),
        tmp.path().to_path_buf(),
        CancellationToken::new(),
        |chunk| got.extend(chunk),
    ));
    assert_eq!(got.len(), paths.len());
    let third = got.iter().find(|(p, _)| *p == paths[3]).expect("annotated");
    assert!(third.1.markers.compose);

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let mut none = 0;
    rt.block_on(annotate_all(
        paths,
        tmp.path().to_path_buf(),
        cancelled,
        |chunk| none += chunk.len(),
    ));
    assert_eq!(none, 0, "a cancelled read hands nothing over");
}

// --- Typed paths ---

#[test]
fn folders_display_and_resolve_round_trip_the_home() {
    let home = Path::new("/Users/me");
    assert_eq!(display_path(Path::new("/Users/me"), home), "~");
    assert_eq!(
        display_path(Path::new("/Users/me/Dev/side"), home),
        "~/Dev/side"
    );
    assert_eq!(
        display_path(Path::new("/Volumes/X/a"), home),
        "/Volumes/X/a"
    );
    let base = Path::new("/Users/me/Dev");
    assert_eq!(
        resolve_input("~/Dev/side/", base, home),
        Path::new("/Users/me/Dev/side")
    );
    assert_eq!(resolve_input("~", base, home), home);
    assert_eq!(resolve_input("/etc/", base, home), Path::new("/etc"));
    assert_eq!(
        resolve_input("side/../new", base, home),
        Path::new("/Users/me/Dev/new")
    );
    assert_eq!(resolve_input("  ", base, home), base);
    assert_eq!(resolve_input("/", base, home), Path::new("/"));
}

#[test]
fn folders_complete_names_finishes_single_matches_and_shared_starts() {
    let list = |v: &[&str]| v.iter().map(ToString::to_string).collect::<Vec<_>>();
    let names = list(&["side", "scratch", "Documents", "docs-site", "docs-old"]);
    assert_eq!(complete_names("si", &names), Some("side/".to_string()));
    assert_eq!(
        complete_names("doc", &names),
        None,
        "Documents and docs-* share only `doc`"
    );
    assert_eq!(complete_names("docs", &names), Some("docs-".to_string()));
    assert_eq!(
        complete_names("docs-", &names),
        None,
        "nothing more is shared"
    );
    assert_eq!(
        complete_names("docs-s", &names),
        Some("docs-site/".to_string())
    );
    assert_eq!(complete_names("zz", &names), None);
    assert_eq!(
        complete_names("S", &names),
        None,
        "side and scratch share only `s`"
    );
}

#[test]
fn folders_complete_path_keeps_what_was_typed_and_reads_one_folder() {
    let tmp = tempfile::tempdir().expect("tmp");
    let home = tmp.path();
    dir(home, "Dev/side/new-app");
    dir(home, "Dev/scratch");
    dir(home, "Dev/.cache");
    let base = home.join("Dev");
    assert_eq!(
        complete_path("~/Dev/si", &base, home, false),
        Some("~/Dev/side/".to_string())
    );
    assert_eq!(
        complete_path("si", &base, home, false),
        Some("side/".to_string()),
        "a relative path starts at the folder being looked at"
    );
    assert_eq!(
        complete_path("~/Dev/.c", &base, home, false),
        Some("~/Dev/.cache/".to_string())
    );
    assert_eq!(
        complete_path("~/Dev/", &base, home, false),
        Some("~/Dev/s".to_string()),
        "side and scratch share their first letter"
    );
    assert_eq!(
        complete_path("~/Dev/s", &base, home, false),
        None,
        "nothing more is shared"
    );
    assert_eq!(complete_path("~/Nowhere/x", &base, home, false), None);
}

// --- Places, recents, pins ---

#[test]
fn folders_places_list_home_favourites_standard_folders_and_volumes_once() {
    let tmp = tempfile::tempdir().expect("tmp");
    let volumes = dir(tmp.path(), "Volumes");
    dir(&volumes, "Macintosh HD");
    dir(&volumes, "Backup");
    dir(&volumes, ".hidden");
    file(&volumes, "stray.txt");
    let home = Path::new("/Users/me");
    let list = places(
        home,
        &[home.join("Dev"), home.join("Documents")],
        mounted_volumes(&volumes),
    );
    let got: Vec<(&str, PlaceKind)> = list.iter().map(|p| (p.name.as_str(), p.kind)).collect();
    assert_eq!(
        got,
        [
            ("Home", PlaceKind::Home),
            ("Dev", PlaceKind::Favourite),
            ("Documents", PlaceKind::Favourite),
            ("Desktop", PlaceKind::Standard),
            ("Backup", PlaceKind::Volume),
            ("Macintosh HD", PlaceKind::Volume),
        ],
        "a favourite is not listed twice; hidden entries and files are not volumes"
    );
    assert_eq!(list[0].icon(), "home");
    assert_eq!(list[4].icon(), "hard-drive");
}

#[test]
fn folders_pins_and_recents_round_trip_through_the_store() {
    let store = berth_store::Store::open_in_memory().expect("store");
    let known = Known::default();
    let t0 = chrono::DateTime::from_timestamp(1_792_000_000, 0).expect("time");
    let t1 = t0 + chrono::Duration::hours(1);
    let a = Path::new("/Users/me/Dev/side/new-app");
    let b = Path::new("/Users/me/Dev/clients/shop-api");

    assert_eq!(
        record_open(&store, a, &engine(), &known, t0).expect("a"),
        OpenOutcome::Pinned
    );
    assert_eq!(
        record_open(&store, b, &engine(), &known, t0).expect("b"),
        OpenOutcome::Pinned
    );
    // Opening a again moves it to the top of Recent and keeps its pin date.
    assert_eq!(
        record_open(&store, a, &engine(), &known, t1).expect("a again"),
        OpenOutcome::Pinned
    );

    let pins = store.pins().expect("pins");
    assert_eq!(pins.len(), 2);
    let pin_a = pins.iter().find(|p| p.folder == a).expect("pin a");
    assert_eq!(pin_a.engine, engine());
    assert_eq!(pin_a.added, t0, "a second open keeps the date of the first");

    let recent = recent_places(&store, RECENT_SHOWN);
    let got: Vec<&str> = recent.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(got, ["new-app", "shop-api"]);
    assert_eq!(recent[0].path, a);
}

#[test]
fn folders_opening_a_known_project_goes_to_it_and_pins_nothing() {
    let store = berth_store::Store::open_in_memory().expect("store");
    let folder = Path::new("/Users/me/Dev/analytics");
    let known = Known::from_projects(&[project(folder, "analytics")]);
    let outcome = record_open(&store, folder, &engine(), &known, chrono::Utc::now()).expect("open");
    assert_eq!(
        outcome,
        OpenOutcome::GoTo(ProjectId::for_folder(folder, &engine()))
    );
    assert!(store.pins().expect("pins").is_empty());
    assert_eq!(store.recent_folders(5).expect("recent"), [folder]);
}

#[test]
fn folders_start_folder_follows_the_last_open_then_the_favourite_then_home() {
    let home = Path::new("/Users/me");
    let recent = vec![Place {
        name: "new-app".to_string(),
        path: PathBuf::from("/Users/me/Dev/side/new-app"),
        kind: PlaceKind::Standard,
    }];
    let favs = [PathBuf::from("/Users/me/Dev")];
    assert_eq!(
        start_folder(&recent, &favs, home),
        Path::new("/Users/me/Dev/side")
    );
    assert_eq!(start_folder(&[], &favs, home), Path::new("/Users/me/Dev"));
    assert_eq!(start_folder(&[], &[], home), home);
}

#[test]
fn folders_pins_go_to_the_scoped_engine_then_the_default_then_a_running_one() {
    let a = EngineId("colima:a".to_string());
    let b = EngineId("colima:b".to_string());
    assert_eq!(pick_engine(Some(&a), Some(&b), &[]), a);
    assert_eq!(pick_engine(None, Some(&b), std::slice::from_ref(&a)), b);
    assert_eq!(pick_engine(None, None, &[a.clone(), b]), a);
    assert_eq!(pick_engine(None, None, &[]), engine());
}

// --- The browser model ---

fn ready(names: &[&str], parent: &str) -> Result<Vec<Entry>, FolderError> {
    Ok(names
        .iter()
        .map(|n| Entry {
            name: (*n).to_string(),
            path: Path::new(parent).join(n),
            hidden: n.starts_with('.'),
            details: None,
        })
        .collect())
}

fn browser() -> Browser {
    let home = Path::new("/Users/me");
    let mut b = Browser::at(Path::new("/Users/me/Dev/side"), home);
    b.set_listing(
        Path::new("/Users/me"),
        ready(&["Desktop", "Dev", ".config"], "/Users/me"),
    );
    b.set_listing(
        Path::new("/Users/me/Dev"),
        ready(&["analytics", "side"], "/Users/me/Dev"),
    );
    b.set_listing(
        Path::new("/Users/me/Dev/side"),
        ready(&["landing-v2", "new-app", "old-api"], "/Users/me/Dev/side"),
    );
    b
}

#[test]
fn folders_browser_starts_with_one_column_per_folder_from_the_home() {
    let home = Path::new("/Users/me");
    let b = Browser::at(Path::new("/Users/me/Dev/side"), home);
    let folders: Vec<&Path> = b.columns.iter().map(|c| c.folder.as_path()).collect();
    assert_eq!(
        folders,
        [
            Path::new("/Users/me"),
            Path::new("/Users/me/Dev"),
            Path::new("/Users/me/Dev/side")
        ]
    );
    assert!(b
        .columns
        .iter()
        .all(|c| matches!(c.state, ColumnState::Loading { .. })));
    // Each column highlights the folder the next one shows; the last none.
    assert_eq!(
        b.columns[0].selected.as_deref(),
        Some(Path::new("/Users/me/Dev"))
    );
    assert_eq!(b.columns[2].selected, None);
    // Outside the home the chain starts at the root.
    let outside = Browser::at(Path::new("/Volumes/Backup/x"), home);
    assert_eq!(outside.columns[0].folder, Path::new("/"));
    assert_eq!(outside.columns.len(), 4);
}

#[test]
fn folders_open_path_keeps_the_columns_it_shares_and_reports_the_new_ones() {
    let home = Path::new("/Users/me");
    let mut b = browser();
    let read = b.open_path(Path::new("/Users/me/Dev/analytics"), home);
    assert_eq!(read, [PathBuf::from("/Users/me/Dev/analytics")]);
    assert_eq!(b.columns.len(), 3);
    assert!(
        matches!(b.columns[0].state, ColumnState::Ready(_)),
        "the home column stays loaded"
    );
    assert!(matches!(b.columns[1].state, ColumnState::Ready(_)));
    assert!(matches!(b.columns[2].state, ColumnState::Loading { .. }));
    assert_eq!(
        b.columns[1].selected.as_deref(),
        Some(Path::new("/Users/me/Dev/analytics"))
    );
}

#[test]
fn folders_select_truncates_columns_that_no_longer_follow() {
    let mut b = browser();
    // Highlighting another folder in the middle column drops the third.
    b.select(1, Path::new("/Users/me/Dev/analytics"));
    assert_eq!(b.columns.len(), 2);
    assert_eq!(
        b.columns[1].selected.as_deref(),
        Some(Path::new("/Users/me/Dev/analytics"))
    );
    // Highlighting the folder the next column shows keeps it.
    let mut b = browser();
    b.select(1, Path::new("/Users/me/Dev/side"));
    assert_eq!(b.columns.len(), 3);
    // Highlighting in the last column adds nothing.
    b.select(2, Path::new("/Users/me/Dev/side/new-app"));
    assert_eq!(b.columns.len(), 3);
    assert_eq!(
        b.preview_target().as_deref(),
        Some(Path::new("/Users/me/Dev/side/new-app"))
    );
}

#[test]
fn folders_descend_adds_a_loading_column_once() {
    let mut b = browser();
    let new_app = Path::new("/Users/me/Dev/side/new-app");
    assert_eq!(b.descend(2, new_app), Some(new_app.to_path_buf()));
    assert_eq!(b.columns.len(), 4);
    assert_eq!(b.current_folder(), Some(new_app));
    assert_eq!(b.descend(2, new_app), None, "the column is already there");
    assert_eq!(b.columns.len(), 4);
}

#[test]
fn folders_up_drops_a_column_and_at_the_first_goes_to_the_parent() {
    let home = Path::new("/Users/me");
    let mut b = browser();
    assert!(b.up(home).is_empty());
    assert_eq!(b.columns.len(), 2);
    assert_eq!(b.current_folder(), Some(Path::new("/Users/me/Dev")));
    let mut b = Browser::at(Path::new("/Users/me"), home);
    let read = b.up(home);
    assert_eq!(b.columns[0].folder, Path::new("/"));
    assert_eq!(b.current_folder(), Some(Path::new("/Users")));
    assert_eq!(read, [PathBuf::from("/"), PathBuf::from("/Users")]);
    assert_eq!(
        b.columns.last().and_then(|c| c.selected.as_deref()),
        Some(Path::new("/Users/me")),
        "the folder just left stays highlighted"
    );
}

#[test]
fn folders_target_is_the_highlighted_folder_else_the_column_and_never_a_failed_one() {
    let mut b = browser();
    assert_eq!(b.target().as_deref(), Some(Path::new("/Users/me/Dev/side")));
    b.select(2, Path::new("/Users/me/Dev/side/new-app"));
    assert_eq!(
        b.target().as_deref(),
        Some(Path::new("/Users/me/Dev/side/new-app"))
    );

    let home = Path::new("/Users/me");
    let mut denied = Browser::at(Path::new("/Users/me/Documents"), home);
    denied.set_listing(
        Path::new("/Users/me/Documents"),
        Err(FolderError::NoAccess(PathBuf::from("/Users/me/Documents"))),
    );
    assert_eq!(
        denied.target(),
        None,
        "Open stays off for a folder that cannot be read"
    );
    assert_eq!(denied.preview_target(), None);
    assert_eq!(
        denied.failure().map(FolderError::headline),
        Some("Berth can't read Documents".to_string())
    );
}

#[test]
fn folders_visible_applies_hidden_files_and_filters_the_last_column_only() {
    let mut b = browser();
    // The home column hides `.config` until hidden files are shown.
    assert_eq!(b.visible(0), [0, 1]);
    b.show_hidden = true;
    assert_eq!(b.visible(0), [0, 1, 2]);
    b.show_hidden = false;
    // The filter is for the column being looked at.
    b.filter = "APP".to_string();
    assert_eq!(b.visible(2), [1], "new-app, any case");
    assert_eq!(b.visible(0), [0, 1], "other columns are not filtered");
    b.filter = "zzz".to_string();
    assert!(b.visible(2).is_empty());
}

#[test]
fn folders_step_moves_the_highlight_through_visible_rows_and_stays_in_range() {
    let mut b = browser();
    let side = |n: &str| Path::new("/Users/me/Dev/side").join(n);
    assert_eq!(
        b.step(1),
        Some(side("landing-v2")),
        "from nothing, down is the first"
    );
    assert_eq!(b.step(1), Some(side("new-app")));
    assert_eq!(b.step(5), Some(side("old-api")), "clamped to the last");
    assert_eq!(b.step(-1), Some(side("new-app")));
    b.filter = "old".to_string();
    assert_eq!(
        b.step(-1),
        Some(side("old-api")),
        "only filtered rows count"
    );
    b.columns[2].selected = None;
    b.filter.clear();
    assert_eq!(
        b.step(-1),
        Some(side("old-api")),
        "from nothing, up is the last"
    );
}

#[test]
fn folders_details_land_on_their_entries_and_late_results_are_dropped() {
    let mut b = browser();
    let side = PathBuf::from("/Users/me/Dev/side");
    let new_app = side.join("new-app");
    let details = Details {
        markers: FolderMarkers {
            compose: false,
            dockerfile: true,
        },
        items: Some(4),
    };
    b.set_details(&side, vec![(new_app.clone(), details)]);
    let ColumnState::Ready(entries) = &b.columns[2].state else {
        panic!("ready");
    };
    assert_eq!(entries[1].details, Some(details));
    assert_eq!(entries[0].details, None);
    // A folder that is not a column any more is ignored without a panic.
    b.set_details(Path::new("/gone"), vec![(new_app, details)]);
    b.set_listing(Path::new("/gone"), ready(&["x"], "/gone"));
    assert_eq!(b.columns.len(), 3);
}

#[test]
fn folders_a_slow_column_is_flagged_only_while_it_is_loading() {
    let home = Path::new("/Users/me");
    let mut b = Browser::at(Path::new("/Volumes/Backup"), home);
    b.mark_slow(Path::new("/Volumes/Backup"));
    assert_eq!(
        b.columns.last().map(|c| c.state.clone()),
        Some(ColumnState::Loading { slow: true })
    );
    b.set_listing(
        Path::new("/Volumes/Backup"),
        ready(&["a"], "/Volumes/Backup"),
    );
    b.mark_slow(Path::new("/Volumes/Backup"));
    assert!(matches!(b.columns[2].state, ColumnState::Ready(_)));
}

// --- Preview ---

#[test]
fn folders_preview_reads_the_named_files_and_counts_entries() {
    let tmp = tempfile::tempdir().expect("tmp");
    let app = dir(tmp.path(), "new-app");
    std::fs::write(app.join("Dockerfile"), "FROM node:22-alpine\n").expect("write");
    std::fs::write(app.join(".env"), "A=1\nB=2\n").expect("write");
    std::fs::write(
        app.join("package.json"),
        r#"{"dependencies":{"next":"^15.0.3","pg":"8"}}"#,
    )
    .expect("write");
    std::fs::write(app.join("pnpm-lock.yaml"), "lockfileVersion: 9\n").expect("write");
    dir(&app, "src");

    let preview = read_preview(&app).expect("preview");
    assert_eq!(
        preview.items, 4,
        "Dockerfile, package.json, pnpm-lock.yaml and src; .env hidden"
    );
    let rows = found_rows(&preview.detection);
    assert_eq!(rows[0].detail.as_deref(), Some("FROM node:22-alpine"));
    assert_eq!(
        rows[1].detail.as_deref(),
        Some("2 keys, values stay hidden")
    );
    assert_eq!(rows[2].detail, None, "no Compose file");
    assert_eq!(rows.map(|r| r.file), ["Dockerfile", ".env", "compose.yaml"]);
    let stack = preview.detection.stack.first().expect("stack");
    assert!(
        stack_label(stack).contains("Next.js 15"),
        "{}",
        stack_label(stack)
    );
}

#[test]
fn folders_preview_of_an_unreadable_folder_is_no_access() {
    let tmp = tempfile::tempdir().expect("tmp");
    let locked = dir(tmp.path(), "Documents");
    let guard = Locked::new(&locked);
    let err = read_preview(&locked);
    drop(guard);
    assert_eq!(err.expect_err("denied"), FolderError::NoAccess(locked));
}

#[test]
fn folders_stack_and_service_labels_read_like_the_frame() {
    let node = StackGuess::Node {
        pm: "pnpm".to_string(),
        framework: Some("Next.js".to_string()),
        framework_version: Some("15".to_string()),
        version: Some("22".to_string()),
    };
    assert_eq!(stack_label(&node), "Node 22 \u{b7} pnpm \u{b7} Next.js 15");
    assert_eq!(stack_label(&StackGuess::Go), "Go");
    assert_eq!(
        stack_label(&StackGuess::Python {
            tool: "uv".to_string(),
            framework: Some("FastAPI".to_string())
        }),
        "Python \u{b7} uv \u{b7} FastAPI"
    );
    assert_eq!(service_label(ServiceKind::Postgres), "Postgres");
    assert_eq!(service_label(ServiceKind::Minio), "S3 storage");
}

#[test]
fn folders_privacy_settings_url_is_the_files_and_folders_pane() {
    assert_eq!(
        PRIVACY_SETTINGS_URL,
        "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders"
    );
}
