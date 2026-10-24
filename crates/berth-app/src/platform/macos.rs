use dioxus::desktop::{Config, LogicalPosition, LogicalSize, WindowBuilder};
use dioxus::prelude::use_hook;

use crate::assets;

const TRAFFIC_LIGHT_INSET: (f64, f64) = (12.0, 19.0);

pub fn config() -> Config {
    use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;
    let window = WindowBuilder::new()
        .with_title("Berth")
        .with_inner_size(LogicalSize::new(1440.0, 900.0))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0))
        .with_titlebar_transparent(true)
        .with_fullsize_content_view(true)
        .with_title_hidden(true);
    Config::new()
        .with_window(window)
        .with_custom_head(assets::head_html())
}

/// Place the traffic lights inside Berth's own 38 px title bar (D8).
/// Goes through the webview because wry replaces the content view, so
/// tao's own with_traffic_light_inset never applies (Ferry precedent).
pub fn use_traffic_lights() {
    use dioxus::desktop::wry::WebViewExtMacOS;
    let window = dioxus::desktop::use_window();
    use_hook(move || {
        let inset = LogicalPosition::new(TRAFFIC_LIGHT_INSET.0, TRAFFIC_LIGHT_INSET.1);
        if let Err(e) = window.webview.set_traffic_light_inset(inset) {
            tracing::warn!(error = %e, "could not place the traffic lights");
        }
    });
}
