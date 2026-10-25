use dioxus::prelude::*;

use crate::assets;
use crate::platform;
use crate::scenes;
use crate::services::actions::ActionsService;
use crate::services::problems::ProblemsService;
use crate::services::stats::{use_window_visibility, StatsService};
use crate::services::{action_runners, activity, engines, Services};
use crate::state::{theme_script, use_app_state, AppState};
use crate::ui::dock::{InspectorSlot, InspectorSplitHandle, InspectorWidthHandle, RegionMode};
use crate::ui::{BottomPanel, Inspector, InspectorFrame, Sidebar, StatusBar, TitleBar};

struct Boot {
    services: Services,
    actions: ActionsService,
    problems: ProblemsService,
    stats: StatsService,
    /// Brings engines online after Colima starts them; None in a scene.
    link: Option<engines::EngineLink>,
    // Dropped with the App root on quit (cancels tasks, saves snapshots).
    _engines: Option<engines::EnginesHandle>,
}

/// Open the store and start the engine service once (P2-T1, D23).
/// `BERTH_DATA_DIR` wins through `berth_sys::paths::data_dir`;
/// `BERTH_MOCK=1` selects the mock world; a `BERTH_SCENE` frame is
/// authoritative, so no live service runs under it (shots stay exact).
fn boot_services(mut state: AppState) -> Boot {
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
        Ok(mut settings) => {
            // A stored value that is not one of the steps snaps to the nearest.
            settings.text_scale = crate::text_scale::snap(settings.text_scale);
            // The same for a width or share edited by hand.
            settings.inspector_width_px =
                crate::ui::dock::clamp_stored_width(settings.inspector_width_px);
            settings.inspector_split_pct = settings.inspector_split_pct.clamp(1, 99);
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
    state
        .backing
        .set(crate::services::backing::Backing::at_boot(mock, scene));
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
    // The Problems list (P5-T8): rules re-run when the engines, projects or a Compose
    // failure change. A scene shows the frame's problems and runs no pass.
    let problems = if scene {
        ProblemsService::inert()
    } else {
        ProblemsService::start(state, services.clone(), actions.clone())
    };
    // Live stats (D26): streams open only for what is on screen.
    let stats = StatsService::start(clients.clone(), cancel.clone());
    let handle = if scene {
        None
    } else {
        Some(engines::start(state, store, clients, cancel, mock))
    };
    let link = handle.as_ref().map(engines::EnginesHandle::link);
    if let Some(link) = &link {
        actions.set_link(link.clone());
    }
    Boot {
        services,
        actions,
        problems,
        stats,
        link,
        _engines: handle,
    }
}

/// Desktop entry point. Platform hooks that need a real window live here so
/// the shared Shell stays renderable under dioxus-ssr (scenes).
#[component]
pub fn App() -> Element {
    assets::use_assets();
    crate::bridge::use_web_assets();
    platform::use_traffic_lights();
    let mut state = use_app_state();
    use_context_provider(|| state);
    let views = crate::providers::provide_screen_views(state);
    crate::ui::dock::use_window_width(views.dock);
    platform::use_menu_events(state);

    // Services spawn once, before scene/theme forcing below (D23).
    let boot = use_hook(|| std::rc::Rc::new(boot_services(state)));
    let services = boot.services.clone();
    use_context_provider(|| services);
    let actions = boot.actions.clone();
    use_context_provider(|| actions);
    let problems = boot.problems.clone();
    use_context_provider(|| problems.clone());
    use_effect(move || {
        // Everything the rules read; the service skips a pass whose inputs did not change.
        let _ = state.engines.read();
        let _ = state.containers.read();
        let _ = state.projects.read();
        let _ = state.pending.read();
        problems.nudge();
    });
    let link = boot.link.clone();
    use_context_provider(|| link);
    let stats = boot.stats.clone();
    use_window_visibility(stats.clone());
    use_context_provider(|| stats);
    // Terminal sessions (P3-T6): the view signals first, then the core that
    // keeps the tab strip current.
    let terminal_view = crate::services::exec::use_terminal_view();
    use_context_provider(|| terminal_view);
    let exec = use_hook(|| {
        crate::services::exec::ExecCore::start(
            state,
            terminal_view,
            boot.services.clients.clone(),
            boot.services.store.clone(),
            boot.services.cancel.clone(),
        )
    });
    use_context_provider(|| exec);

    // BERTH_SCENE puts the UI into a frame's state for screenshots (D58).
    use_hook(move || {
        if let Ok(frame) = std::env::var("BERTH_SCENE") {
            if let Some(scene) = scenes::find(&frame) {
                (scene.setup)(&mut state);
                scenes::seed_stores(&state, &views);
            } else if frame != crate::bridge::dev::FRAME {
                tracing::warn!(%frame, "unknown BERTH_SCENE frame");
            }
        }
        // BERTH_OPEN_GENERATE=<folder> opens the Generate sheet for a folder on a live engine
        // (the folder browser and drop overlay that normally open it come later in Phase 5).
        if let Some(folder) = std::env::var_os("BERTH_OPEN_GENERATE") {
            state
                .overlays
                .write()
                .push(crate::state::Overlay::Generate(folder.into()));
        }
        // BERTH_OPEN_DETECTED=<folder> opens the Detected sheet for a folder, as a drop of it would
        // (for live checks where the Finder drag cannot be scripted).
        if let Some(folder) = std::env::var_os("BERTH_OPEN_DETECTED") {
            state
                .overlays
                .write()
                .push(crate::state::Overlay::Detected(folder.into()));
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
        // BERTH_TEXT_SCALE forces the text size for development shots.
        if let Some(scale) = crate::text_scale::forced_from_env() {
            state.settings.write().text_scale = scale;
        }
        // BERTH_INSPECTOR_WIDTH forces the inspector width for shots.
        if let Some(width) = crate::ui::dock::forced_width_from_env() {
            state.settings.write().inspector_width_px = width;
        }
    });

    // Text size and inspector changes are saved as they happen (one small
    // write, off the UI path; a drag writes once, on mouse-up). The first
    // run only records the loaded values, and a forced BERTH_TEXT_SCALE or
    // BERTH_INSPECTOR_WIDTH, or a scene, is never written to the owner's
    // settings.
    let store = boot.services.store.clone();
    let saved = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(None)));
    use_effect(move || {
        let settings = state.settings.read().clone();
        let key = (
            settings.text_scale.to_bits(),
            settings.inspector_width_px,
            settings.inspector_dock,
            settings.inspector_split_pct,
        );
        if saved.get() == Some(key) {
            return;
        }
        let first_run = saved.get().is_none();
        saved.set(Some(key));
        if first_run
            || crate::text_scale::forced_from_env().is_some()
            || crate::ui::dock::forced_width_from_env().is_some()
            || std::env::var_os("BERTH_SCENE").is_some()
        {
            return;
        }
        let store = store.clone();
        spawn(async move {
            let saved = tokio::task::spawn_blocking(move || store.set_settings(&settings)).await;
            if !matches!(saved, Ok(Ok(()))) {
                tracing::warn!("settings save failed");
            }
        });
    });

    // The View menu item names the side a click moves the inspector to.
    use_effect(move || {
        let dock = state.settings.read().inspector_dock;
        platform::set_dock_menu_label(dock);
    });

    // Theme and density ride on <html> so tokens.css media rules apply (D7).
    use_effect(move || {
        let settings = state.settings.read();
        let script = theme_script(settings.theme, settings.density, settings.text_scale);
        drop(settings);
        spawn(async move {
            let _ = document::eval(&script).await;
        });
    });

    rsx! {
        style { "{assets::FONTS_CSS}" }
        if crate::bridge::dev::active() {
            crate::bridge::dev::DevEcho {}
        } else {
            Shell {}
        }
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
                {crate::routes::overlay(overlay)}
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
    let dock = crate::ui::dock::use_dock_view(state);
    let layout = *dock.layout.read();
    // One body, drawn in the right column, as an overlay or at the bottom.
    let zone = crate::ui::drop_overlay::use_drop_zone(state);
    let hide_stream_tabs = layout.region == Some(RegionMode::Split);
    let pane = crate::routes::InspectorPane {
        pane_px: layout.pane_px,
        hide_stream_tabs,
    };
    let body = crate::routes::inspector_body(&state, pane);
    let right = match layout.slot {
        InspectorSlot::Docked(width) => Some((InspectorFrame::Docked, width)),
        InspectorSlot::Overlay(width) => Some((InspectorFrame::Overlay, width)),
        InspectorSlot::Bottom | InspectorSlot::Hidden => None,
    };
    let bottom_inspector = if layout.slot == InspectorSlot::Bottom {
        let split = layout.region == Some(RegionMode::Split);
        rsx! {
            Inspector {
                frame: InspectorFrame::Bottom,
                width_px: if split { layout.split_px },
                handle: rsx! { if split { InspectorSplitHandle {} } },
                {body.clone()}
            }
        }
    } else {
        rsx! {}
    };
    let (on_enter, on_over, on_leave, on_drop) =
        (zone.clone(), zone.clone(), zone.clone(), zone.clone());
    rsx! {
        div {
            class: "shell",
            tabindex: "-1",
            ondragenter: move |e| on_enter.enter(&e),
            ondragover: move |e| on_over.over(&e),
            ondragleave: move |_| on_leave.leave(),
            ondrop: move |e| on_drop.dropped(&e),
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
                            if let Some(dev) = try_consume_context::<crate::scenes::DevStatsPanel>() {
                                crate::ui::metric_tile::StatsPanelScene { id: dev.0 }
                            }
                            if let Some(dev) = try_consume_context::<crate::scenes::DevFilesPanel>() {
                                crate::ui::files_panel::FilesPanelScene { id: dev.0 }
                            }
                            {crate::routes::screen(*state.section.read(), *state.dev_gallery.read())}
                        }
                        if let Some((frame, width)) = right {
                            Inspector {
                                frame,
                                width_px: width,
                                handle: rsx! { InspectorWidthHandle {} },
                                {body}
                            }
                        }
                    }
                    BottomPanel { inspector: bottom_inspector }
                }
            }
            StatusBar {}
            OverlayStack {}
            crate::ui::drop_overlay::DropOverlay { hover: zone.hover() }
            crate::ui::ToastStack {}
        }
    }
}
