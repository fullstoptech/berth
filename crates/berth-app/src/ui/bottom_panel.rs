use dioxus::prelude::*;

use super::icon_button::IconButton;
use super::tabs::{TabItem, Tabs, TabsVariant};
use crate::state::{AppState, BottomTab};

pub const MIN_HEIGHT: u32 = 160;
const MAX_FRACTION: f64 = 0.6;

/// The panel height for a drag: never below 160 px, never above 60 % of
/// the window (PLAN 7.1). A window too small for 160 px keeps the minimum.
pub fn clamp_panel_height(requested: f64, window_height: f64) -> u32 {
    let max = (window_height * MAX_FRACTION)
        .floor()
        .max(f64::from(MIN_HEIGHT));
    let clamped = requested.clamp(f64::from(MIN_HEIGHT), max);
    // Clamped into [160, max] above, so the conversion cannot truncate.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let height = clamped as u32;
    height
}

fn tab_id(tab: BottomTab) -> &'static str {
    match tab {
        BottomTab::Logs => "logs",
        BottomTab::Terminal => "terminal",
        BottomTab::Problems => "problems",
        BottomTab::Assistant => "assistant",
    }
}

fn tab_from_id(id: &str) -> Option<BottomTab> {
    [
        BottomTab::Logs,
        BottomTab::Terminal,
        BottomTab::Problems,
        BottomTab::Assistant,
    ]
    .into_iter()
    .find(|t| tab_id(*t) == id)
}

/// The tab strip's items; Problems carries the live count.
pub fn bottom_tabs(problem_count: usize) -> Vec<TabItem> {
    vec![
        TabItem::new("logs", "Logs"),
        TabItem::new("terminal", "Terminal"),
        TabItem::with_count("problems", "Problems", problem_count),
        TabItem::new("assistant", "Assistant"),
    ]
}

/// A drag in progress: where it started and the room it has.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Drag {
    start_y: f64,
    start_height: u32,
    window_height: f64,
}

fn window_height() -> f64 {
    let window = dioxus::desktop::window();
    window
        .inner_size()
        .to_logical::<f64>(window.scale_factor())
        .height
}

/// Bottom panel (⌃`): tabs, a close button and a top edge you drag to
/// resize, 160 px up to 60 % of the window. Tab bodies are filled by the
/// log (P3-T2), terminal (P3-T6), problems and assistant tasks.
#[component]
pub fn BottomPanel() -> Element {
    let mut state = use_context::<AppState>();
    let mut drag = use_signal(|| None::<Drag>);
    let panel = state.bottom.read().clone();
    if !panel.open {
        return rsx! {};
    }
    let problem_count = state.problems.read().len();
    let dragging = drag().is_some();
    rsx! {
        section {
            class: "bottom-panel",
            style: "height: {panel.height_px}px;",
            "aria-label": "Bottom panel",
            div {
                class: "bottom-resize",
                role: "separator",
                "aria-orientation": "horizontal",
                "aria-valuenow": "{panel.height_px}",
                "aria-label": "Resize panel",
                onmousedown: move |e| {
                    drag.set(Some(Drag {
                        start_y: e.client_coordinates().y,
                        start_height: state.bottom.peek().height_px,
                        window_height: window_height(),
                    }));
                },
            }
            div { class: "bottom-head",
                Tabs {
                    items: bottom_tabs(problem_count),
                    selected: tab_id(panel.tab).to_string(),
                    variant: TabsVariant::Panel,
                    aria_label: "Panel".to_string(),
                    on_select: move |id: String| {
                        if let Some(tab) = tab_from_id(&id) {
                            state.bottom.write().tab = tab;
                        }
                    },
                }
                div { class: "bottom-close",
                    IconButton {
                        icon: "x",
                        label: "Close panel".to_string(),
                        tooltip: "Close \u{b7} \u{2303}`".to_string(),
                        onclick: move |_| state.bottom.write().open = false,
                    }
                }
            }
            div { class: "bottom-body", role: "tabpanel" }
            if dragging {
                div {
                    class: "resize-capture",
                    onmousemove: move |e| {
                        if let Some(d) = drag() {
                            let requested =
                                f64::from(d.start_height) + (d.start_y - e.client_coordinates().y);
                            let height = clamp_panel_height(requested, d.window_height);
                            if state.bottom.peek().height_px != height {
                                state.bottom.write().height_px = height;
                            }
                        }
                    },
                    onmouseup: move |_| drag.set(None),
                    onmouseleave: move |_| drag.set(None),
                }
            }
        }
    }
}
