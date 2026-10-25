//! Engine actions (brief §7.2 row + dashboard; the menu bar reuses these).

use super::{def, ActionDef, ActionScope as S, Area};
use crate::keys::{Chord, SelectionKind};

pub const DEFS: &[ActionDef] = &[
    def("engine.start", "Start engine", None, S::Engine),
    def("engine.stop", "Stop engine", None, S::Engine),
    def("engine.start_stop", "Start/Stop engine", None, S::Engine),
    def("engine.restart", "Restart engine", None, S::Engine),
    def("engine.edit_resources", "Edit resources", None, S::Engine),
    def("engine.clean_up", "Clean up", None, S::Engine),
    def("engine.add", "Add engine", None, S::Engine),
];

const CHORDS: &[Chord] = &[Chord::cmd(&["."], "engine.start_stop")
    .when(|ctx| !ctx.in_assistant && ctx.selection == SelectionKind::Engine)];

const PENDING: &[(&str, &str)] = &[
    ("engine.start", "Starting…"),
    ("engine.stop", "Stopping…"),
    ("engine.restart", "Restarting…"),
    ("engine.edit_resources", "Applying…"),
];

pub const AREA: Area = Area {
    defs: DEFS,
    chords: CHORDS,
    pending: PENDING,
    ..Area::EMPTY
};
