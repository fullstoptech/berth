use dioxus::prelude::*;

use crate::state::{AppState, Section, Selection};
use crate::ui::{Button, ButtonVariant, ControlSize};

/// Does the current section show an inspector for the current selection?
/// A container selection inspects in Containers, an image in Images, and
/// so on; a project has its own screen, not an inspector.
pub fn inspector_visible(section: Section, selection: &Selection) -> bool {
    match (section, selection) {
        // Two or more selected rows show the bulk bar instead (P2-T8).
        (Section::Containers, Selection::Containers(ids)) => ids.len() == 1,
        // Engines draws its dashboard in the main area (frame "Engines /
        // dashboard / 1440"): a selected engine opens no inspector.
        (Section::Images, Selection::Image(_))
        | (Section::Volumes, Selection::Volume(_))
        | (Section::Networks, Selection::Network(_)) => true,
        _ => false,
    }
}

/// Which frame the inspector is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorFrame {
    /// The right column (PLAN 7.1), 360 px unless resized (AD21).
    #[default]
    Docked,
    /// Over the main area: Space opens it when the window is too narrow to
    /// dock it (D8, AD21).
    Overlay,
    /// The right pane of the bottom region, or all of it (AD22).
    Bottom,
}

impl InspectorFrame {
    #[must_use]
    pub fn class(self) -> &'static str {
        match self {
            Self::Docked => "inspector",
            Self::Overlay => "inspector inspector-overlay",
            Self::Bottom => "inspector inspector-bottom",
        }
    }
}

/// The inspector frame: a column at the right, an overlay, or a pane of the
/// bottom region. `width_px` is the width the user chose (a column, or a
/// pane next to the panel); without it the CSS gives the default width, or
/// all the room at the bottom. `handle` is the separator on its left edge.
/// The body is filled per section by later tasks (P3-T3 for containers);
/// the shell only owns the frame.
#[component]
pub fn Inspector(
    #[props(default)] frame: InspectorFrame,
    #[props(default)] width_px: Option<u32>,
    #[props(default = rsx! {})] handle: Element,
    children: Element,
) -> Element {
    let style = width_px.map(|w| format!("width: {w}px; min-width: {w}px; flex: none;"));
    rsx! {
        aside {
            class: frame.class(),
            style,
            "aria-label": "Inspector",
            {handle}
            div { class: "inspector-body", {children} }
        }
    }
}

/// The header button that opens the inspector as an overlay where the window
/// is too narrow to dock it (D8, AD21). Every screen with an inspector
/// (Containers, Images, Volumes, Networks) puts it last in its controls
/// row; it is hidden by the stylesheet while the inspector is docked, and
/// it runs the same action as Space and the View menu.
#[component]
pub fn InspectorToggle() -> Element {
    let mut state = use_context::<AppState>();
    let overlay = crate::ui::dock::inspector_is_overlay();
    rsx! {
        div {
            class: if overlay { "toggle-inspector toggle-inspector-on" } else { "toggle-inspector" },
            Button {
                label: "Inspector".to_string(),
                icon: "file-text",
                variant: ButtonVariant::Secondary,
                size: ControlSize::Md,
                onclick: move |_| crate::keys::dispatch(&mut state, "global.toggle_inspector"),
            }
        }
    }
}
