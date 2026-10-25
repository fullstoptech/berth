use dioxus::prelude::*;

use crate::state::{Section, Selection};

/// Does the current section show an inspector for the current selection?
/// A container selection inspects in Containers, an image in Images, and
/// so on; a project has its own screen, not an inspector.
pub fn inspector_visible(section: Section, selection: &Selection) -> bool {
    match (section, selection) {
        (Section::Containers, Selection::Containers(ids)) => !ids.is_empty(),
        (Section::Images, Selection::Image(_))
        | (Section::Volumes, Selection::Volume(_))
        | (Section::Networks, Selection::Network(_))
        | (Section::Engines, Selection::Engine(_)) => true,
        _ => false,
    }
}

/// The 360 px inspector column (PLAN 7.1). Docked at 1200 px and wider;
/// below that it is hidden, and `overlay_open` (Space, P2-T7) shows it as
/// an overlay. The body is filled per section by later tasks (P3-T3 for
/// containers); the shell only owns the frame.
#[component]
pub fn Inspector(#[props(default)] overlay_open: bool, children: Element) -> Element {
    let class = if overlay_open {
        "inspector inspector-open"
    } else {
        "inspector"
    };
    rsx! {
        aside { class, "aria-label": "Inspector", {children} }
    }
}
