use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

use super::engine_switcher::EngineSwitcher;
use super::palette_trigger::PaletteTrigger;
use super::status_bar::ProblemsChip;

/// The 38 px window chrome: traffic lights (native, inset 12/19), the
/// engine switcher, the palette trigger and the problems count. The bar
/// itself is the drag region: a primary-button mousedown starts a window
/// drag and a double-click zooms. Interactive children stop both events,
/// so clicking them never moves or zooms the window.
#[component]
pub fn TitleBar() -> Element {
    rsx! {
        header {
            class: "titlebar",
            onmousedown: move |e| {
                if e.data.trigger_button() == Some(MouseButton::Primary) {
                    dioxus::desktop::window().drag();
                }
            },
            ondoubleclick: move |_| dioxus::desktop::window().toggle_maximized(),
            EngineSwitcher {}
            PaletteTrigger {}
            ProblemsChip {}
        }
    }
}
