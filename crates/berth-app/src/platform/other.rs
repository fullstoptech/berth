use dioxus::desktop::{Config, LogicalSize, WindowBuilder};

use crate::assets;

pub fn config() -> Config {
    let window = WindowBuilder::new()
        .with_title("Berth")
        .with_inner_size(LogicalSize::new(
            super::window_size().0,
            super::window_size().1,
        ))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0));
    Config::new()
        .with_window(window)
        .with_custom_head(assets::head_html())
}

pub fn use_traffic_lights() {}

/// No menu bar item off macOS yet (D3).
pub fn use_tray(_model: dioxus::prelude::ReadSignal<crate::services::menubar::MenuModel>) {}

/// No Dock off macOS.
pub fn set_dock_icon_visible(_visible: bool) {}

/// No native menu bar off macOS yet (D3).
pub fn use_menu_events(_state: crate::state::AppState) {}

/// No native menu bar off macOS yet (D3), so no item to relabel.
pub fn set_dock_menu_label(_dock: berth_store::InspectorDock) {}

/// What to run in a terminal app (macOS only, D42).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecTarget {
    pub docker_host: String,
    pub container: String,
    pub shell: String,
    pub user: Option<String>,
    pub workdir: Option<String>,
}

pub async fn open_in_terminal_app(_target: ExecTarget) -> Result<&'static str, String> {
    Err("Open in Terminal app is available on macOS only".to_string())
}
