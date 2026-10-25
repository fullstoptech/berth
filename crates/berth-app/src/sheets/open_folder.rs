//! The Open folder sheet (P5-T4, PLAN §6.18, D36): Berth's own folder browser.
//! Places and Recent on the left; a path bar, a filter, the hidden files
//! switch and Finder-style columns in the middle; what Berth finds in the
//! highlighted folder on the right; "Use system dialog…", Cancel and the
//! primary action in the footer.
//!
//! Berth reads only what is on screen: the columns of the chain down to the
//! folder being looked at (one level each), the named files of the
//! highlighted folder for the preview, and nothing in the background. The
//! text inputs are uncontrolled (D59): the path bar remounts, keyed by a
//! revision, when the columns move it; Tab sets the typed text through the
//! DOM so focus stays.
//!
//! Opening a folder records it (`services::folders::record_open`): Recent,
//! and a pin unless Berth already has a project there ("Go to <project>").
//! P5-T5 turns the new pin into the Detected sheet.

use std::path::{Path, PathBuf};

use dioxus::html::input_data::keyboard_types::Key;
use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use crate::services::folders::{
    self, classify_path, complete_path, display_path, folder_name, found_rows, resolve_input,
    service_label, stack_label, Browser, FolderError, Known, OpenOutcome, PathState, Place,
    PlaceKind, Preview,
};
use crate::services::Services;
use crate::state::{AppState, Overlay, Section, Selection};
use crate::ui::column_browser::{ColumnBrowser, ROW_HEIGHT_BASE_PX};
use crate::ui::{
    Button, ButtonVariant, CheckState, Checkbox, ControlSize, Icon, IconButton, IconSize, TextField,
};

/// The sheet's CSS.
const SHEET_CSS: &str = include_str!("../../assets/css/open_folder.css");

/// The viewport of a column: the sheet's body minus the toolbar.
const COLUMN_VIEWPORT_PX: u32 = 520;

/// A folder that has not answered after this long is "slow".
const SLOW_AFTER: std::time::Duration = std::time::Duration::from_millis(1500);

/// What the right-hand pane shows.
#[derive(Debug, Clone, PartialEq)]
pub enum PreviewState {
    /// Nothing is highlighted.
    Empty,
    Loading(PathBuf),
    Ready(Preview),
    Failed(FolderError),
}

/// The sheet's own state for the folder shortcuts, kept above the sheet so
/// ⌘⇧. (keys.rs) reaches it: whether hidden folders are shown.
#[derive(Clone, Copy)]
pub struct FolderView {
    pub show_hidden: Signal<bool>,
}

/// Provide the [`FolderView`] once from the App root (or a scene root).
pub fn provide_folder_view() -> FolderView {
    use_hook(|| {
        provide_context(FolderView {
            show_hidden: Signal::new(false),
        })
    })
}

/// ⌘⇧.: show or hide hidden folders in the browser. A no-op without a
/// provided [`FolderView`].
pub fn toggle_hidden(_state: &mut AppState, _action: crate::actions::ActionId) {
    if let Some(view) = try_consume_context::<FolderView>() {
        let mut hidden = view.show_hidden;
        let next = !*hidden.peek();
        hidden.set(next);
    }
}

/// Scenes put the sheet into a frame's state before it is drawn: the model
/// stands in for what the disk would answer, and no read runs.
#[derive(Clone)]
pub struct DevOpenFolder {
    pub home: PathBuf,
    pub browser: Browser,
    pub preview: PreviewState,
    pub favourites: Vec<PathBuf>,
    pub volumes: Vec<Place>,
    pub recent: Vec<Place>,
}

fn close_sheet(state: &mut AppState) {
    state.overlays.write().retain(|o| *o != Overlay::OpenFolder);
}

/// The place that holds `folder`: the longest place path that is a prefix.
#[must_use]
pub fn active_place(places: &[Place], folder: &Path) -> Option<usize> {
    places
        .iter()
        .enumerate()
        .filter(|(_, p)| folder.starts_with(&p.path))
        .max_by_key(|(_, p)| p.path.components().count())
        .map(|(i, _)| i)
}

/// The primary button: what it says and why it is off.
#[must_use]
pub fn primary_label(target: Option<&Path>, known: &Known) -> (String, Option<String>) {
    match target {
        Some(path) => match known.get(path) {
            Some(project) => (format!("Go to {}", project.name), None),
            None => (format!("Open {}", folder_name(path)), None),
        },
        None => (
            "Open folder".to_string(),
            Some("Berth can't read this folder".to_string()),
        ),
    }
}

/// Reads one column and then annotates its children, writing the model as
/// each part arrives. Does nothing once `token` is cancelled.
fn start_load(
    mut browser: Signal<Browser>,
    home: PathBuf,
    folder: PathBuf,
    token: CancellationToken,
) {
    let slow_token = token.clone();
    let slow_folder = folder.clone();
    spawn(async move {
        tokio::select! {
            () = slow_token.cancelled() => {}
            () = tokio::time::sleep(SLOW_AFTER) => browser.write().mark_slow(&slow_folder),
        }
    });
    spawn(async move {
        let result = folders::list(folder.clone()).await;
        if token.is_cancelled() {
            return;
        }
        let mut paths: Vec<PathBuf> = Vec::new();
        if let Ok(entries) = &result {
            // Visible folders first, so the first screen fills first.
            paths.extend(entries.iter().filter(|e| !e.hidden).map(|e| e.path.clone()));
            paths.extend(entries.iter().filter(|e| e.hidden).map(|e| e.path.clone()));
        }
        browser.write().set_listing(&folder, result);
        // The read is done: the slow note is over.
        let annotate_folder = folder.clone();
        folders::annotate_all(paths, home, token.clone(), move |chunk| {
            browser.write().set_details(&annotate_folder, chunk);
        })
        .await;
        token.cancel();
    });
}

#[component]
pub fn OpenFolderSheet() -> Element {
    let mut state = use_context::<AppState>();
    let services = try_consume_context::<Services>();
    let view = try_consume_context::<FolderView>();
    let dev = try_consume_context::<DevOpenFolder>();

    let home = use_hook({
        let dev = dev.clone();
        move || {
            dev.as_ref()
                .map_or_else(berth_sys::paths::home, |d| d.home.clone())
        }
    });
    let sheet_token = use_hook(CancellationToken::new);
    {
        let token = sheet_token.clone();
        use_drop(move || token.cancel());
    }

    // Recent and the mounted volumes: read once, off the UI thread.
    let mut recent = use_signal(|| dev.as_ref().map(|d| d.recent.clone()).unwrap_or_default());
    let mut volumes = use_signal(|| dev.as_ref().map(|d| d.volumes.clone()).unwrap_or_default());
    let mut first_folder = use_signal(|| None::<PathBuf>);
    let dev_for_places = dev.clone();
    let store = services.as_ref().map(|s| s.store.clone());
    use_hook({
        let store = store.clone();
        move || {
            if dev_for_places.is_some() {
                return;
            }
            spawn(async move {
                let loaded = tokio::task::spawn_blocking(move || {
                    let recent = store
                        .as_ref()
                        .map(|s| folders::recent_places(s, folders::RECENT_SHOWN))
                        .unwrap_or_default();
                    (recent, folders::mounted_volumes(Path::new("/Volumes")))
                })
                .await;
                if let Ok((found_recent, found_volumes)) = loaded {
                    first_folder.set(
                        found_recent
                            .first()
                            .and_then(|p| p.path.parent())
                            .map(Path::to_path_buf),
                    );
                    recent.set(found_recent);
                    volumes.set(found_volumes);
                }
            });
        }
    });

    let favourites: Vec<PathBuf> = match &dev {
        Some(d) => d.favourites.clone(),
        None => state.settings.read().favourites.clone(),
    };
    let places = folders::places(&home, &favourites, volumes.read().clone());

    // The model, the preview and the revisions that remount the inputs.
    let browser = use_signal({
        let dev = dev.clone();
        let home = home.clone();
        let start = favourites.first().cloned().unwrap_or_else(|| home.clone());
        move || {
            dev.as_ref()
                .map_or_else(|| Browser::at(&start, &home), |d| d.browser.clone())
        }
    });
    let mut preview = use_signal(|| {
        dev.as_ref()
            .map_or(PreviewState::Empty, |d| d.preview.clone())
    });
    let path_rev = use_signal(|| 0_u32);
    let filter_rev = use_signal(|| 0_u32);
    let typed = use_signal(|| None::<String>);
    let path_state = use_signal(|| PathState::Folder);
    // The columns moved: the path bar shows the new folder, the filter starts empty.
    let bump_path = move || {
        let (mut rev, mut typed, mut state) = (path_rev, typed, path_state);
        rev += 1;
        typed.set(None);
        state.set(PathState::Folder);
    };
    let bump_filter = move || {
        let mut rev = filter_rev;
        rev += 1;
    };
    let tokens = use_signal(Vec::<(PathBuf, CancellationToken)>::new);

    let known = use_memo(move || Known::from_projects(&state.projects.read()));
    let target = use_memo(move || browser.read().target());
    let preview_target = use_memo(move || browser.read().preview_target());
    let current = use_memo(move || browser.read().current_folder().map(Path::to_path_buf));

    // Hidden files follow the shortcut's signal.
    let dev_active = dev.is_some();
    use_effect(move || {
        if let Some(view) = view {
            let on = *view.show_hidden.read();
            let mut browser = browser;
            if browser.peek().show_hidden != on {
                browser.write().show_hidden = on;
            }
        }
    });

    // Loads: the first chain once, then whatever `go_to` and `descend` add.
    let load_home = home.clone();
    let load_sheet = sheet_token.clone();
    let load = move |folder: PathBuf| {
        if dev_active {
            return;
        }
        let token = load_sheet.child_token();
        let mut tokens = tokens;
        tokens.write().push((folder.clone(), token.clone()));
        start_load(browser, load_home.clone(), folder, token);
    };
    // Reads for columns that no longer exist stop.
    let prune = move || {
        let mut tokens = tokens;
        let alive: Vec<PathBuf> = browser
            .peek()
            .columns
            .iter()
            .map(|c| c.folder.clone())
            .collect();
        tokens.write().retain(|(folder, token)| {
            let keep = alive.contains(folder);
            if !keep {
                token.cancel();
            }
            keep
        });
    };
    let start_columns: Vec<PathBuf> = browser
        .peek()
        .columns
        .iter()
        .map(|c| c.folder.clone())
        .collect();
    {
        let load = load.clone();
        use_hook(move || {
            if !dev_active {
                for folder in start_columns {
                    load(folder);
                }
            }
        });
    }
    // The first folder follows the last one opened, once Recent has loaded.
    {
        let load = load.clone();
        let home = home.clone();
        use_effect(move || {
            let Some(start) = first_folder() else { return };
            let mut browser = browser;
            let at_default = browser.peek().columns.len() <= 3
                && browser
                    .peek()
                    .columns
                    .last()
                    .is_some_and(|c| c.selected.is_none());
            if !at_default {
                return;
            }
            let read = browser.write().open_path(&start, &home);
            for folder in read {
                load(folder);
            }
            prune();
            bump_filter();
            bump_path();
        });
    }

    // The preview follows the highlighted folder.
    let mut generation = use_signal(|| 0_u64);
    use_effect(move || {
        let wanted = preview_target();
        if dev_active {
            return;
        }
        let next = generation.peek().wrapping_add(1);
        generation.set(next);
        let Some(folder) = wanted else {
            preview.set(PreviewState::Empty);
            return;
        };
        preview.set(PreviewState::Loading(folder.clone()));
        spawn(async move {
            let result = folders::preview(folder).await;
            if *generation.peek() != next {
                return;
            }
            preview.set(match result {
                Ok(done) => PreviewState::Ready(done),
                Err(err) => PreviewState::Failed(err),
            });
        });
    });

    // Moves the columns to `path` (a place, a typed path, a chosen folder).
    let go_to = {
        let home = home.clone();
        let load = load.clone();
        move |path: PathBuf| {
            let mut browser = browser;
            let read = browser.write().open_path(&path, &home);
            for folder in read {
                load(folder);
            }
            prune();
            bump_filter();
            bump_path();
        }
    };

    let descend = {
        let load = load.clone();
        move |column: usize, path: PathBuf| {
            let mut browser = browser;
            let read = browser.write().descend(column, &path);
            if let Some(folder) = read {
                load(folder);
            }
            bump_filter();
            bump_path();
        }
    };
    let select = move |column: usize, path: PathBuf| {
        let mut browser = browser;
        browser.write().select(column, &path);
        prune();
        bump_path();
    };

    // Footer.
    let known_now = known.read().clone();
    let target_now = target();
    let (label, blocker) = primary_label(target_now.as_deref(), &known_now);
    let open_store = store.clone();
    let open_engine_state = state;
    let do_open = move || {
        let Some(path) = target.peek().clone() else {
            return;
        };
        let known = known.peek().clone();
        let engine = {
            let scope = match &*open_engine_state.engine_scope.peek() {
                crate::state::EngineScope::One(id) => Some(id.clone()),
                crate::state::EngineScope::All => None,
            };
            let default = open_engine_state.settings.peek().default_engine.clone();
            let running: Vec<berth_core::EngineId> = open_engine_state
                .engines
                .peek()
                .iter()
                .filter(|e| e.state == berth_core::EngineState::Running)
                .map(|e| e.id.clone())
                .collect();
            folders::pick_engine(scope.as_ref(), default.as_ref(), &running)
        };
        let outcome = match &open_store {
            Some(store) => folders::record_open(store, &path, &engine, &known, chrono::Utc::now()),
            None => Ok(known
                .get(&path)
                .map_or(OpenOutcome::Pinned, |p| OpenOutcome::GoTo(p.id.clone()))),
        };
        let mut state = state;
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(err) => {
                tracing::warn!(%err, "recording the opened folder failed");
                OpenOutcome::Pinned
            }
        };
        close_sheet(&mut state);
        match outcome {
            OpenOutcome::GoTo(id) => {
                state.section.set(Section::Projects);
                state.selection.set(Selection::Project(id));
            }
            OpenOutcome::Pinned => state.overlays.write().push(Overlay::Detected(path)),
        }
    };

    // System dialog: the native panel, started in the folder being looked at.
    let pick_with_dialog = {
        let go_to = go_to.clone();
        move |start: Option<PathBuf>| {
            let go_to = go_to.clone();
            spawn(async move {
                let mut dialog = rfd::AsyncFileDialog::new().set_title("Open folder");
                if let Some(start) = start {
                    dialog = dialog.set_directory(start);
                }
                if let Some(picked) = dialog.pick_folder().await {
                    go_to(picked.path().to_path_buf());
                }
            });
        }
    };

    // Path bar.
    let path_initial = {
        let b = browser.read();
        match (b.current_folder(), b.failure()) {
            (Some(folder), None) => {
                format!("{}/", display_path(folder, &home).trim_end_matches('/'))
            }
            (Some(folder), Some(_)) => display_path(folder, &home),
            _ => String::new(),
        }
    };
    let path_error = match path_state() {
        PathState::Folder => None,
        PathState::Missing => Some("Path not found".to_string()),
        PathState::NotAFolder => Some("That is a file, not a folder".to_string()),
        PathState::Denied => Some("Berth can't read that folder".to_string()),
    };
    let bar_home = home.clone();
    let bar_current = current();
    let on_path_input = move |e: FormEvent| {
        let (mut typed, mut path_state) = (typed, path_state);
        let text = e.value();
        typed.set(Some(text.clone()));
        let base = bar_current.clone().unwrap_or_else(|| bar_home.clone());
        let path = resolve_input(&text, &base, &bar_home);
        if text.trim().is_empty() {
            path_state.set(PathState::Folder);
            return;
        }
        spawn(async move {
            let state_of = tokio::task::spawn_blocking(move || classify_path(&path))
                .await
                .unwrap_or(PathState::Missing);
            // Only the answer to what is typed now counts.
            if typed.peek().as_deref() == Some(text.as_str()) {
                path_state.set(state_of);
            }
        });
    };
    let tab_initial = path_initial.clone();
    let key_home = home.clone();
    let key_current = current();
    let key_go = go_to.clone();
    let on_path_key = move |e: KeyboardEvent| {
        let (mut typed, mut path_state) = (typed, path_state);
        match e.key() {
            Key::Enter => {
                e.stop_propagation();
                let Some(text) = typed.peek().clone() else {
                    return;
                };
                if text.trim().is_empty() {
                    return;
                }
                let base = key_current.clone().unwrap_or_else(|| key_home.clone());
                let path = resolve_input(&text, &base, &key_home);
                let go = key_go.clone();
                spawn(async move {
                    let probe = path.clone();
                    let found = tokio::task::spawn_blocking(move || classify_path(&probe))
                        .await
                        .unwrap_or(PathState::Missing);
                    if found == PathState::Folder {
                        go(path);
                    } else {
                        path_state.set(found);
                    }
                });
            }
            Key::Tab => {
                e.prevent_default();
                e.stop_propagation();
                let text = typed.peek().clone().unwrap_or_else(|| tab_initial.clone());
                let base = key_current.clone().unwrap_or_else(|| key_home.clone());
                let hidden = browser.peek().show_hidden;
                let home = key_home.clone();
                spawn(async move {
                    let done = tokio::task::spawn_blocking(move || {
                        complete_path(&text, &base, &home, hidden)
                    })
                    .await
                    .ok()
                    .flatten();
                    let Some(done) = done else { return };
                    typed.set(Some(done.clone()));
                    let quoted = done.replace('\\', "\\\\").replace('\'', "\\'");
                    let set = document::eval(&format!(
                        "var e = document.getElementById('of-path'); if (e) {{ e.value = '{quoted}'; e.focus(); e.setSelectionRange(e.value.length, e.value.length); e.scrollLeft = e.scrollWidth; }}"
                    ))
                    .await;
                    if let Err(err) = set {
                        tracing::warn!(%err, "setting the path bar text failed");
                    }
                    path_state.set(PathState::Folder);
                });
            }
            Key::Escape => {}
            _ => e.stop_propagation(),
        }
    };

    // Sheet keys: arrows move through the last column, Enter opens.
    let key_open = do_open.clone();
    let key_descend = descend.clone();
    let key_home2 = home.clone();
    let key_load = load.clone();
    let on_sheet_key = move |e: KeyboardEvent| {
        use dioxus::html::ModifiersInteraction;
        let mods = e.data.modifiers();
        if mods.contains(dioxus::html::Modifiers::META)
            || mods.contains(dioxus::html::Modifiers::CONTROL)
        {
            return;
        }
        let mut browser = browser;
        match e.key() {
            Key::ArrowDown => {
                e.prevent_default();
                e.stop_propagation();
                if browser.write().step(1).is_some() {
                    bump_path();
                }
            }
            Key::ArrowUp => {
                e.prevent_default();
                e.stop_propagation();
                if browser.write().step(-1).is_some() {
                    bump_path();
                }
            }
            Key::ArrowRight => {
                e.prevent_default();
                e.stop_propagation();
                let (column, selected) = {
                    let b = browser.peek();
                    (b.columns.len().saturating_sub(1), b.preview_target())
                };
                if let Some(path) = selected {
                    key_descend(column, path);
                }
            }
            Key::ArrowLeft => {
                e.prevent_default();
                e.stop_propagation();
                let read = browser.write().up(&key_home2);
                for folder in read {
                    key_load(folder);
                }
                prune();
                bump_filter();
                bump_path();
            }
            Key::Enter => {
                e.stop_propagation();
                key_open();
            }
            _ => {}
        }
    };

    let toggle_hidden_box = move |on: bool| {
        if let Some(view) = view {
            let mut hidden = view.show_hidden;
            hidden.set(on);
        } else {
            let mut browser = browser;
            browser.write().show_hidden = on;
        }
    };

    let active = current().and_then(|folder| active_place(&places, &folder));
    let recent_list = recent.read().clone();
    let filter_initial = String::new();
    let show_hidden = browser.read().show_hidden;
    let cols_go = go_to.clone();
    let cols_other = go_to.clone();
    let cols_home = home.clone();
    let cols_dialog = pick_with_dialog.clone();
    let failure_start = current();
    let footer_dialog = pick_with_dialog;
    let footer_open = do_open.clone();

    rsx! {
        style { "{SHEET_CSS}" }
        div {
            class: "of-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Open folder",
            tabindex: "0",
            onmounted: move |e| {
                spawn(async move {
                    let _ = e.data().set_focus(true).await;
                });
            },
            onclick: move |e| e.stop_propagation(),
            onkeydown: on_sheet_key,
            header { class: "of-header",
                h2 { class: "of-title", "Open folder" }
                IconButton {
                    icon: "x",
                    label: "Close".to_string(),
                    size: ControlSize::Sm,
                    onclick: move |_| close_sheet(&mut state),
                }
            }
            div { class: "of-body",
                nav { class: "of-places", "aria-label": "Places",
                    div { class: "of-places-label", "Places" }
                    for (i, place) in places.iter().enumerate() {
                        {
                            let selected = active == Some(i);
                            let favourite = place.kind == PlaceKind::Favourite;
                            let path = place.path.clone();
                            let star_path = place.path.clone();
                            let go = cols_go.clone();
                            let row_class = if selected { "of-place of-place-selected" } else { "of-place" };
                            let icon = place.icon();
                            rsx! {
                                div { key: "{place.path.display()}", class: row_class,
                                    button {
                                        class: "of-place-button",
                                        r#type: "button",
                                        "aria-current": if selected { "true" } else { "false" },
                                        onclick: move |_| go(path.clone()),
                                        span { class: "of-place-icon", Icon { name: icon, size: IconSize::Sm } }
                                        span { class: "of-place-name", "{place.name}" }
                                    }
                                    if matches!(place.kind, PlaceKind::Favourite | PlaceKind::Standard) {
                                        button {
                                            class: if favourite { "of-star of-star-on" } else { "of-star" },
                                            r#type: "button",
                                            title: if favourite { "Remove from favourites" } else { "Add to favourites" },
                                            "aria-label": if favourite { "Remove from favourites" } else { "Add to favourites" },
                                            "aria-pressed": "{favourite}",
                                            onclick: move |_| {
                                                let mut settings = state.settings.write();
                                                if let Some(at) = settings.favourites.iter().position(|f| *f == star_path) {
                                                    settings.favourites.remove(at);
                                                } else {
                                                    settings.favourites.push(star_path.clone());
                                                }
                                            },
                                            Icon { name: "star", size: IconSize::Sm }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !recent_list.is_empty() {
                        div { class: "of-places-label of-recent-label", "Recent" }
                        for place in recent_list.iter() {
                            {
                                let path = place.path.clone();
                                let go = cols_other.clone();
                                rsx! {
                                    div { key: "{place.path.display()}", class: "of-place",
                                        button {
                                            class: "of-place-button",
                                            r#type: "button",
                                            title: "{display_path(&place.path, &cols_home)}",
                                            onclick: move |_| go(path.clone()),
                                            span { class: "of-place-icon", Icon { name: "folder", size: IconSize::Sm } }
                                            span { class: "of-place-name", "{place.name}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                section { class: "of-main",
                    div { class: "of-toolbar",
                        div { class: if path_error.is_some() { "of-pathbar of-pathbar-error" } else { "of-pathbar" },
                            span { class: "of-pathbar-icon", Icon { name: "folder", size: IconSize::Sm } }
                            for rev in [path_rev()] {
                                input {
                                    key: "{rev}",
                                    id: "of-path",
                                    class: "of-path-input",
                                    r#type: "text",
                                    initial_value: "{path_initial}",
                                    spellcheck: "false",
                                    autocomplete: "off",
                                    "aria-label": "Folder path",
                                    "aria-invalid": "{path_error.is_some()}",
                                    placeholder: "~/Dev/app",
                                    oninput: {
                                        let handler = on_path_input.clone();
                                        move |e| handler(e)
                                    },
                                    onkeydown: {
                                        let handler = on_path_key.clone();
                                        move |e| handler(e)
                                    },
                                }
                            }
                        }
                        div { class: "of-filter",
                            for rev in [filter_rev()] {
                                TextField {
                                    key: "{rev}",
                                    initial_value: filter_initial.clone(),
                                    placeholder: "Filter this folder".to_string(),
                                    prefix_icon: Some("search"),
                                    aria_label: "Filter this folder".to_string(),
                                    oninput: move |e: FormEvent| {
                                        let mut browser = browser;
                                        browser.write().filter = e.value();
                                    },
                                }
                            }
                        }
                        Checkbox {
                            state: CheckState::from_bool(show_hidden),
                            label: "Hidden files".to_string(),
                            aria_label: "Show hidden files".to_string(),
                            on_toggle: toggle_hidden_box,
                        }
                    }
                    if let Some(message) = &path_error {
                        p { class: "of-path-error", role: "alert", "{message}" }
                    }
                    div { class: "of-columns",
                        ColumnBrowser {
                            browser,
                            known,
                            row_height_px: crate::text_scale::scaled_px(
                                ROW_HEIGHT_BASE_PX,
                                state.settings.read().text_scale,
                            ),
                            viewport_height_px: COLUMN_VIEWPORT_PX,
                            on_select: move |(column, path): (usize, PathBuf)| select(column, path),
                            on_open: move |(column, path): (usize, PathBuf)| descend(column, path),
                            on_open_settings: move |()| folders::open_privacy_settings(),
                            on_system_dialog: move |folder: PathBuf| cols_dialog(Some(folder)),
                            on_other: move |()| {
                                go_to_home(&cols_other, &cols_home);
                            },
                        }
                    }
                }
                aside { class: "of-preview", "aria-label": "Preview",
                    PreviewPane { state: preview(), known: known_now.clone(), home: home.clone() }
                }
            }
            footer { class: "of-footer",
                Button {
                    label: "Use system dialog\u{2026}".to_string(),
                    variant: ButtonVariant::Ghost,
                    size: ControlSize::Md,
                    onclick: move |_| footer_dialog(failure_start.clone()),
                }
                span { class: "of-spacer" }
                Button {
                    label: "Cancel".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: move |_| close_sheet(&mut state),
                }
                Button {
                    label: label,
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    disabled: blocker.is_some(),
                    disabled_reason: blocker,
                    onclick: move |_| footer_open(),
                }
            }
        }
    }
}

/// "Choose another folder": back to the home.
fn go_to_home(go: &impl Fn(PathBuf), home: &Path) {
    go(home.to_path_buf());
}

/// The right-hand pane: what Berth finds in the highlighted folder.
#[component]
fn PreviewPane(state: PreviewState, known: Known, home: PathBuf) -> Element {
    match state {
        PreviewState::Empty => rsx! {
            p { class: "of-preview-empty", "Select a folder to see what Berth finds" }
        },
        PreviewState::Loading(folder) => rsx! {
            div { class: "of-preview-body", "aria-busy": "true",
                h3 { class: "of-preview-name",
                    Icon { name: "folder", size: IconSize::Md }
                    span { "{folder_name(&folder)}" }
                }
                p { class: "of-preview-path", "{display_path(&folder, &home)}" }
                p { class: "of-preview-note", "Reading\u{2026}" }
            }
        },
        PreviewState::Failed(err) => rsx! {
            div { class: "of-preview-body",
                h3 { class: "of-preview-name",
                    Icon { name: "folder", size: IconSize::Md }
                    span { "{err.folder_name()}" }
                }
                p { class: "of-preview-path", "{display_path(err.folder(), &home)}" }
                p { class: "of-preview-note", "{err.headline()}" }
            }
        },
        PreviewState::Ready(preview) => {
            let detection = &preview.detection;
            let found: Vec<(&'static str, &'static str, bool, String)> = found_rows(detection)
                .into_iter()
                .map(|row| {
                    let text = row
                        .detail
                        .clone()
                        .unwrap_or_else(|| "not found".to_string());
                    let yes = row.detail.is_some();
                    (row.file, if yes { "circle-check" } else { "x" }, yes, text)
                })
                .collect();
            let stack: Vec<String> = detection.stack.iter().map(stack_label).collect();
            let services: Vec<(String, String)> = detection
                .services_needed
                .iter()
                .map(|g| (service_label(g.kind).to_string(), format!("({})", g.reason)))
                .collect();
            let name = folder_name(&preview.folder);
            let path_text = display_path(&preview.folder, &home);
            let items_text = crate::ui::file_browser::item_count(preview.items);
            let branch = detection.git_branch.clone();
            let known_as = known.get(&preview.folder).map(|k| k.name.clone());
            rsx! {
                div { class: "of-preview-body",
                    h3 { class: "of-preview-name",
                        Icon { name: "folder", size: IconSize::Md }
                        span { "{name}" }
                    }
                    p { class: "of-preview-path", "{path_text}" }
                    p { class: "of-preview-facts",
                        if let Some(branch) = branch {
                            span { class: "of-fact",
                                Icon { name: "git-branch", size: IconSize::Sm }
                                span { class: "of-branch", "{branch}" }
                            }
                            span { class: "of-dot", "\u{b7}" }
                        }
                        span { "{items_text}" }
                    }
                    if let Some(project) = known_as {
                        p { class: "of-preview-known", "In Berth as {project}" }
                    }
                    h4 { class: "of-preview-label", "Found" }
                    ul { class: "of-found",
                        for (file, icon, yes, text) in found {
                            li {
                                key: "{file}",
                                class: if yes { "of-found-row of-found-yes" } else { "of-found-row of-found-no" },
                                span { class: "of-found-icon",
                                    Icon { name: icon, size: IconSize::Sm }
                                }
                                code { class: "of-found-file", "{file}" }
                                span { class: "of-found-detail", "{text}" }
                            }
                        }
                    }
                    if !stack.is_empty() {
                        h4 { class: "of-preview-label", "Stack" }
                        for line in stack {
                            p { key: "{line}", class: "of-stack", "{line}" }
                        }
                    }
                    if !services.is_empty() {
                        h4 { class: "of-preview-label", "Services needed" }
                        ul { class: "of-services",
                            for (service, reason) in services {
                                li { key: "{service}-{reason}", class: "of-service",
                                    Icon { name: "box", size: IconSize::Sm }
                                    span { class: "of-service-name", "{service}" }
                                    span { class: "of-service-reason", "{reason}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "open_folder_tests.rs"]
mod tests;
