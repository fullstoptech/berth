#![allow(unused_imports)]
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod other;

#[cfg(target_os = "macos")]
pub use macos::{
    config, open_in_terminal_app, set_dock_icon_visible, set_dock_menu_label, use_menu_events,
    use_traffic_lights, use_tray, ExecTarget,
};
#[cfg(not(target_os = "macos"))]
pub use other::{
    config, open_in_terminal_app, set_dock_icon_visible, set_dock_menu_label, use_menu_events,
    use_traffic_lights, use_tray, ExecTarget,
};

thread_local! {
    /// The main window, kept so the pieces that run outside a component (the dock click, a tray
    /// item) can show or close it. Main thread only, like the window.
    static WINDOW: std::cell::RefCell<Option<dioxus::desktop::DesktopContext>> =
        const { std::cell::RefCell::new(None) };
}

/// Remember the main window (called once from the tray hook, inside the App root).
pub(crate) fn register_window(window: dioxus::desktop::DesktopContext) {
    WINDOW.with(|slot| *slot.borrow_mut() = Some(window));
}

/// Bring the main window back: unhide, unminimise, focus and activate the app.
pub fn show_window() {
    WINDOW.with(|slot| {
        if let Some(window) = slot.borrow().as_ref() {
            window.window.set_visible(true);
            window.window.set_minimized(false);
            window.window.set_focus();
        }
    });
}

/// Quit for real: save the snapshot now (the final save of D23), make closing the window close
/// instead of hide, then close it, which ends the event loop. Call from inside the App root's
/// scope (an event handler registered by a hook).
pub fn quit() {
    if let (Some(state), Some(services)) = (
        dioxus::prelude::try_consume_context::<crate::state::AppState>(),
        dioxus::prelude::try_consume_context::<crate::services::Services>(),
    ) {
        crate::services::engines::save_snapshots(&state, &services.store);
    }
    WINDOW.with(|slot| {
        if let Some(window) = slot.borrow().as_ref() {
            window.set_close_behavior(dioxus::desktop::WindowCloseBehaviour::WindowCloses);
            window.close();
        } else {
            std::process::exit(0);
        }
    });
}

/// The window's inner size: 1440x900 (D8), or `BERTH_WINDOW=<w>x<h>` for
/// shots of the narrower frames (for example `1024x700`). The minimum
/// size (900x600) still applies.
pub fn window_size() -> (f64, f64) {
    parse_window_size(std::env::var("BERTH_WINDOW").ok().as_deref())
}

fn parse_window_size(value: Option<&str>) -> (f64, f64) {
    const DEFAULT: (f64, f64) = (1440.0, 900.0);
    let Some((w, h)) = value.and_then(|v| v.split_once('x')) else {
        return DEFAULT;
    };
    match (w.trim().parse::<f64>(), h.trim().parse::<f64>()) {
        (Ok(w), Ok(h)) if w >= 900.0 && h >= 600.0 => (w, h),
        _ => DEFAULT,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_window_size;

    #[test]
    fn window_size_defaults_and_accepts_the_frame_sizes() {
        assert_eq!(parse_window_size(None), (1440.0, 900.0));
        assert_eq!(parse_window_size(Some("1024x700")), (1024.0, 700.0));
        assert_eq!(parse_window_size(Some("nonsense")), (1440.0, 900.0));
        assert_eq!(parse_window_size(Some("300x300")), (1440.0, 900.0));
    }
}
