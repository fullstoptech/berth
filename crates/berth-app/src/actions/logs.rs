//! Log view actions (PLAN §7.4 F8; brief §7.3 Logs actions).

use super::{def, ActionDef, ActionScope as S, Area, Handler};
use crate::keys::Chord;

pub const DEFS: &[ActionDef] = &[
    def("logs.next_error", "Next error", Some("F8"), S::Logs),
    def("logs.prev_error", "Previous error", Some("⇧F8"), S::Logs),
    def("logs.follow", "Follow", None, S::Logs),
    def("logs.explain", "Explain", None, S::Logs),
    def("logs.ask", "Ask about these lines", None, S::Logs),
    def("logs.copy", "Copy", None, S::Logs),
    def("logs.save", "Save logs", None, S::Logs),
];

const HANDLERS: &[(&str, Handler)] = &[
    // F8 and Shift+F8 reach the active log view (P3-T2).
    ("logs.next_error", |_, action| {
        crate::ui::log_view::dispatch(action);
    }),
    ("logs.prev_error", |_, action| {
        crate::ui::log_view::dispatch(action);
    }),
];

const CHORDS: &[Chord] = &[
    Chord::function(&["F8"], false, "logs.next_error").when(|ctx| ctx.in_log_view),
    Chord::function(&["F8"], true, "logs.prev_error").when(|ctx| ctx.in_log_view),
];

pub const AREA: Area = Area {
    defs: DEFS,
    handlers: HANDLERS,
    chords: CHORDS,
    ..Area::EMPTY
};
