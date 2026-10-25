//! Assistant tab actions (D77: ⌘. stops the turn here, but
//! `container.start_stop` carries the shortcut string).

use super::{def, ActionDef, ActionScope as S, Area};
use crate::keys::Chord;

pub const DEFS: &[ActionDef] = &[
    def(
        "assistant.stop_turn",
        "Stop the assistant turn",
        None,
        S::Assistant,
    ),
    def("assistant.approve_tool", "Approve", None, S::Assistant),
    def("assistant.deny_tool", "Deny", None, S::Assistant),
];

const CHORDS: &[Chord] = &[Chord::cmd(&["."], "assistant.stop_turn").when(|ctx| ctx.in_assistant)];

pub const AREA: Area = Area {
    defs: DEFS,
    chords: CHORDS,
    ..Area::EMPTY
};
