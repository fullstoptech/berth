//! Project actions: the row and detail header buttons.

use super::{danger, def, ActionDef, ActionScope as S, Area, Handler};
use crate::keys::{Chord, SelectionKind};
use crate::state::Overlay;
use dioxus::prelude::{ReadableExt, WritableExt};

/// Project (brief §7.2 row + detail header).
pub const DEFS: &[ActionDef] = &[
    def("project.start", "Start project", None, S::Project),
    def("project.stop", "Stop project", None, S::Project),
    def("project.start_stop", "Start/Stop project", None, S::Project),
    def("project.restart", "Restart project", None, S::Project),
    def("project.rebuild", "Rebuild", None, S::Project),
    def("project.pull_images", "Pull images", None, S::Project),
    def("project.logs", "Logs", None, S::Project),
    def("project.open_folder", "Open folder", None, S::Project),
    def("project.open_in_editor", "Open in editor", None, S::Project),
    danger("project.delete", "Delete project", None, S::Project),
];

const HANDLERS: &[(&str, Handler)] = &[
    // Backspace and the palette: ask first, in the Delete project sheet (P5-T6).
    ("project.delete", |state, _| {
        // Only on the project's own screen: a project that stays selected
        // after the user left it must not catch a stray Backspace.
        if *state.section.peek() != crate::state::Section::Project {
            return;
        }
        if let Some(project) = crate::services::project_files::selected_project(state) {
            state
                .overlays
                .write()
                .push(Overlay::DeleteProject(project.id));
        }
    }),
    ("project.open_folder", |state, _| {
        crate::screens::project_detail::open_selected_folder(state);
    }),
    ("project.open_in_editor", |state, _| {
        crate::screens::project_detail::open_selected_in_editor(state);
    }),
];

const CHORDS: &[Chord] = &[
    Chord::cmd(&["."], "project.start_stop")
        .when(|ctx| !ctx.in_assistant && ctx.selection == SelectionKind::Project),
    Chord::bare(&["Backspace"], "project.delete")
        .when(|ctx| ctx.selection == SelectionKind::Project),
];

const PENDING: &[(&str, &str)] = &[
    ("project.start", "Starting…"),
    ("project.stop", "Stopping…"),
    ("project.restart", "Restarting…"),
    ("project.delete", "Deleting…"),
    ("project.rebuild", "Rebuilding…"),
    ("project.pull_images", "Pulling…"),
];

pub const AREA: Area = Area {
    defs: DEFS,
    handlers: HANDLERS,
    chords: CHORDS,
    pending: PENDING,
};
