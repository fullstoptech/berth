//! Dropping folders on the window (P5-T5, D37).
//!
//! wry's drag-drop handler, installed by dioxus-desktop, gives the DOM drag events the real paths
//! (`DragData::files` of a drag event carries `NativeFileHover`'s paths), so the shell root listens
//! for `dragenter`, `dragover`, `dragleave` and `drop`. While files hover, [`DropOverlay`] shows
//! what dropping them does; a drop opens each folder (Compose files and Dockerfiles stand for the
//! folder they are in): a folder Berth already has jumps to its project, any other is pinned,
//! listed under Recent and shown in the Detected sheet, one at a time. Nothing is written.

use std::path::PathBuf;
use std::sync::Arc;

use dioxus::html::HasFileData;
use dioxus::prelude::*;

use berth_core::{EngineId, ProjectId};

use crate::services::drops::{classify, hover_text, Dropped};
use crate::services::engines::EngineLink;
use crate::services::folders::{self, Known, OpenOutcome};
use crate::services::Services;
use crate::state::{AppState, Overlay, Section, Selection, Toast, ToastTone};
use crate::ui::{Icon, IconSize};

/// The overlay's CSS, rendered with it (tokens only).
pub const DROP_CSS: &str = include_str!("../../assets/css/drop_overlay.css");

/// Folders dropped and not shown yet: the Detected sheet takes the first whenever none is open.
#[derive(Clone, Copy)]
pub struct DropQueue(pub Signal<Vec<PathBuf>>);

/// A scene or test starts the window with these files hovering.
#[derive(Debug, Clone, PartialEq)]
pub struct SeedHover(pub Dropped);

/// What opening the dropped folders did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Opened {
    /// The first folder Berth already has a project for.
    pub go_to: Option<ProjectId>,
    /// Folders now pinned that have no project yet, in drop order.
    pub fresh: Vec<PathBuf>,
}

/// Record each folder as opened (`record_open`: Recent, and a pin unless Berth has a project
/// there) and tell the engines service about new pins (the link's pins-changed call), once.
#[must_use]
pub fn open_dropped(
    store: Option<&berth_store::Store>,
    link: Option<&EngineLink>,
    folders: &[PathBuf],
    engine: &EngineId,
    known: &Known,
    now: chrono::DateTime<chrono::Utc>,
) -> Opened {
    let mut opened = Opened::default();
    for folder in folders {
        let outcome = match store {
            Some(store) => folders::record_open(store, folder, engine, known, now),
            None => Ok(known
                .get(folder)
                .map_or(OpenOutcome::Pinned, |p| OpenOutcome::GoTo(p.id.clone()))),
        };
        let outcome = outcome.unwrap_or_else(|err| {
            tracing::warn!(%err, "recording the dropped folder failed");
            OpenOutcome::Pinned
        });
        match outcome {
            OpenOutcome::GoTo(id) => {
                if opened.go_to.is_none() {
                    opened.go_to = Some(id);
                }
            }
            OpenOutcome::Pinned => opened.fresh.push(folder.clone()),
        }
    }
    if !opened.fresh.is_empty() {
        if let Some(link) = link {
            link.pins_changed();
        }
    }
    opened
}

/// Whether a drop is welcome now: only the sheets of the folder flow are open (or none). A
/// confirm dialog, Review changes or the palette keep the window to themselves.
#[must_use]
pub fn accepts(overlays: &[Overlay]) -> bool {
    overlays.iter().all(in_folder_flow)
}

/// The sheets that lead to opening a folder.
fn in_folder_flow(overlay: &Overlay) -> bool {
    matches!(
        overlay,
        Overlay::OpenFolder
            | Overlay::Detected(_)
            | Overlay::DropFolder(_)
            | Overlay::NewProject
            | Overlay::Generate(_)
    )
}

/// The folder the Detected sheet shows next: the head of the queue, when no Detected sheet or
/// Review changes is open.
#[must_use]
pub fn next_in_queue(overlays: &[Overlay], queue: &[PathBuf]) -> Option<PathBuf> {
    let busy = overlays
        .iter()
        .any(|o| matches!(o, Overlay::Detected(_) | Overlay::ReviewChanges(_)));
    (!busy).then(|| queue.first().cloned()).flatten()
}

/// Put `opened` on screen: fresh folders join the queue (the Open folder sheet gives way), else
/// the first known folder's project opens.
pub fn apply(state: &mut AppState, queue: &mut Signal<Vec<PathBuf>>, opened: Opened) {
    if !opened.fresh.is_empty() {
        state
            .overlays
            .write()
            .retain(|o| !matches!(o, Overlay::OpenFolder | Overlay::NewProject));
        let shown: Vec<PathBuf> = state
            .overlays
            .peek()
            .iter()
            .filter_map(|o| match o {
                Overlay::Detected(f) => Some(f.clone()),
                _ => None,
            })
            .collect();
        let mut waiting = queue.write();
        for folder in opened.fresh {
            if !waiting.contains(&folder) && !shown.contains(&folder) {
                waiting.push(folder);
            }
        }
    } else if let Some(id) = opened.go_to {
        state.overlays.write().retain(|o| !in_folder_flow(o));
        state.section.set(Section::Projects);
        state.selection.set(Selection::Project(id));
    }
}

/// The paths a drag event carries.
fn paths_of(event: &DragEvent) -> Vec<PathBuf> {
    event
        .files()
        .iter()
        .map(dioxus::html::FileData::path)
        .collect()
}

/// The window's drop state and handlers. Create it once in the shell.
#[derive(Clone)]
pub struct DropZone {
    state: AppState,
    hover: Signal<Option<Dropped>>,
    depth: Signal<i32>,
    queue: Signal<Vec<PathBuf>>,
    store: Option<Arc<berth_store::Store>>,
    link: Option<EngineLink>,
}

/// Start the drop state: the queue is provided as [`DropQueue`] and drained into Detected sheets.
pub fn use_drop_zone(state: AppState) -> DropZone {
    let seed = try_consume_context::<SeedHover>();
    let hover = use_signal(move || seed.map(|s| s.0));
    let depth = use_signal(|| 0);
    let mut queue = use_signal(Vec::<PathBuf>::new);
    use_context_provider(|| DropQueue(queue));
    let services = try_consume_context::<Services>();
    let link = try_consume_context::<Option<EngineLink>>().flatten();

    use_effect(move || {
        let overlays = state.overlays.read().clone();
        let waiting = queue.read().clone();
        if let Some(next) = next_in_queue(&overlays, &waiting) {
            queue.write().retain(|f| *f != next);
            let mut state = state;
            state.overlays.write().push(Overlay::Detected(next));
        }
    });

    DropZone {
        state,
        hover,
        depth,
        queue,
        store: services.map(|s| s.store.clone()),
        link,
    }
}

impl DropZone {
    /// What is hovering, for [`DropOverlay`].
    #[must_use]
    pub fn hover(&self) -> Signal<Option<Dropped>> {
        self.hover
    }

    /// `dragenter`: the files start hovering.
    pub fn enter(&self, event: &DragEvent) {
        event.prevent_default();
        if !accepts(&self.state.overlays.peek()) {
            return;
        }
        let paths = paths_of(event);
        if paths.is_empty() {
            return;
        }
        let mut depth = self.depth;
        *depth.write() += 1;
        let mut hover = self.hover;
        hover.set(Some(classify(&paths)));
    }

    /// `dragover`: allow the drop, and show the overlay if `enter` was missed.
    pub fn over(&self, event: &DragEvent) {
        event.prevent_default();
        if self.hover.peek().is_none() && accepts(&self.state.overlays.peek()) {
            let paths = paths_of(event);
            if !paths.is_empty() {
                let mut hover = self.hover;
                hover.set(Some(classify(&paths)));
                let mut depth = self.depth;
                depth.set(1);
            }
        }
    }

    /// `dragleave`: the overlay goes when the pointer has left every element it entered.
    pub fn leave(&self) {
        let mut depth = self.depth;
        let left = {
            let mut d = depth.write();
            *d = (*d - 1).max(0);
            *d == 0
        };
        if left {
            let mut hover = self.hover;
            hover.set(None);
        }
    }

    /// `drop`: open what was dropped.
    pub fn dropped(&self, event: &DragEvent) {
        event.prevent_default();
        let paths = paths_of(event);
        self.finish(&paths);
    }

    /// Open `paths` as a drop would (the part of `drop` after the event).
    pub fn finish(&self, paths: &[PathBuf]) {
        let mut hover = self.hover;
        hover.set(None);
        let mut depth = self.depth;
        depth.set(0);
        let mut state = self.state;
        if paths.is_empty() || !accepts(&state.overlays.peek()) {
            return;
        }
        let dropped = classify(paths);
        if dropped.folders.is_empty() {
            let text = hover_text(&dropped, &berth_sys::paths::home());
            state.toasts.write().push(Toast {
                id: uuid::Uuid::now_v7(),
                text: format!("{}. {}", text.title, text.body),
                tone: ToastTone::Danger,
                action: None,
            });
            return;
        }
        let known = Known::from_projects(&state.projects.peek());
        let engine = engine_for_new_pin(&state);
        let opened = open_dropped(
            self.store.as_deref(),
            self.link.as_ref(),
            &dropped.folders,
            &engine,
            &known,
            chrono::Utc::now(),
        );
        let mut queue = self.queue;
        apply(&mut state, &mut queue, opened);
    }
}

/// The engine a new pin belongs to: the engine in scope, else the default, else a running one.
fn engine_for_new_pin(state: &AppState) -> EngineId {
    let scope = match &*state.engine_scope.peek() {
        crate::state::EngineScope::One(id) => Some(id.clone()),
        crate::state::EngineScope::All => None,
    };
    let default = state.settings.peek().default_engine.clone();
    let running: Vec<EngineId> = state
        .engines
        .peek()
        .iter()
        .filter(|e| e.state == berth_core::EngineState::Running)
        .map(|e| e.id.clone())
        .collect();
    folders::pick_engine(scope.as_ref(), default.as_ref(), &running)
}

/// What shows over the window while files hover. It lets the pointer through, so the events keep
/// reaching the shell under it.
#[component]
pub fn DropOverlay(hover: Signal<Option<Dropped>>) -> Element {
    let Some(dropped) = hover.read().clone() else {
        return rsx! {};
    };
    let text = hover_text(&dropped, &berth_sys::paths::home());
    let refuse = text.refuse;
    let icon = if refuse { "alert-circle" } else { "folder" };
    rsx! {
        style { "{DROP_CSS}" }
        div {
            class: if refuse { "drop-overlay drop-refuse" } else { "drop-overlay" },
            role: "status",
            "aria-live": "polite",
            div { class: "drop-box",
                div { class: "drop-icon",
                    Icon { name: icon, size: IconSize::Md }
                }
                h2 { class: "drop-title", "{text.title}" }
                if !text.detail.is_empty() {
                    p { class: "drop-detail", "{text.detail}" }
                }
                p { class: "drop-body", "{text.body}" }
            }
        }
    }
}

#[cfg(test)]
#[path = "drop_overlay_tests.rs"]
mod tests;
