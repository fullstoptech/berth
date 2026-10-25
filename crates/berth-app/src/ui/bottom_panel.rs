use dioxus::prelude::*;

use super::dock::RegionMode;
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

/// The Logs tab: a log view over the selected containers (or the selected project's), opened by
/// the Logs button on a row, ⌘L or the project row's Logs button.
#[component]
fn BottomLogs() -> Element {
    let state = use_context::<AppState>();
    let ids = crate::screens::logs::ids_from_selection(
        &state.selection.read(),
        &state.containers.read(),
        &state.projects.read(),
    );
    rsx! { crate::screens::logs::LogPanel { containers: ids } }
}

/// What the region shows now: the dock's answer, or the panel alone when
/// the shell made no dock signals (a panel rendered by itself).
fn region_of(open: bool) -> Option<RegionMode> {
    match super::dock::dock_view() {
        Some(view) => view.layout.read().region,
        None => open.then_some(RegionMode::PanelOnly),
    }
}

/// The bottom region (⌃`, AD22): one full-width area under the main area with
/// a top edge you drag to resize, 160 px up to 60 % of the window. It shows
/// the Logs/Terminal/Problems/Assistant panel, the docked inspector (the
/// `inspector` element the shell built), or both side by side with the panel
/// on the left. Tab bodies are filled by the log (P3-T2), terminal (P3-T6),
/// problems (P5-T8) and assistant tasks.
#[component]
pub fn BottomPanel(#[props(default = rsx! {})] inspector: Element) -> Element {
    let mut state = use_context::<AppState>();
    let mut drag = use_signal(|| None::<Drag>);
    let panel = state.bottom.read().clone();
    let Some(mode) = region_of(panel.open) else {
        return rsx! {};
    };
    let problem_count = state.problems.read().len();
    let dragging = drag().is_some();
    let data_mode = match mode {
        RegionMode::InspectorOnly => "inspector",
        RegionMode::PanelOnly => "panel",
        RegionMode::Split => "split",
    };
    let shows_panel = mode != RegionMode::InspectorOnly;
    rsx! {
        section {
            class: "bottom-panel",
            "data-mode": data_mode,
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
            div { class: "bottom-split",
                if shows_panel {
                    div { class: "bottom-main",
                        div { class: "bottom-head",
                            Tabs {
                                items: bottom_tabs(problem_count),
                                selected: tab_id(panel.tab).to_string(),
                                variant: TabsVariant::Underline,
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
                        div { class: "bottom-body", role: "tabpanel",
                            if panel.tab == BottomTab::Terminal {
                                super::terminal::TerminalBody {}
                            }
                            if panel.tab == BottomTab::Logs {
                                BottomLogs {}
                            }
                            if panel.tab == BottomTab::Problems {
                                crate::screens::problems::ProblemsPanel {}
                            }
                        }
                    }
                }
                {inspector}
            }
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
