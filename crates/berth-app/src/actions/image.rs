//! Image actions (brief §7.2).

use super::{danger, def, ActionDef, ActionScope as S, Area, Handler};
use crate::keys::{Chord, SelectionKind};

pub const DEFS: &[ActionDef] = &[
    def("image.run", "Run", None, S::Image),
    def("image.pull", "Pull image", None, S::Image),
    def("image.pull_latest", "Pull latest", None, S::Image),
    def("image.tag", "Tag", None, S::Image),
    def("image.push", "Push", None, S::Image),
    danger("image.delete", "Delete", None, S::Image),
];

const HANDLERS: &[(&str, Handler)] = &[
    // ⌫ on the selected image asks first (AD25a, P4-T1).
    ("image.delete", |state, _| {
        crate::screens::images::delete_selected(state);
    }),
];

const CHORDS: &[Chord] =
    &[Chord::bare(&["Backspace"], "image.delete")
        .when(|ctx| ctx.selection == SelectionKind::Image)];

const PENDING: &[(&str, &str)] = &[("image.delete", "Deleting…")];

pub const AREA: Area = Area {
    defs: DEFS,
    handlers: HANDLERS,
    chords: CHORDS,
    pending: PENDING,
};
