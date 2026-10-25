//! Project actions: the row and detail header buttons.

use super::{danger, def, ActionDef, ActionScope as S, Area};
use crate::keys::{Chord, SelectionKind};

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
    chords: CHORDS,
    pending: PENDING,
    ..Area::EMPTY
};
