//! Actions of the overlays: closing the top one, the palette's own keys
//! and the folder browser's hidden-files toggle.

use super::{def, ActionDef, ActionScope as S, Area, Handler};
use crate::keys::Chord;
use dioxus::prelude::WritableExt;

pub const DEFS: &[ActionDef] = &[
    def("overlay.close", "Close", Some("Esc"), S::Global),
    def("palette.next", "Next result", Some("↓"), S::Global),
    def("palette.prev", "Previous result", Some("↑"), S::Global),
    def("palette.run", "Run selected", Some("↵"), S::Global),
    def(
        "folders.toggle_hidden",
        "Show hidden files",
        Some("⌘⇧."),
        S::Global,
    ),
];

const HANDLERS: &[(&str, Handler)] = &[
    ("overlay.close", |state, _| {
        state.overlays.write().pop();
    }),
    (
        "folders.toggle_hidden",
        crate::sheets::open_folder::toggle_hidden,
    ),
];

const CHORDS: &[Chord] = &[
    // ⌘⇧. only means something in the folder browser.
    Chord::cmd_shift(&["."], "folders.toggle_hidden").when(|ctx| ctx.in_folder_browser),
];

pub const AREA: Area = Area {
    defs: DEFS,
    handlers: HANDLERS,
    chords: CHORDS,
    ..Area::EMPTY
};
