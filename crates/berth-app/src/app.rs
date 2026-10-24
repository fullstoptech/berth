use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

use crate::assets;
use crate::platform;
use crate::scenes;
use crate::state::{theme_script, use_app_state, AppState, Section};

/// Desktop entry point. Platform hooks that need a real window live here so
/// the shared Shell stays renderable under dioxus-ssr (scenes).
#[component]
pub fn App() -> Element {
    assets::use_assets();
    platform::use_traffic_lights();
    let mut state = use_app_state();
    use_context_provider(|| state);

    // BERTH_SCENE puts the UI into a frame's state for screenshots (D58).
    use_hook(move || {
        if let Ok(frame) = std::env::var("BERTH_SCENE") {
            if let Some(scene) = scenes::find(&frame) {
                (scene.setup)(&mut state);
            } else {
                tracing::warn!(%frame, "unknown BERTH_SCENE frame");
            }
        }
        // BERTH_THEME forces the theme for development shots.
        if let Ok(forced) = std::env::var("BERTH_THEME") {
            let theme = match forced.as_str() {
                "dark" => Some(berth_store::Theme::Dark),
                "light" => Some(berth_store::Theme::Light),
                _ => None,
            };
            if let Some(theme) = theme {
                state.settings.write().theme = theme;
            }
        }
    });

    // Theme and density ride on <html> so tokens.css media rules apply (D7).
    use_effect(move || {
        let settings = state.settings.peek();
        let script = theme_script(settings.theme, settings.density);
        drop(settings);
        spawn(async move {
            let _ = document::eval(&script).await;
        });
    });

    rsx! {
        style { "{assets::FONTS_CSS}" }
        Shell {}
    }
}

#[component]
fn NavItem(icon: &'static str, label: &'static str, target: Section) -> Element {
    let state = use_context::<AppState>();
    let selected = *state.section.read() == target;
    let class = if selected {
        "nav-item selected"
    } else {
        "nav-item"
    };
    rsx! {
        div { class,
            svg {
                r#use { href: "/icons.svg#{icon}" }
            }
            span { "{label}" }
        }
    }
}

/// The shell skeleton (P0-T4): title bar, sidebar navigation, empty main.
/// Screen content lands per frame from Phase 2 on.
#[component]
pub fn Shell() -> Element {
    let state = use_context::<AppState>();
    let problem_count = state.problems.read().len();
    let problems_label = if problem_count == 0 {
        "No problems".to_string()
    } else {
        format!("{problem_count} problems")
    };
    rsx! {
        div { class: "shell",
            div {
                class: "titlebar",
                onmousedown: move |e| {
                    if e.data.trigger_button() == Some(MouseButton::Primary) {
                        dioxus::desktop::window().drag();
                    }
                },
                div { class: "titlebar-search", "Search or run a command…" }
                div { class: "titlebar-problems", "{problems_label}" }
            }
            div { class: "body",
                nav { class: "sidebar",
                    NavItem { icon: "folder", label: "Projects", target: Section::Projects }
                    NavItem { icon: "box", label: "Containers", target: Section::Containers }
                    NavItem { icon: "logs", label: "Logs", target: Section::Logs }
                    NavItem { icon: "layers", label: "Images", target: Section::Images }
                    NavItem { icon: "hard-drive", label: "Volumes", target: Section::Volumes }
                    NavItem { icon: "network", label: "Networks", target: Section::Networks }
                    NavItem { icon: "file-text", label: "Templates", target: Section::Templates }
                    NavItem { icon: "server", label: "Engines", target: Section::Engines }
                    NavItem { icon: "cloud", label: "Registries", target: Section::Registries }
                    NavItem { icon: "activity", label: "Activity", target: Section::Activity }
                }
                main { class: "main" }
            }
        }
    }
}
