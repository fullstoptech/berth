//! Volume actions (brief §7.2).

use super::{danger, def, ActionDef, ActionScope as S, Area, Handler};
use crate::keys::{Chord, SelectionKind};

pub const DEFS: &[ActionDef] = &[
    def("volume.browse", "Browse", None, S::Volume),
    def("volume.backup", "Back up", None, S::Volume),
    def("volume.create", "Create volume", None, S::Volume),
    danger("volume.delete", "Delete", None, S::Volume),
];

const HANDLERS: &[(&str, Handler)] = &[
    // ⌫ on the selected volume asks for its name first (P4-T4).
    ("volume.delete", |state, _| {
        crate::screens::volumes::open_delete_for_selection(state);
    }),
];

const CHORDS: &[Chord] =
    &[Chord::bare(&["Backspace"], "volume.delete")
        .when(|ctx| ctx.selection == SelectionKind::Volume)];

const PENDING: &[(&str, &str)] = &[
    ("volume.create", "Creating…"),
    ("volume.delete", "Deleting…"),
];

pub const AREA: Area = Area {
    defs: DEFS,
    handlers: HANDLERS,
    chords: CHORDS,
    pending: PENDING,
};
