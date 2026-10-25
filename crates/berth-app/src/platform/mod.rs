#![allow(unused_imports)]
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod other;

#[cfg(target_os = "macos")]
pub use macos::{config, use_menu_events, use_traffic_lights};
#[cfg(not(target_os = "macos"))]
pub use other::{config, use_menu_events, use_traffic_lights};

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
