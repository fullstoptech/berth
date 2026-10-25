use dioxus::desktop::{Config, LogicalSize, WindowBuilder};

use crate::assets;

pub fn config() -> Config {
    let window = WindowBuilder::new()
        .with_title("Berth")
        .with_inner_size(LogicalSize::new(1440.0, 900.0))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0));
    Config::new()
        .with_window(window)
        .with_custom_head(assets::head_html())
}

pub fn use_traffic_lights() {}

/// No native menu bar off macOS yet (D3).
pub fn use_menu_events(_state: crate::state::AppState) {}
