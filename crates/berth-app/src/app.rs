use dioxus::prelude::*;

use crate::assets;
use crate::platform;
use crate::scenes;
use crate::services::actions::ActionsService;
use crate::services::{action_runners, activity, engines, Services};
use crate::state::{theme_script, use_app_state, AppState, Overlay};
use crate::ui::{inspector_visible, BottomPanel, Inspector, Sidebar, StatusBar, TitleBar};

struct Boot {
    services: Services,
    actions: ActionsService,
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
    let scene = std::env::var_os("BERTH_SCENE").is_some();
    let mock = std::env::var("BERTH_MOCK").is_ok_and(|v| v == "1");
    // A scene runs no live service, so its actions never reach a real binary.
    let (compose, colima) = action_runners(mock || scene);
    let actions = ActionsService::start(
        state,
        clients.clone(),
        std::sync::Arc::clone(&store),
        compose,
        colima,
        cancel.clone(),
    );
    let handle = if scene {
        None
    } else {
        Some(engines::start(state, store, clients, cancel, mock))
    };
    Boot {
        services,
        actions,
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
    crate::screens::containers::provide_containers_view();
    platform::use_menu_events(state);

    // Services spawn once, before scene/theme forcing below (D23).
    let boot = use_hook(|| std::rc::Rc::new(boot_services(state)));
    let services = boot.services.clone();
    use_context_provider(|| services);
    let actions = boot.actions.clone();
    use_context_provider(|| actions);

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

/// What an overlay is called until its own screen exists (P2-T8, P2-T9,
/// P4 and P5 each replace the placeholder body for their variants).
fn overlay_title(overlay: &Overlay) -> &'static str {
    match overlay {
        Overlay::Palette => "Command palette",
        Overlay::Confirm(_) => "Confirm",
        Overlay::ReviewChanges(_) => "Review changes",
        Overlay::Generate(_) => "Generate",
        Overlay::Detected(_) => "Detected",
        Overlay::OpenFolder => "Open folder",
        Overlay::DropFolder(_) => "Add folders",
        Overlay::NewProject => "New project",
        Overlay::EditRecreate(_) => "Edit and recreate",
        Overlay::Run(_) => "Run image",
        Overlay::CleanUp(_) => "Clean up",
        Overlay::CreateVolume => "Create volume",
        Overlay::CreateNetwork => "Create network",
        Overlay::EditResources(_) => "Edit resources",
        Overlay::AddEngine => "Add engine",
        Overlay::AddRegistry => "Add registry",
        Overlay::BuildImage => "Build image",
        Overlay::AddService(_) => "Add service",
    }
}

/// The overlay stack (PLAN 6.11): every entry dims what is below it, and
/// Esc (keys.rs `overlay.close`) or a click on the dimmed area closes the
/// top one only.
#[component]
fn OverlayStack() -> Element {
    let mut state = use_context::<AppState>();
    let overlays = state.overlays.read().clone();
    let top = overlays.len().saturating_sub(1);
    rsx! {
        for (index, overlay) in overlays.iter().enumerate() {
            div {
                key: "{index}",
                class: "overlay-scrim",
                style: "z-index: {50 + index};",
                onclick: move |_| {
                    if index == top {
                        state.overlays.write().pop();
                    }
                },
                if matches!(overlay, Overlay::Palette) {
                    crate::screens::palette::PaletteScreen {}
                } else if let Overlay::Confirm(model) = overlay {
                    crate::ui::ConfirmOverlay { key: "{model.title}", model: model.clone() }
                } else {
                    div {
                        class: "overlay-card",
                        role: "dialog",
                        "aria-modal": "true",
                        "aria-label": overlay_title(overlay),
                        onclick: move |e| e.stop_propagation(),
                        h2 { "{overlay_title(overlay)}" }
                        p { "This screen is not built yet. Press Esc to close." }
                    }
                }
            }
        }
    }
}

/// The shell (PLAN 7.1): title bar over [sidebar | main + inspector over
/// the bottom panel], status bar under everything, overlays on top. The
/// root is focusable so the one keydown handler (keys.rs) sees shortcuts
/// whenever nothing narrower holds focus. Screen content lands per frame
/// from Phase 2 on.
#[component]
pub fn Shell() -> Element {
    let state = use_context::<AppState>();
    let show_inspector = inspector_visible(*state.section.read(), &state.selection.read());
    rsx! {
        div {
            class: "shell",
            tabindex: "-1",
            onmounted: move |e| {
                spawn(async move {
                    let _ = e.set_focus(true).await;
                });
            },
            onkeydown: move |e| {
                let mut state = state;
                crate::keys::handle_root_keydown(&mut state, &e);
            },
            TitleBar {}
            div { class: "shell-body",
                Sidebar {}
                div { class: "shell-center",
                    div { class: "shell-work",
                        main { class: "main",
                            if *state.dev_gallery.read() {
                                crate::ui::PrimitivesGallery {}
                            }
                            if *state.section.read() == crate::state::Section::Activity {
                                crate::screens::activity::ActivityScreen {}
                            }
                            if !*state.dev_gallery.read() && *state.section.read() == crate::state::Section::Projects {
                                crate::screens::projects::ProjectsScreen {}
                            }
                            if *state.section.read() == crate::state::Section::Containers {
                                crate::screens::containers::ContainersScreen {}
                            }
                        }
                        if show_inspector {
                            Inspector { overlay_open: crate::screens::containers::inspector_toggled() }
                        }
                    }
                    BottomPanel {}
                }
            }
            StatusBar {}
            OverlayStack {}
        }
    }
}
