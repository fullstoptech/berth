//! Network actions (brief §7.2).

use super::{danger, def, ActionDef, ActionScope as S, Area, Handler};
use crate::keys::{Chord, SelectionKind};

pub const DEFS: &[ActionDef] = &[
    def(
        "network.connect_container",
        "Connect container",
        None,
        S::Network,
    ),
    def("network.disconnect", "Disconnect", None, S::Network),
    def("network.create", "Create network", None, S::Network),
    danger("network.delete", "Delete", None, S::Network),
];

const HANDLERS: &[(&str, Handler)] = &[
    // ⌫ on the selected network asks first, unless it is built in (P4-T5).
    ("network.delete", |state, _| {
        crate::screens::networks::delete_selected(state);
    }),
];

const CHORDS: &[Chord] = &[Chord::bare(&["Backspace"], "network.delete")
    .when(|ctx| ctx.selection == SelectionKind::Network)];

const PENDING: &[(&str, &str)] = &[
    ("network.create", "Creating…"),
    ("network.delete", "Deleting…"),
    ("network.connect_container", "Connecting…"),
    ("network.disconnect", "Disconnecting…"),
];

pub const AREA: Area = Area {
    defs: DEFS,
    handlers: HANDLERS,
    chords: CHORDS,
    pending: PENDING,
};
