//! Container actions: the row and detail header buttons, the shortcuts
//! (⌘T, ⌘L, ⌘E, ⌘R, ⌘., ⌫) and the bulk bar.

use super::{danger, def, ActionDef, ActionScope as S, Area, Handler};
use crate::keys::{Chord, SelectionKind};
use crate::state::{AppState, BottomTab, Section, Selection};
use dioxus::prelude::{ReadableExt, WritableExt};

/// Container (brief §7.2 row + detail header), then the bulk bar (labels
/// gain counts at render time).
pub const DEFS: &[ActionDef] = &[
    def("container.start", "Start", None, S::Container),
    def("container.stop", "Stop", None, S::Container),
    def(
        "container.start_stop",
        "Start/Stop",
        Some("⌘."),
        S::Container,
    ),
    def("container.restart", "Restart", Some("⌘R"), S::Container),
    def("container.pause", "Pause", None, S::Container),
    def("container.resume", "Resume", None, S::Container),
    danger("container.kill", "Kill", None, S::Container),
    danger("container.delete", "Delete", Some("⌫"), S::Container),
    def("container.logs", "Logs", Some("⌘L"), S::Container),
    def("container.terminal", "Terminal", Some("⌘T"), S::Container),
    def(
        "container.edit",
        "Edit and recreate",
        Some("⌘E"),
        S::Container,
    ),
    def("container.duplicate", "Duplicate", None, S::Container),
    def(
        "container.recreate_latest",
        "Recreate with latest image",
        None,
        S::Container,
    ),
    def("container.rename", "Rename", None, S::Container),
    def(
        "container.save_as_image",
        "Save as image",
        None,
        S::Container,
    ),
    def("bulk.start", "Start selected", None, S::Container),
    def("bulk.stop", "Stop selected", None, S::Container),
    def("bulk.restart", "Restart selected", None, S::Container),
    def("bulk.pause", "Pause selected", None, S::Container),
    danger("bulk.delete", "Delete selected", None, S::Container),
];

const HANDLERS: &[(&str, Handler)] = &[
    ("container.logs", |state, _| {
        let mut bottom = state.bottom.write();
        bottom.open = true;
        bottom.tab = BottomTab::Logs;
    }),
    // ⌘T and the row's Terminal button (P3-T6).
    ("container.terminal", |state, _| {
        crate::services::exec::dispatch_open(state);
    }),
    // ⌘E opens the edit and recreate sheet of the selected container (P3-T7).
    ("container.edit", |state, _| {
        crate::sheets::edit_recreate::open_for_selection(state);
    }),
    // ⌫ on the selected containers asks first (D33, P2-T8).
    ("container.delete", |state, _| {
        open_delete_for_selection(state)
    }),
    ("bulk.delete", |state, _| open_delete_for_selection(state)),
];

const CHORDS: &[Chord] = &[
    Chord::cmd(&["t", "T"], "container.terminal"),
    Chord::cmd(&["l", "L"], "container.logs"),
    Chord::cmd(&["e", "E"], "container.edit"),
    Chord::cmd(&["r", "R"], "container.restart"),
    Chord::cmd(&["."], "container.start_stop")
        .when(|ctx| !ctx.in_assistant && ctx.selection == SelectionKind::Container),
    Chord::bare(&["Backspace"], "container.delete")
        .when(|ctx| ctx.selection == SelectionKind::Container),
];

/// What the actions service says while one of these runs (D34).
const PENDING: &[(&str, &str)] = &[
    ("container.start", "Starting…"),
    ("container.stop", "Stopping…"),
    ("container.restart", "Restarting…"),
    ("container.pause", "Pausing…"),
    ("container.resume", "Resuming…"),
    ("container.kill", "Killing…"),
    ("container.delete", "Deleting…"),
    ("container.rename", "Renaming…"),
    ("container.apply_limits", "Applying…"),
    ("container.recreate", "Recreating…"),
    ("container.duplicate", "Duplicating…"),
];

pub const AREA: Area = Area {
    defs: DEFS,
    handlers: HANDLERS,
    chords: CHORDS,
    pending: PENDING,
};

/// Open the delete dialog for the selected containers that can be
/// deleted. Only on the Containers screen and never over another overlay.
fn open_delete_for_selection(state: &mut AppState) {
    if *state.section.peek() != Section::Containers || !state.overlays.peek().is_empty() {
        return;
    }
    let ids = match &*state.selection.peek() {
        Selection::Containers(ids) => ids.clone(),
        _ => return,
    };
    let deletable = crate::services::actions::with_view(state, |view| {
        crate::ui::bulk_bar::applicable(&ids, crate::ui::BulkAction::Delete, view)
    });
    crate::services::confirm::open_delete(state, &deletable);
}
