//! The View menu: where the inspector docks and how big the text is.

use super::{def, ActionDef, ActionScope as S, Area, Handler};
use crate::keys::Chord;
use crate::state::AppState;
use crate::text_scale::Step;
use dioxus::prelude::{ReadableExt, WritableExt};

pub const DEFS: &[ActionDef] = &[
    // View > dock the inspector at the other side (AD21, AD22). The View menu
    // item's label names the side a click moves to.
    def(
        "view.dock_inspector",
        "Dock inspector at the other side",
        Some("⌘⇧D"),
        S::Global,
    ),
    // View > text size (owner request 2026-10-25). Bigger also answers to
    // ⌘+ and ⌘⇧= (`keys::map_key`); only the ⌘= string is carried here.
    def("view.text_bigger", "Bigger text", Some("⌘="), S::Global),
    def("view.text_smaller", "Smaller text", Some("⌘-"), S::Global),
    def("view.text_reset", "Actual text size", Some("⌘0"), S::Global),
];

const HANDLERS: &[(&str, Handler)] = &[
    ("view.dock_inspector", |state, _| {
        crate::ui::dock::flip_dock(state);
    }),
    ("view.text_bigger", |state, _| {
        step_text_scale(state, Step::Bigger);
    }),
    ("view.text_smaller", |state, _| {
        step_text_scale(state, Step::Smaller);
    }),
    ("view.text_reset", |state, _| {
        step_text_scale(state, Step::Reset);
    }),
];

const CHORDS: &[Chord] = &[
    // ⌘⇧= types "+" on most layouts; same as ⌘+.
    Chord::cmd_shift(&["+", "="], "view.text_bigger"),
    Chord::cmd_shift(&["D", "d"], "view.dock_inspector"),
    Chord::cmd(&["=", "+"], "view.text_bigger"),
    Chord::cmd(&["-", "_"], "view.text_smaller"),
    Chord::cmd(&["0"], "view.text_reset"),
];

pub const AREA: Area = Area {
    defs: DEFS,
    handlers: HANDLERS,
    chords: CHORDS,
    ..Area::EMPTY
};

/// Move the text size one step; the App root applies and saves it.
fn step_text_scale(state: &mut AppState, step: Step) {
    let current = state.settings.peek().text_scale;
    let next = crate::text_scale::next_scale(current, step);
    if (next - current).abs() > f32::EPSILON {
        state.settings.write().text_scale = next;
    }
}
