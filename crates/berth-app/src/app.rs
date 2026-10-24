use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

use crate::assets;
use crate::platform;
use crate::scenes;
use crate::services::{activity, engines, Services};
use crate::state::{theme_script, use_app_state, AppState, Section};

struct Boot {
    services: Services,
    // Dropped with the App root on quit (cancels tasks, saves snapshots).
    _engines: Option<engines::EnginesHandle>,
}

/// Open the store and start the engine service once (P2-T1, D23).
/// `BERTH_DATA_DIR` wins through `berth_sys::paths::data_dir`;
/// `BERTH_MOCK=1` selects the mock world; a `BERTH_SCENE` frame is
/// authoritative, so no live service runs under it (shots stay exact).
fn boot_services(state: AppState) -> Boot {
    let data_dir = berth_sys::paths::data_dir();
    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        tracing::warn!(%err, dir = %data_dir.display(), "data dir create failed");
    }
    let db_path = data_dir.join("berth.db");
    let store = match berth_store::Store::open(&db_path) {
        Ok(store) => store,
        Err(err) => {
            tracing::error!(%err, path = %db_path.display(), "store open failed; using in-memory store");
            match berth_store::Store::open_in_memory() {
                Ok(store) => store,
                Err(err) => {
                    // An in-memory SQLite open failing leaves nothing
                    // honest to run on.
                    tracing::error!(%err, "in-memory store open failed; exiting");
                    std::process::exit(1);
                }
            }
        }
    };
    let store = std::sync::Arc::new(store);
    match store.settings() {
        Ok(settings) => {
            let mut state = state;
            state.settings.set(settings);
        }
        Err(err) => tracing::warn!(%err, "settings load failed"),
    }
    activity::prune_old(&store, chrono::Utc::now());

    let clients = engines::client_map();
    let cancel = tokio_util::sync::CancellationToken::new();
    let services = Services {
        store: std::sync::Arc::clone(&store),
        clients: clients.clone(),
        cancel: cancel.clone(),
    };
    let handle = if std::env::var_os("BERTH_SCENE").is_some() {
        None
    } else {
        let mock = std::env::var("BERTH_MOCK").is_ok_and(|v| v == "1");
        Some(engines::start(state, store, clients, cancel, mock))
    };
    Boot {
        services,
        _engines: handle,
    }
}

/// Desktop entry point. Platform hooks that need a real window live here so
/// the shared Shell stays renderable under dioxus-ssr (scenes).
#[component]
pub fn App() -> Element {
    assets::use_assets();
    platform::use_traffic_lights();
    let mut state = use_app_state();
    use_context_provider(|| state);

    // Services spawn once, before scene/theme forcing below (D23).
    let boot = use_hook(|| std::rc::Rc::new(boot_services(state)));
    let services = boot.services.clone();
    use_context_provider(|| services);

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
        div {
            class: "shell",
            onkeydown: move |e| {
                let mut state = state;
                crate::keys::handle_root_keydown(&mut state, &e);
            },
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
