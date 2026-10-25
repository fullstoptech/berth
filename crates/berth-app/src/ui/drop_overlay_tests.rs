//! Tests of dropping on the window (P5-T5): what a drop opens, what it records, and what the
//! overlay shows. A real Finder drag is an owner check; these feed `DropZone::finish` the exact
//! paths a drop delivers (`NativeFileHover`'s), against an in-memory store.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use berth_core::{EngineId, Project, ProjectId, ProjectSource, ProjectStatus};

use super::*;
use crate::scenes::render_scene_html;
use crate::state::use_app_state;

fn engine() -> EngineId {
    EngineId("colima:default".to_string())
}

fn project_at(folder: &Path) -> Project {
    Project {
        id: ProjectId::for_folder(folder, &engine()),
        name: folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        folder: Some(folder.to_path_buf()),
        compose_files: Vec::new(),
        engine: engine(),
        source: ProjectSource::Compose,
        services: Vec::new(),
        git: None,
        status: ProjectStatus::AllRunning,
        pinned: false,
    }
}

// --- A window to drop on ---

thread_local! {
    static SLOT: std::cell::RefCell<Option<(AppState, DropZone)>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Rig() -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    let mut zone = use_drop_zone(state);
    // The in-memory store stands in for the app's.
    zone.store = berth_store::Store::open_in_memory().ok().map(Arc::new);
    use_hook(move || SLOT.with(|slot| *slot.borrow_mut() = Some((state, zone.clone()))));
    rsx! { div {} }
}

fn rig() -> (VirtualDom, AppState, DropZone) {
    let mut dom = VirtualDom::new(Rig);
    let _ = dom.rebuild_to_vec();
    let (state, zone) = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("the rig captured the window");
    (dom, state, zone)
}

fn overlays(state: &AppState) -> Vec<Overlay> {
    state.overlays.peek().clone()
}

fn pins(zone: &DropZone) -> Vec<PathBuf> {
    zone.store
        .as_ref()
        .expect("store")
        .pins()
        .expect("pins")
        .into_iter()
        .map(|p| p.folder)
        .collect()
}

fn folder_in(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(&path).expect("mkdir");
    path
}

#[test]
fn drop_a_new_folder_is_pinned_listed_in_recent_and_shown_in_the_detected_sheet() {
    let root = tempfile::tempdir().expect("tempdir");
    let app = folder_in(root.path(), "new-app");
    let (mut dom, state, zone) = rig();
    dom.in_runtime(|| zone.finish(std::slice::from_ref(&app)));
    let _ = dom.render_immediate_to_vec();

    assert_eq!(overlays(&state), [Overlay::Detected(app.clone())]);
    assert_eq!(pins(&zone), std::slice::from_ref(&app));
    let store = zone.store.as_ref().expect("store");
    assert_eq!(store.recent_folders(5).expect("recent"), [app]);
}

#[test]
fn drop_a_folder_berth_already_has_jumps_to_its_project_and_pins_nothing() {
    let root = tempfile::tempdir().expect("tempdir");
    let analytics = folder_in(root.path(), "analytics");
    let (mut dom, mut state, zone) = rig();
    state.projects.set(vec![project_at(&analytics)]);
    // An earlier sheet is open: the jump closes it.
    state.overlays.write().push(Overlay::OpenFolder);
    dom.in_runtime(|| zone.finish(std::slice::from_ref(&analytics)));
    let _ = dom.render_immediate_to_vec();

    assert!(overlays(&state).is_empty(), "{:?}", overlays(&state));
    assert_eq!(*state.section.peek(), Section::Projects);
    assert_eq!(
        *state.selection.peek(),
        Selection::Project(ProjectId::for_folder(&analytics, &engine()))
    );
    assert!(pins(&zone).is_empty());
}

#[test]
fn drop_a_compose_file_or_a_dockerfile_opens_its_folder() {
    let root = tempfile::tempdir().expect("tempdir");
    let api = folder_in(root.path(), "api");
    let compose = api.join("compose.yaml");
    std::fs::write(&compose, "services: {}\n").expect("write");
    let (mut dom, state, zone) = rig();
    dom.in_runtime(|| zone.finish(std::slice::from_ref(&compose)));
    let _ = dom.render_immediate_to_vec();
    assert_eq!(overlays(&state), [Overlay::Detected(api.clone())]);

    let web = folder_in(root.path(), "web");
    let dockerfile = web.join("Dockerfile");
    std::fs::write(&dockerfile, "FROM scratch\n").expect("write");
    let (mut dom, state, zone) = rig();
    dom.in_runtime(|| zone.finish(&[dockerfile]));
    let _ = dom.render_immediate_to_vec();
    assert_eq!(overlays(&state), [Overlay::Detected(web)]);
}

#[test]
fn drop_the_wrong_file_type_says_so_and_opens_nothing() {
    let root = tempfile::tempdir().expect("tempdir");
    let notes = root.path().join("notes.txt");
    std::fs::write(&notes, "hello").expect("write");
    let (mut dom, state, zone) = rig();
    dom.in_runtime(|| zone.finish(&[notes]));
    let _ = dom.render_immediate_to_vec();

    assert!(overlays(&state).is_empty());
    assert!(pins(&zone).is_empty());
    let toasts = state.toasts.peek().clone();
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0].tone, ToastTone::Danger);
    assert!(
        toasts[0]
            .text
            .starts_with("Berth cannot open notes.txt. Drop a folder"),
        "{}",
        toasts[0].text
    );
}

#[test]
fn drop_several_folders_show_one_sheet_at_a_time() {
    let root = tempfile::tempdir().expect("tempdir");
    let a = folder_in(root.path(), "a");
    let b = folder_in(root.path(), "b");
    let c = folder_in(root.path(), "c");
    let (mut dom, mut state, zone) = rig();
    dom.in_runtime(|| zone.finish(&[a.clone(), b.clone(), c.clone()]));
    let _ = dom.render_immediate_to_vec();

    assert_eq!(overlays(&state), [Overlay::Detected(a.clone())]);
    assert_eq!(*zone.queue.peek(), [b.clone(), c.clone()]);
    assert_eq!(pins(&zone).len(), 3, "all three are pinned when dropped");

    // Closing the first brings the second.
    state.overlays.write().clear();
    let _ = dom.render_immediate_to_vec();
    assert_eq!(overlays(&state), [Overlay::Detected(b)]);
    assert_eq!(*zone.queue.peek(), [c]);
}

#[test]
fn drop_while_a_dialog_is_open_is_ignored() {
    let root = tempfile::tempdir().expect("tempdir");
    let app = folder_in(root.path(), "new-app");
    let (mut dom, mut state, zone) = rig();
    state.overlays.write().push(Overlay::Palette);
    dom.in_runtime(|| zone.finish(&[app]));
    let _ = dom.render_immediate_to_vec();

    assert_eq!(overlays(&state), [Overlay::Palette]);
    assert!(pins(&zone).is_empty());
}

#[test]
fn drop_the_open_folder_sheet_gives_way_to_the_dropped_folder() {
    let root = tempfile::tempdir().expect("tempdir");
    let app = folder_in(root.path(), "new-app");
    let (mut dom, mut state, zone) = rig();
    state.overlays.write().push(Overlay::OpenFolder);
    dom.in_runtime(|| zone.finish(std::slice::from_ref(&app)));
    let _ = dom.render_immediate_to_vec();
    assert_eq!(overlays(&state), [Overlay::Detected(app)]);
}

#[test]
fn drop_what_hovers_is_shown_and_ends_with_the_drop() {
    let root = tempfile::tempdir().expect("tempdir");
    let app = folder_in(root.path(), "new-app");
    let (dom, _state, zone) = rig();
    dom.in_runtime(|| {
        let mut hover = zone.hover();
        hover.set(Some(classify(std::slice::from_ref(&app))));
        zone.finish(std::slice::from_ref(&app));
        assert!(hover.peek().is_none(), "the overlay goes with the drop");
    });
}

#[test]
fn drop_leave_hides_the_overlay_only_when_every_enter_has_left() {
    let (dom, _state, zone) = rig();
    dom.in_runtime(|| {
        let mut hover = zone.hover();
        let mut depth = zone.depth;
        hover.set(Some(Dropped::default()));
        depth.set(2);
        zone.leave();
        assert!(hover.peek().is_some(), "still inside a child");
        zone.leave();
        assert!(hover.peek().is_none());
        zone.leave();
        assert_eq!(*depth.peek(), 0, "never below zero");
    });
}

#[test]
fn drop_the_queue_waits_while_a_detected_sheet_or_review_is_open() {
    let queue = [PathBuf::from("/a")];
    assert_eq!(next_in_queue(&[], &queue), Some(PathBuf::from("/a")));
    assert_eq!(
        next_in_queue(&[Overlay::Detected(PathBuf::from("/x"))], &queue),
        None
    );
    assert_eq!(
        next_in_queue(&[Overlay::ReviewChanges(uuid::Uuid::nil())], &queue),
        None
    );
    assert_eq!(next_in_queue(&[], &[]), None);
}

#[test]
fn drop_only_the_folder_flow_accepts_a_drop() {
    assert!(accepts(&[]));
    assert!(accepts(&[
        Overlay::OpenFolder,
        Overlay::Detected(PathBuf::from("/x"))
    ]));
    assert!(!accepts(&[Overlay::Palette]));
    assert!(!accepts(&[Overlay::ReviewChanges(uuid::Uuid::nil())]));
    assert!(!accepts(&[Overlay::OpenFolder, Overlay::CreateVolume]));
}

#[test]
fn drop_open_dropped_tells_the_engines_service_only_through_a_link_and_only_for_new_pins() {
    // With a store and no link, pins are written and nothing else happens.
    let store = berth_store::Store::open_in_memory().expect("store");
    let known = Known::from_projects(&[project_at(Path::new("/Users/me/analytics"))]);
    let opened = open_dropped(
        Some(&store),
        None,
        &[
            PathBuf::from("/Users/me/analytics"),
            PathBuf::from("/Users/me/new-app"),
        ],
        &engine(),
        &known,
        chrono::Utc::now(),
    );
    assert_eq!(
        opened.go_to,
        Some(ProjectId::for_folder(
            Path::new("/Users/me/analytics"),
            &engine()
        ))
    );
    assert_eq!(opened.fresh, [PathBuf::from("/Users/me/new-app")]);
    let pinned: Vec<PathBuf> = store
        .pins()
        .expect("pins")
        .into_iter()
        .map(|p| p.folder)
        .collect();
    assert_eq!(pinned, [PathBuf::from("/Users/me/new-app")]);
}

// --- The frames ---

#[test]
fn drop_folder_frame_says_what_dropping_does() {
    let html = render_scene_html("Projects / drop folder / 1440").expect("scene");
    for text in [
        "class=\"drop-overlay\"",
        "Drop to open new-app",
        "Dev/side/new-app",
        "Berth reads the folder and shows what it finds. Nothing is written yet.",
    ] {
        assert!(html.contains(text), "missing {text:?}");
    }
}

#[test]
fn drop_wrong_type_scene_warns_instead_of_inviting() {
    let html = render_scene_html("Projects / drop folder / wrong type / 1440").expect("scene");
    assert!(html.contains("drop-overlay drop-refuse"), "{html}");
    assert!(html.contains("Berth cannot open notes.txt"));
    assert!(html.contains("Drop a folder, a Compose file or a Dockerfile."));
    assert!(!html.contains("Drop to open"));
}

#[test]
fn drop_nothing_hovering_draws_no_overlay() {
    let html = render_scene_html("Projects / default / 1440").expect("scene");
    assert!(!html.contains("drop-overlay"));
}

// --- Drags that are not drops of files ---

fn drag_event(files: &[&Path], texts: &[(&str, &str)]) -> DragEvent {
    use dioxus::html::{
        DragData, SerializedDataTransfer, SerializedDataTransferItem, SerializedDragData,
        SerializedFileData, SerializedPointInteraction,
    };
    let data = SerializedDragData {
        mouse: SerializedPointInteraction::default(),
        data_transfer: SerializedDataTransfer {
            items: texts
                .iter()
                .map(|(kind, data)| SerializedDataTransferItem {
                    kind: "string".to_string(),
                    type_: (*kind).to_string(),
                    data: (*data).to_string(),
                })
                .collect(),
            files: files
                .iter()
                .map(|path| SerializedFileData {
                    path: path.to_path_buf(),
                    size: 0,
                    last_modified: 0,
                    content_type: None,
                    contents: None,
                })
                .collect(),
            effect_allowed: "all".to_string(),
            drop_effect: "none".to_string(),
        },
    };
    Event::new(std::rc::Rc::new(DragData::new(data)), true)
}

#[test]
fn drop_a_drag_of_files_shows_the_overlay_and_a_drag_of_text_with_the_last_paths_does_not() {
    let root = tempfile::tempdir().expect("tempdir");
    let app = folder_in(root.path(), "new-app");
    let (dom, _state, zone) = rig();

    // dioxus-desktop attaches the last drop's paths to an in-app drag of selected text.
    let text = drag_event(&[app.as_path()], &[("text/plain", "selected words")]);
    dom.in_runtime(|| {
        zone.enter(&text);
        zone.over(&text);
    });
    assert_eq!(dom.in_runtime(|| zone.hover().peek().clone()), None);

    let files = drag_event(&[app.as_path()], &[]);
    dom.in_runtime(|| zone.enter(&files));
    let shown = dom.in_runtime(|| zone.hover().peek().clone());
    assert_eq!(shown.map(|d| d.folders), Some(vec![app]));
}

#[test]
fn drop_a_drag_without_paths_shows_nothing() {
    let (dom, _state, zone) = rig();
    let nothing = drag_event(&[], &[]);
    dom.in_runtime(|| {
        zone.enter(&nothing);
        zone.over(&nothing);
    });
    assert_eq!(dom.in_runtime(|| zone.hover().peek().clone()), None);
}

#[test]
fn drop_a_drop_of_text_with_the_last_paths_opens_nothing_and_clears_the_overlay() {
    let root = tempfile::tempdir().expect("tempdir");
    let app = folder_in(root.path(), "new-app");
    let (mut dom, state, zone) = rig();
    dom.in_runtime(|| zone.enter(&drag_event(&[app.as_path()], &[])));
    assert!(dom.in_runtime(|| zone.hover().peek().is_some()));

    let text = drag_event(&[app.as_path()], &[("text/plain", "selected words")]);
    dom.in_runtime(|| zone.dropped(&text));
    let _ = dom.render_immediate_to_vec();

    assert_eq!(
        dom.in_runtime(|| zone.hover().peek().clone()),
        None,
        "the overlay goes"
    );
    assert!(overlays(&state).is_empty(), "{:?}", overlays(&state));
    assert!(pins(&zone).is_empty());
    assert!(
        state.toasts.peek().is_empty(),
        "no wrong-type message either"
    );
}

#[test]
fn drop_a_leave_clears_the_overlay_and_a_text_file_still_counts_as_files() {
    let root = tempfile::tempdir().expect("tempdir");
    let app = folder_in(root.path(), "new-app");
    let (dom, _state, zone) = rig();
    dom.in_runtime(|| zone.enter(&drag_event(&[app.as_path()], &[])));
    dom.in_runtime(|| zone.leave());
    assert_eq!(dom.in_runtime(|| zone.hover().peek().clone()), None);

    // WebKit lists a dragged notes.txt as a text/plain item whose data is its name.
    let notes = root.path().join("notes.txt");
    let file = drag_event(&[notes.as_path()], &[("text/plain", "notes.txt")]);
    dom.in_runtime(|| zone.enter(&file));
    assert!(dom.in_runtime(|| zone.hover().peek().is_some()));
}

#[test]
fn drop_file_drag_judges_the_paths_and_the_text_a_drag_carries() {
    let folder = PathBuf::from("/work/new-app");
    let one = std::slice::from_ref(&folder);
    assert_eq!(file_drag(&[], &[]), None);
    assert_eq!(file_drag(one, &[]), Some(vec![folder.clone()]));
    assert_eq!(
        file_drag(one, &["file:///work/new-app".to_string()]),
        Some(vec![folder.clone()])
    );
    assert_eq!(
        file_drag(one, &["new-app".to_string(), String::new()]),
        Some(vec![folder.clone()])
    );
    assert_eq!(file_drag(one, &["https://example.com".to_string()]), None);
}
