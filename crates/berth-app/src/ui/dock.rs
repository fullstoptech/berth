//! Where the inspector sits (D21, D22): a right column you can resize, or
//! the right pane of the bottom region as in browser dev tools.
//!
//! Everything that decides a size or a place is a pure function here, so
//! the shell, the table and the tests read one answer. The window width
//! comes from the desktop window (`DockView::window_w`); the stored
//! choices live in `Settings` (`inspector_width_px`, `inspector_dock`,
//! `inspector_split_pct`). While a handle is dragged the live value sits in
//! a UI-only signal and is written to `Settings` once, on mouse-up.

use std::time::{Duration, Instant};

use dioxus::html::ModifiersInteraction;
use dioxus::prelude::*;

use berth_store::{
    InspectorDock, INSPECTOR_SPLIT_DEFAULT, INSPECTOR_WIDTH_DEFAULT, INSPECTOR_WIDTH_MAX,
    INSPECTOR_WIDTH_MIN,
};

use super::inspector::inspector_visible;
use crate::state::AppState;

/// The sidebar's width (`--size-sidebar`), which the main area gives up.
pub const SIDEBAR_PX: f64 = 220.0;
/// The main area never drops below this at 100 % text next to a docked
/// inspector; it grows and shrinks with the text size (D24).
pub const MAIN_MIN_PX: f64 = 640.0;
/// A window narrower than this never docks the inspector at the right.
pub const DOCK_MIN_WINDOW_PX: f64 = 1200.0;
/// Neither pane of the bottom region goes below this.
pub const PANE_MIN_PX: u32 = 320;
/// An arrow key moves a handle by this much, with Shift by the second.
pub const KEY_STEP_PX: u32 = 16;
pub const KEY_STEP_BIG_PX: u32 = 64;
/// Two presses on a handle this close together reset it.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);

// ---------------------------------------------------------------------
// The right column
// ---------------------------------------------------------------------

/// A stored width forced into 320..=560 (hand-edited or old data).
#[must_use]
pub fn clamp_stored_width(px: u32) -> u32 {
    px.clamp(INSPECTOR_WIDTH_MIN, INSPECTOR_WIDTH_MAX)
}

/// The least main area a docked inspector leaves (D24): 640 px at 100 %
/// text, 544 at 85 %, 960 at 150 %. The tables' narrowest column lists fit
/// it at every text size (the sweep in `containers_tests.rs`).
#[must_use]
pub fn main_min_px(text_scale: f32) -> f64 {
    // The steps are whole percents: round the f32 noise away (0.85 is not
    // exactly 85/100) so 544 px is 544.
    MAIN_MIN_PX * (f64::from(text_scale) * 100.0).round() / 100.0
}

/// The widest a docked inspector may be: the window less the sidebar and the
/// main area's floor, at most 560. `None` when the window is under 1200 px
/// or not even 320 px fit (at 1440 px and 150 % text): then it overlays.
#[must_use]
pub fn docked_max(window_w: f64, text_scale: f32) -> Option<u32> {
    if window_w < DOCK_MIN_WINDOW_PX {
        return None;
    }
    let room = floor_px(window_w - SIDEBAR_PX - main_min_px(text_scale));
    (room >= INSPECTOR_WIDTH_MIN).then_some(room.min(INSPECTOR_WIDTH_MAX))
}

/// The docked inspector's width: the stored one clamped to keep the floor,
/// or `None` when the window cannot dock it.
#[must_use]
pub fn docked_width(window_w: f64, stored: u32, text_scale: f32) -> Option<u32> {
    docked_max(window_w, text_scale).map(|max| clamp_stored_width(stored).min(max))
}

/// The overlay's width: the stored width, at most half the window.
#[must_use]
pub fn overlay_width(window_w: f64, stored: u32) -> u32 {
    clamp_stored_width(stored).min(floor_px(window_w * 0.5))
}

/// What a drag or a key may reach for the right column: 320..=560, and
/// while it can dock never so wide that the main area drops below its floor;
/// as an overlay never more than half the window.
#[must_use]
pub fn right_range(window_w: f64, text_scale: f32) -> (u32, u32) {
    let max = docked_max(window_w, text_scale).unwrap_or_else(|| {
        floor_px(window_w * 0.5).clamp(INSPECTOR_WIDTH_MIN, INSPECTOR_WIDTH_MAX)
    });
    (INSPECTOR_WIDTH_MIN, max)
}

/// `value` forced into the range a drag may reach.
#[must_use]
pub fn clamp_right_width(value: i64, window_w: f64, text_scale: f32) -> u32 {
    let (min, max) = right_range(window_w, text_scale);
    u32::try_from(value.clamp(i64::from(min), i64::from(max))).unwrap_or(min)
}

/// Where the inspector is drawn, if it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorSlot {
    #[default]
    Hidden,
    /// The right column, this wide.
    Docked(u32),
    /// Over the main area, this wide (Space opened it).
    Overlay(u32),
    /// The right pane (or all) of the bottom region.
    Bottom,
}

/// The inspector's place. `toggled` is Space: it flips the default, which
/// is shown when docked (right or bottom) and hidden as an overlay.
#[must_use]
pub fn inspector_slot(
    visible: bool,
    toggled: bool,
    dock: InspectorDock,
    stored: u32,
    window_w: f64,
    text_scale: f32,
) -> InspectorSlot {
    if !visible {
        return InspectorSlot::Hidden;
    }
    match dock {
        InspectorDock::Bottom if toggled => InspectorSlot::Hidden,
        InspectorDock::Bottom => InspectorSlot::Bottom,
        InspectorDock::Right => match docked_width(window_w, stored, text_scale) {
            Some(_) if toggled => InspectorSlot::Hidden,
            Some(width) => InspectorSlot::Docked(width),
            None if toggled => InspectorSlot::Overlay(overlay_width(window_w, stored)),
            None => InspectorSlot::Hidden,
        },
    }
}

// ---------------------------------------------------------------------
// The bottom region
// ---------------------------------------------------------------------

/// What the one full-width bottom region shows (D22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionMode {
    /// The inspector alone: the Logs/Terminal panel is closed.
    InspectorOnly,
    /// Logs/Terminal alone: nothing is selected or the inspector is hidden.
    PanelOnly,
    /// Logs/Terminal on the left, the inspector on the right.
    Split,
}

#[must_use]
pub fn region_mode(inspector_at_bottom: bool, panel_open: bool) -> Option<RegionMode> {
    match (inspector_at_bottom, panel_open) {
        (true, true) => Some(RegionMode::Split),
        (true, false) => Some(RegionMode::InspectorOnly),
        (false, true) => Some(RegionMode::PanelOnly),
        (false, false) => None,
    }
}

/// The width of the region: the window less the sidebar.
#[must_use]
pub fn region_width(window_w: f64) -> f64 {
    (window_w - SIDEBAR_PX).max(0.0)
}

/// What the divider may reach, as the inspector's width: each pane keeps
/// 320 px. A region too narrow for two panes splits in half.
#[must_use]
pub fn split_range(region_w: f64) -> (u32, u32) {
    let max = floor_px(region_w).saturating_sub(PANE_MIN_PX);
    if max < PANE_MIN_PX {
        let half = floor_px(region_w / 2.0);
        (half, half)
    } else {
        (PANE_MIN_PX, max)
    }
}

/// The inspector's width in a region for a stored share.
#[must_use]
pub fn split_px(region_w: f64, pct: u8) -> u32 {
    let (min, max) = split_range(region_w);
    round_px(region_w * f64::from(pct) / 100.0).clamp(min, max)
}

/// The share (1..=99 %) a width comes to.
#[must_use]
pub fn pct_for_px(px: u32, region_w: f64) -> u8 {
    if region_w <= 0.0 {
        return INSPECTOR_SPLIT_DEFAULT;
    }
    let pct = (f64::from(px) / region_w * 100.0).round().clamp(1.0, 99.0);
    // Clamped into 1..=99 above, so the conversion cannot truncate.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pct = pct as u8;
    pct
}

/// The share to store after the divider moved from `old_px` to `new_px`.
/// A whole percent is wider than a 16 px key step in a narrow region, so a
/// move that rounds back to the old share still moves one percent.
#[must_use]
pub fn pct_after_move(old_pct: u8, old_px: u32, new_px: u32, region_w: f64) -> u8 {
    let pct = pct_for_px(new_px, region_w);
    if pct == old_pct && new_px != old_px && split_px(region_w, pct) == old_px {
        return if new_px > old_px {
            old_pct.saturating_add(1).min(99)
        } else {
            old_pct.saturating_sub(1).max(1)
        };
    }
    pct
}

/// Everything the shell draws from, decided once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DockLayout {
    pub slot: InspectorSlot,
    pub region: Option<RegionMode>,
    /// The inspector's width, which the tab strip measures itself against.
    pub pane_px: u32,
    /// In `Split`, the inspector's width in the region.
    pub split_px: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutInput {
    pub inspector_visible: bool,
    pub toggled: bool,
    pub dock: InspectorDock,
    pub stored_width: u32,
    pub split_pct: u8,
    /// The divider's live width during a drag.
    pub split_live: Option<u32>,
    pub window_w: f64,
    pub text_scale: f32,
    pub panel_open: bool,
}

#[must_use]
pub fn layout(input: &LayoutInput) -> DockLayout {
    let slot = inspector_slot(
        input.inspector_visible,
        input.toggled,
        input.dock,
        input.stored_width,
        input.window_w,
        input.text_scale,
    );
    let region = region_mode(slot == InspectorSlot::Bottom, input.panel_open);
    let region_w = region_width(input.window_w);
    let (min, max) = split_range(region_w);
    let split = input.split_live.map_or_else(
        || split_px(region_w, input.split_pct),
        |px| px.clamp(min, max),
    );
    let pane_px = match (slot, region) {
        (InspectorSlot::Docked(w) | InspectorSlot::Overlay(w), _) => w,
        (InspectorSlot::Bottom, Some(RegionMode::Split)) => split,
        (InspectorSlot::Bottom, _) => floor_px(region_w),
        (InspectorSlot::Hidden, _) => 0,
    };
    DockLayout {
        slot,
        region,
        pane_px,
        split_px: split,
    }
}

// ---------------------------------------------------------------------
// Keyboard and double click
// ---------------------------------------------------------------------

/// The value a key on a handle moves to. The handle is the inspector's left
/// edge, so ← moves it left and widens the inspector (the WAI-ARIA window
/// splitter keys); Home and End reach the least and most; Enter resets.
#[must_use]
pub fn key_move(
    key: &str,
    shift: bool,
    value: u32,
    range: (u32, u32),
    default: u32,
) -> Option<u32> {
    let (min, max) = range;
    let step = if shift { KEY_STEP_BIG_PX } else { KEY_STEP_PX };
    let next = match key {
        "ArrowLeft" => value.saturating_add(step),
        "ArrowRight" => value.saturating_sub(step),
        "Home" => min,
        "End" => max,
        "Enter" => default,
        _ => return None,
    };
    Some(next.clamp(min, max))
}

/// Is this press the second of a double click.
#[must_use]
pub fn is_double_click(previous: Option<Instant>, now: Instant) -> bool {
    previous.is_some_and(|p| now.saturating_duration_since(p) <= DOUBLE_CLICK)
}

// A pixel count that is never negative and never near `u32::MAX`.
fn floor_px(value: f64) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let px = value.max(0.0).floor() as u32;
    px
}

fn round_px(value: f64) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let px = value.max(0.0).round() as u32;
    px
}

// ---------------------------------------------------------------------
// The signals
// ---------------------------------------------------------------------

/// The shell's dock signals, provided from the App root (and made by the
/// shell itself when nothing above did, as scenes and tests render it).
#[derive(Clone, Copy)]
pub struct DockView {
    /// The window's inner width in logical px.
    pub window_w: Signal<f64>,
    /// The right column's width while it is dragged.
    pub width_live: Signal<Option<u32>>,
    /// The divider's inspector width while it is dragged.
    pub split_live: Signal<Option<u32>>,
    pub layout: Memo<DockLayout>,
    /// The inspector takes room from the table: docked at the right.
    pub docked: Memo<bool>,
    /// The window cannot dock the inspector at the right, so Space (or the
    /// Containers header's button) opens it as an overlay.
    pub overlay: Memo<bool>,
}

fn initial_window_width() -> f64 {
    try_consume_context::<dioxus::desktop::DesktopContext>().map_or_else(
        || crate::platform::window_size().0,
        |window| {
            window
                .inner_size()
                .to_logical::<f64>(window.scale_factor())
                .width
        },
    )
}

/// Create the signals (call from a component body). When an ancestor
/// already provided a [`DockView`] that one is returned.
pub fn use_dock_view(state: AppState) -> DockView {
    let existing = use_hook(try_consume_context::<DockView>);
    let toggled = use_hook(|| {
        try_consume_context::<crate::screens::containers::ContainersView>()
            .map(|view| view.inspector_toggled)
    });
    let window_w = use_signal(initial_window_width);
    let width_live = use_signal(|| None::<u32>);
    let split_live = use_signal(|| None::<u32>);
    // Both memos read the same signals themselves: a memo of a memo can be
    // read stale in the pass that changed its source.
    let input = move || {
        let settings = state.settings.read();
        LayoutInput {
            inspector_visible: inspector_visible(*state.section.read(), &state.selection.read()),
            toggled: toggled.is_some_and(|signal| *signal.read()),
            dock: settings.inspector_dock,
            stored_width: width_live().unwrap_or(settings.inspector_width_px),
            split_pct: settings.inspector_split_pct,
            split_live: split_live(),
            window_w: window_w(),
            text_scale: settings.text_scale,
            panel_open: state.bottom.read().open,
        }
    };
    let layout = use_memo(move || self::layout(&input()));
    let docked = use_memo(move || matches!(self::layout(&input()).slot, InspectorSlot::Docked(_)));
    let overlay = use_memo(move || {
        let settings = state.settings.read();
        settings.inspector_dock == InspectorDock::Right
            && docked_max(window_w(), settings.text_scale).is_none()
    });
    use_hook(move || {
        existing.unwrap_or_else(|| {
            let view = DockView {
                window_w,
                width_live,
                split_live,
                layout,
                docked,
                overlay,
            };
            provide_context(view);
            view
        })
    })
}

/// Follow the window's width (call once, from the App root).
pub fn use_window_width(view: DockView) {
    dioxus::desktop::use_wry_event_handler(move |event, _| {
        use dioxus::desktop::tao::event::{Event, WindowEvent};
        let Event::WindowEvent {
            window_id,
            event: WindowEvent::Resized(size),
            ..
        } = event
        else {
            return;
        };
        let window = dioxus::desktop::window();
        if *window_id != window.id() {
            return;
        }
        let width = size.to_logical::<f64>(window.scale_factor()).width;
        let mut window_w = view.window_w;
        if (*window_w.peek() - width).abs() >= 0.5 {
            window_w.set(width);
        }
    });
}

/// `BERTH_INSPECTOR_WIDTH` forces the inspector width for development
/// shots, like `BERTH_TEXT_SCALE`.
#[must_use]
pub fn forced_width_from_env() -> Option<u32> {
    let value: u32 = std::env::var("BERTH_INSPECTOR_WIDTH")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(clamp_stored_width(value))
}

/// The dock signals of the current tree, if the shell made them.
#[must_use]
pub fn dock_view() -> Option<DockView> {
    try_consume_context::<DockView>()
}

/// Has the inspector the table's room: docked at the right. Without a
/// shell (a table rendered alone) nothing is docked.
#[must_use]
pub fn inspector_docked() -> bool {
    dock_view().is_some_and(|view| *view.docked.read())
}

/// Is the inspector an overlay here: docked at the right in a window too
/// narrow to dock it.
#[must_use]
pub fn inspector_is_overlay() -> bool {
    dock_view().is_some_and(|view| *view.overlay.read())
}

/// Move the inspector to `dock`, showing it there (Space's flip is reset).
pub fn set_dock(state: &mut AppState, dock: InspectorDock) {
    if state.settings.peek().inspector_dock == dock {
        return;
    }
    state.settings.write().inspector_dock = dock;
    if let Some(view) = try_consume_context::<crate::screens::containers::ContainersView>() {
        let mut toggled = view.inspector_toggled;
        if *toggled.peek() {
            toggled.set(false);
        }
    }
}

/// The other side: the key ⌘⇧D and the View menu item.
pub fn flip_dock(state: &mut AppState) {
    let next = match state.settings.peek().inspector_dock {
        InspectorDock::Right => InspectorDock::Bottom,
        InspectorDock::Bottom => InspectorDock::Right,
    };
    set_dock(state, next);
}

/// The View menu item's label: it names the side the click moves to.
#[must_use]
pub fn dock_menu_label(current: InspectorDock) -> &'static str {
    match current {
        InspectorDock::Right => "Dock Inspector at Bottom",
        InspectorDock::Bottom => "Dock Inspector at Right",
    }
}

// ---------------------------------------------------------------------
// The handles
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct DragStart {
    x: f64,
    value: u32,
}

/// A vertical separator the user drags, or moves with the keys. `value`,
/// `min` and `max` are the inspector's width in px; dragging left widens it.
/// `on_live` reports each drag step, `on_commit` the final value of a drag,
/// a key press or a reset (double click, Enter).
#[component]
pub fn SplitHandle(
    class: &'static str,
    label: String,
    value: u32,
    min: u32,
    max: u32,
    default: u32,
    on_live: EventHandler<u32>,
    on_commit: EventHandler<u32>,
) -> Element {
    let mut drag = use_signal(|| None::<DragStart>);
    let mut reached = use_signal(|| None::<u32>);
    let mut last_press = use_signal(|| None::<Instant>);
    rsx! {
        div {
            class: "split-handle {class}",
            role: "separator",
            tabindex: "0",
            "aria-orientation": "vertical",
            "aria-label": "{label}",
            "aria-valuenow": "{value}",
            "aria-valuemin": "{min}",
            "aria-valuemax": "{max}",
            onmousedown: move |e| {
                let now = Instant::now();
                if is_double_click(*last_press.peek(), now) {
                    last_press.set(None);
                    on_commit.call(default);
                    return;
                }
                last_press.set(Some(now));
                reached.set(None);
                drag.set(Some(DragStart { x: e.client_coordinates().x, value }));
            },
            onkeydown: move |e| {
                let shift = e.modifiers().shift();
                if let Some(next) = key_move(&e.key().to_string(), shift, value, (min, max), default) {
                    e.prevent_default();
                    e.stop_propagation();
                    if next != value {
                        on_commit.call(next);
                    }
                }
            },
        }
        if drag().is_some() {
            div {
                class: "resize-capture resize-capture-col",
                onmousemove: move |e| {
                    if let Some(start) = drag() {
                        #[allow(clippy::cast_possible_truncation)]
                        let moved = (start.x - e.client_coordinates().x).round() as i64;
                        let next = u32::try_from((i64::from(start.value) + moved).clamp(i64::from(min), i64::from(max)))
                            .unwrap_or(min);
                        if *reached.peek() != Some(next) {
                            reached.set(Some(next));
                            on_live.call(next);
                        }
                    }
                },
                onmouseup: move |_| {
                    if let Some(final_value) = reached.peek().as_ref().copied() {
                        on_commit.call(final_value);
                    }
                    drag.set(None);
                },
                onmouseleave: move |_| {
                    if let Some(final_value) = reached.peek().as_ref().copied() {
                        on_commit.call(final_value);
                    }
                    drag.set(None);
                },
            }
        }
    }
}

/// The handle on the right column's left edge (D21).
#[component]
pub fn InspectorWidthHandle() -> Element {
    let mut state = use_context::<AppState>();
    let Some(view) = dock_view() else {
        return rsx! {};
    };
    let stored = state.settings.read().inspector_width_px;
    let window_w = *view.window_w.read();
    let value = (*view.width_live.read()).unwrap_or(stored);
    let (min, max) = right_range(window_w, state.settings.read().text_scale);
    rsx! {
        SplitHandle {
            class: "split-handle-width",
            label: "Resize inspector".to_string(),
            value: value.clamp(min, max),
            min,
            max,
            default: INSPECTOR_WIDTH_DEFAULT,
            on_live: move |px: u32| {
                let mut live = view.width_live;
                live.set(Some(px));
            },
            on_commit: move |px: u32| {
                let mut live = view.width_live;
                live.set(None);
                if state.settings.peek().inspector_width_px != px {
                    state.settings.write().inspector_width_px = px;
                }
            },
        }
    }
}

/// The divider between the panel and the inspector in the bottom region (D22).
#[component]
pub fn InspectorSplitHandle() -> Element {
    let mut state = use_context::<AppState>();
    let Some(view) = dock_view() else {
        return rsx! {};
    };
    let layout = *view.layout.read();
    let region_w = region_width(*view.window_w.read());
    let (min, max) = split_range(region_w);
    let default_px = split_px(region_w, INSPECTOR_SPLIT_DEFAULT);
    rsx! {
        SplitHandle {
            class: "split-handle-split",
            label: "Resize inspector".to_string(),
            value: layout.split_px,
            min,
            max,
            default: default_px,
            on_live: move |px: u32| {
                let mut live = view.split_live;
                live.set(Some(px));
            },
            on_commit: move |px: u32| {
                let mut live = view.split_live;
                live.set(None);
                let (old_pct, old_px) = {
                    let pct = state.settings.peek().inspector_split_pct;
                    (pct, split_px(region_w, pct))
                };
                let pct = pct_after_move(old_pct, old_px, px, region_w);
                if pct != old_pct {
                    state.settings.write().inspector_split_pct = pct;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_width_clamps_to_320_560_and_leaves_the_main_area_its_floor() {
        // 1440 at 100 %: 1440 - 220 - 640 = 580, so the 560 cap binds.
        assert_eq!(clamp_right_width(100, 1440.0, 1.0), 320);
        assert_eq!(clamp_right_width(450, 1440.0, 1.0), 450);
        assert_eq!(clamp_right_width(900, 1440.0, 1.0), 560);
        // 1300: the main area binds, 1300 - 220 - 640 = 440.
        assert_eq!(clamp_right_width(560, 1300.0, 1.0), 440);
        // 1200: 340 is all that fits next to 640 px of main area.
        assert_eq!(clamp_right_width(560, 1200.0, 1.0), 340);
        // The floor is 640 px x text size (D24): 544 at 85 %, 960 at 150 %.
        assert_eq!(main_min_px(0.85), 544.0);
        assert_eq!(main_min_px(1.0), 640.0);
        assert_eq!(main_min_px(1.5), 960.0);
        assert_eq!(clamp_right_width(560, 1440.0, 0.85), 560);
        assert_eq!(clamp_right_width(560, 1300.0, 0.85), 536);
        assert_eq!(clamp_right_width(560, 1440.0, 1.3), 388);
        // Whatever the width, the main area keeps its floor once docked.
        for scale in crate::text_scale::STEPS {
            for window in [1200.0, 1250.0, 1300.0, 1440.0, 1920.0] {
                let Some(width) = docked_width(window, 10_000, scale) else {
                    continue;
                };
                assert!(
                    window - SIDEBAR_PX - f64::from(width) >= main_min_px(scale),
                    "{window} at {scale}"
                );
            }
        }
    }

    #[test]
    fn dock_rule_docks_from_1200_when_320_px_leave_the_floor_and_clamps_to_it() {
        // (window, text scale, stored width, docked width): the width is the
        // stored one clamped to what keeps the main area at 640 px x scale.
        for (window, scale, stored, docked) in [
            (1199.0, 1.0, 320, None),
            (1199.0, 0.85, 360, None),
            (1200.0, 1.0, 360, Some(340)), // 1200 - 220 - 640
            (1200.0, 1.0, 320, Some(320)),
            (1219.0, 1.0, 360, Some(359)),
            (1220.0, 1.0, 360, Some(360)),
            (1440.0, 1.0, 360, Some(360)),
            (1440.0, 1.0, 560, Some(560)),
            (1440.0, 1.0, 480, Some(480)),
            (1200.0, 1.0, 560, Some(340)),
            // 150 %: 1440 - 220 - 960 = 260 < 320, so it overlays.
            (1440.0, 1.5, 360, None),
            (1440.0, 1.5, 320, None),
            (1440.0, 1.3, 480, Some(388)),
            (1220.0, 1.3, 320, None), // 1220 - 220 - 832 = 168
            (1220.0, 0.85, 480, Some(456)),
            (1920.0, 1.5, 560, Some(560)),
        ] {
            assert_eq!(
                docked_width(window, stored, scale),
                docked,
                "{window} px at {scale}, stored {stored}"
            );
        }
    }

    #[test]
    fn dock_overlay_is_the_stored_width_capped_at_half_the_window() {
        assert_eq!(overlay_width(1024.0, 360), 360);
        assert_eq!(overlay_width(1024.0, 560), 512);
        assert_eq!(overlay_width(900.0, 560), 450);
    }

    #[test]
    fn dock_slot_follows_the_rule_and_space_flips_the_default() {
        let slot = |toggled, dock, stored, window| {
            inspector_slot(true, toggled, dock, stored, window, 1.0)
        };
        let right = InspectorDock::Right;
        // Docked by default, Space hides it.
        assert_eq!(slot(false, right, 360, 1440.0), InspectorSlot::Docked(360));
        assert_eq!(slot(true, right, 360, 1440.0), InspectorSlot::Hidden);
        // Overlay by default hidden, Space opens it.
        assert_eq!(slot(false, right, 360, 1024.0), InspectorSlot::Hidden);
        assert_eq!(slot(true, right, 360, 1024.0), InspectorSlot::Overlay(360));
        assert_eq!(slot(true, right, 560, 1024.0), InspectorSlot::Overlay(512));
        // 150 % text at 1440 overlays too, and Space opens it.
        let big = |toggled| inspector_slot(true, toggled, right, 360, 1440.0, 1.5);
        assert_eq!(big(false), InspectorSlot::Hidden);
        assert_eq!(big(true), InspectorSlot::Overlay(360));
        // A wide stored width docks clamped, not as an overlay.
        assert_eq!(slot(false, right, 560, 1300.0), InspectorSlot::Docked(440));
        // At the bottom the window width does not matter.
        assert_eq!(
            slot(false, InspectorDock::Bottom, 360, 1024.0),
            InspectorSlot::Bottom
        );
        assert_eq!(
            slot(true, InspectorDock::Bottom, 360, 1024.0),
            InspectorSlot::Hidden
        );
        // Nothing to inspect, nothing drawn.
        assert_eq!(
            inspector_slot(false, false, right, 360, 1440.0, 1.0),
            InspectorSlot::Hidden
        );
    }

    #[test]
    fn dock_region_shows_the_inspector_the_panel_or_both() {
        assert_eq!(region_mode(true, false), Some(RegionMode::InspectorOnly));
        assert_eq!(region_mode(false, true), Some(RegionMode::PanelOnly));
        assert_eq!(region_mode(true, true), Some(RegionMode::Split));
        assert_eq!(region_mode(false, false), None);
    }

    fn input(dock: InspectorDock, panel_open: bool, window_w: f64) -> LayoutInput {
        LayoutInput {
            inspector_visible: true,
            toggled: false,
            dock,
            stored_width: 360,
            split_pct: 40,
            split_live: None,
            window_w,
            text_scale: 1.0,
            panel_open,
        }
    }

    #[test]
    fn dock_layout_picks_the_region_mode_from_dock_selection_and_panel() {
        let at_bottom = |panel_open| layout(&input(InspectorDock::Bottom, panel_open, 1440.0));
        assert_eq!(at_bottom(false).region, Some(RegionMode::InspectorOnly));
        assert_eq!(at_bottom(true).region, Some(RegionMode::Split));
        // Hidden inspector: the panel alone.
        let hidden = layout(&LayoutInput {
            toggled: true,
            ..input(InspectorDock::Bottom, true, 1440.0)
        });
        assert_eq!(hidden.region, Some(RegionMode::PanelOnly));
        // Nothing selected: the panel alone, or no region.
        let none = |panel_open| {
            layout(&LayoutInput {
                inspector_visible: false,
                ..input(InspectorDock::Bottom, panel_open, 1440.0)
            })
        };
        assert_eq!(none(true).region, Some(RegionMode::PanelOnly));
        assert_eq!(none(false).region, None);
        // At the right the panel is the region's only content.
        let right = layout(&input(InspectorDock::Right, true, 1440.0));
        assert_eq!(right.region, Some(RegionMode::PanelOnly));
        assert_eq!(right.slot, InspectorSlot::Docked(360));
        assert_eq!(right.pane_px, 360);
    }

    #[test]
    fn dock_split_gives_the_inspector_40_percent_and_each_pane_320_px() {
        // 1440: the region is 1220 wide, 40 % is 488.
        assert_eq!(split_px(1220.0, 40), 488);
        // 1024: the region is 804, 40 % is 322, and both panes fit.
        assert_eq!(split_px(804.0, 40), 322);
        assert_eq!(split_range(804.0), (320, 484));
        // A share that would starve a pane is held back.
        assert_eq!(split_px(804.0, 10), 320);
        assert_eq!(split_px(804.0, 90), 484);
        assert_eq!(split_px(1220.0, 90), 900);
        for window in [900.0, 1024.0, 1200.0, 1440.0, 1920.0] {
            let region = region_width(window);
            for pct in [1, 10, 40, 60, 99] {
                let inspector = split_px(region, pct);
                assert!(inspector >= 320, "{window} {pct}");
                assert!(region - f64::from(inspector) >= 320.0, "{window} {pct}");
            }
        }
        // The layout carries it, and the pane the tabs measure.
        let split = layout(&input(InspectorDock::Bottom, true, 1024.0));
        assert_eq!((split.split_px, split.pane_px), (322, 322));
        // Alone at the bottom the inspector has the whole region.
        let alone = layout(&input(InspectorDock::Bottom, false, 1024.0));
        assert_eq!(alone.pane_px, 804);
    }

    #[test]
    fn dock_split_share_follows_the_divider_and_always_moves_on_a_key() {
        assert_eq!(pct_for_px(488, 1220.0), 40);
        assert_eq!(pct_for_px(0, 1220.0), 1);
        assert_eq!(pct_for_px(5000, 1220.0), 99);
        // 16 px of 804 is 2 %, of 1220 only 1.3 %: still a step each time.
        let region = 804.0;
        let old = split_px(region, 40);
        assert_eq!(pct_after_move(40, old, old + 16, region), 42);
        let region = 4000.0;
        let old = split_px(region, 40);
        assert_eq!(old, 1600);
        // 16 px of 4000 px is 0.4 %: it rounds to the old share, so the
        // key still moves 1 %.
        assert_eq!(pct_after_move(40, old, old + 16, region), 41);
        assert_eq!(pct_after_move(40, old, old - 16, region), 39);
        // No move, no change.
        assert_eq!(pct_after_move(40, old, old, region), 40);
    }

    #[test]
    fn dock_keys_step_16_or_64_and_reach_both_ends() {
        let range = (320, 560);
        // The handle moves left, so the inspector grows.
        assert_eq!(key_move("ArrowLeft", false, 360, range, 360), Some(376));
        assert_eq!(key_move("ArrowRight", false, 360, range, 360), Some(344));
        assert_eq!(key_move("ArrowLeft", true, 360, range, 360), Some(424));
        assert_eq!(key_move("ArrowRight", true, 360, range, 360), Some(320));
        assert_eq!(key_move("Home", false, 400, range, 360), Some(320));
        assert_eq!(key_move("End", false, 400, range, 360), Some(560));
        // Never past either end.
        assert_eq!(key_move("ArrowLeft", true, 540, range, 360), Some(560));
        assert_eq!(key_move("ArrowRight", false, 324, range, 360), Some(320));
        // Enter resets; other keys are not the handle's.
        assert_eq!(key_move("Enter", false, 500, range, 360), Some(360));
        assert_eq!(key_move("a", false, 500, range, 360), None);
        assert_eq!(key_move("ArrowUp", false, 500, range, 360), None);
    }

    #[test]
    fn dock_two_presses_within_400_ms_reset_and_slower_ones_do_not() {
        let t0 = Instant::now();
        assert!(!is_double_click(None, t0));
        assert!(is_double_click(Some(t0), t0 + Duration::from_millis(150)));
        assert!(is_double_click(Some(t0), t0 + DOUBLE_CLICK));
        assert!(!is_double_click(Some(t0), t0 + Duration::from_millis(900)));
    }

    #[test]
    fn dock_menu_label_names_the_side_the_click_moves_to() {
        assert_eq!(
            dock_menu_label(InspectorDock::Right),
            "Dock Inspector at Bottom"
        );
        assert_eq!(
            dock_menu_label(InspectorDock::Bottom),
            "Dock Inspector at Right"
        );
    }
}
