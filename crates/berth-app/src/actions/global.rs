//! Shell-wide actions: the palette, the sections (⌘1…⌘9), opening and
//! creating projects, and the keys that act on whichever list is showing.

use super::{def, ActionDef, ActionScope as S, Area, Handler};
use crate::keys::Chord;
use crate::state::{AppState, Overlay, Section};
use dioxus::prelude::WritableExt;

/// Global / shell (PLAN §7.4).
pub const DEFS: &[ActionDef] = &[
    def("global.palette", "Command palette", Some("⌘K"), S::Global),
    def("section.projects", "Projects", Some("⌘1"), S::Global),
    def("section.containers", "Containers", Some("⌘2"), S::Global),
    def("section.logs", "Logs", Some("⌘3"), S::Global),
    def("section.images", "Images", Some("⌘4"), S::Global),
    def("section.volumes", "Volumes", Some("⌘5"), S::Global),
    def("section.networks", "Networks", Some("⌘6"), S::Global),
    def("section.templates", "Templates", Some("⌘7"), S::Global),
    def("section.engines", "Engines", Some("⌘8"), S::Global),
    def("section.activity", "Activity", Some("⌘9"), S::Global),
    def(
        "global.logs_section",
        "Logs section",
        Some("⌘⇧L"),
        S::Global,
    ),
    def(
        "global.focus_filter",
        "Filter current list",
        Some("⌘F"),
        S::Global,
    ),
    def("global.open_folder", "Open folder…", Some("⌘O"), S::Global),
    def("global.new_project", "New project", Some("⌘N"), S::Global),
    def("global.settings", "Settings", Some("⌘,"), S::Global),
    def(
        "global.toggle_inspector",
        "Toggle inspector",
        Some("Space"),
        S::Global,
    ),
    def(
        "global.toggle_bottom_panel",
        "Toggle bottom panel",
        Some("⌃`"),
        S::Global,
    ),
    def("global.select_next", "Select next", Some("J"), S::Global),
    def(
        "global.select_prev",
        "Select previous",
        Some("K"),
        S::Global,
    ),
];

/// A handler that switches to one section.
macro_rules! goto {
    ($section:expr) => {
        |state: &mut AppState, _| state.section.set($section)
    };
}

const HANDLERS: &[(&str, Handler)] = &[
    ("global.palette", |state, _| {
        state.overlays.write().push(Overlay::Palette);
    }),
    ("global.open_folder", |state, _| {
        state.overlays.write().push(Overlay::OpenFolder);
    }),
    ("global.new_project", |state, _| {
        state.overlays.write().push(Overlay::NewProject);
    }),
    ("global.settings", goto!(Section::Settings)),
    ("global.logs_section", goto!(Section::Logs)),
    ("section.logs", goto!(Section::Logs)),
    ("section.projects", goto!(Section::Projects)),
    ("section.containers", goto!(Section::Containers)),
    ("section.images", goto!(Section::Images)),
    ("section.volumes", goto!(Section::Volumes)),
    ("section.networks", goto!(Section::Networks)),
    ("section.templates", goto!(Section::Templates)),
    ("section.engines", goto!(Section::Engines)),
    ("section.activity", goto!(Section::Activity)),
    ("global.toggle_bottom_panel", |state, _| {
        let mut bottom = state.bottom.write();
        bottom.open = !bottom.open;
    }),
    // J/K and Space act on the visible list (P2-T7).
    ("global.select_next", crate::screens::containers::dispatch),
    ("global.select_prev", crate::screens::containers::dispatch),
    (
        "global.toggle_inspector",
        crate::screens::containers::dispatch,
    ),
];

const CHORDS: &[Chord] = &[
    Chord::cmd(&["k", "K"], "global.palette"),
    Chord::cmd(&["f", "F"], "global.focus_filter"),
    Chord::cmd(&["o", "O"], "global.open_folder"),
    Chord::cmd(&["n", "N"], "global.new_project"),
    Chord::cmd(&[","], "global.settings"),
    // ⌘1…⌘9 follow the sidebar order; ⌘0 is the text size (view.rs).
    Chord::cmd(&["1"], "section.projects"),
    Chord::cmd(&["2"], "section.containers"),
    Chord::cmd(&["3"], "section.logs"),
    Chord::cmd(&["4"], "section.images"),
    Chord::cmd(&["5"], "section.volumes"),
    Chord::cmd(&["6"], "section.networks"),
    Chord::cmd(&["7"], "section.templates"),
    Chord::cmd(&["8"], "section.engines"),
    Chord::cmd(&["9"], "section.activity"),
    Chord::cmd_shift(&["L", "l"], "global.logs_section"),
    // ⌃` toggles the bottom panel, even from a text input.
    Chord::ctrl(&["`"], "global.toggle_bottom_panel"),
    Chord::bare_either_shift(&["j", "J"], "global.select_next"),
    Chord::bare_either_shift(&["k", "K"], "global.select_prev"),
    Chord::bare(&[" "], "global.toggle_inspector"),
];

pub const AREA: Area = Area {
    defs: DEFS,
    handlers: HANDLERS,
    chords: CHORDS,
    ..Area::EMPTY
};
