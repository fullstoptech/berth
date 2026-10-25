//! The command palette and global search (P2-T9, frame "Palette / open /
//! 1440"). Search itself lives in `services::search`; this screen owns the
//! query, the active row and the keys. The query input is uncontrolled
//! (D59): the field is seeded once and remounted to clear it.

use dioxus::html::ModifiersInteraction;
use dioxus::prelude::*;

use crate::services::actions::ActionsService;
use crate::services::search::{self, Index, RecentKey, Results, SearchItem};
use crate::services::Services;
use crate::state::{AppState, Overlay};
use crate::ui::{Icon, IconSize, PaletteItem};

/// Shown on the disabled "Run as docker command?" row until P3-T6 builds
/// terminal tabs.
pub const DOCKER_COMMAND_REASON: &str = "Terminal tabs are not built yet";

thread_local! {
    static INITIAL_QUERY: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Seed the next palette with a query (scenes put the palette into a
/// frame's state, D58). The palette reads it on open and keeps it.
pub fn set_initial_query(query: &str) {
    INITIAL_QUERY.with(|cell| *cell.borrow_mut() = query.to_string());
}

fn initial_query() -> String {
    INITIAL_QUERY.with(|cell| cell.borrow().clone())
}

/// What a key press means inside the palette's input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteKey {
    Next,
    Prev,
    Run,
    /// Esc: clears the query first, closes when it is already empty
    /// (PLAN §7.4).
    ClearOrClose,
    /// ⌘K again closes the palette.
    Close,
    /// A Cmd/Ctrl chord the shell should still see (⌘1, ⌘,).
    Shell,
    /// Plain typing: the input keeps it and the shell never sees it.
    Typing,
}

#[must_use]
pub fn palette_key(key: &str, meta_or_ctrl: bool) -> PaletteKey {
    match key {
        "ArrowDown" => PaletteKey::Next,
        "ArrowUp" => PaletteKey::Prev,
        "Enter" => PaletteKey::Run,
        "Escape" => PaletteKey::ClearOrClose,
        "k" | "K" if meta_or_ctrl => PaletteKey::Close,
        _ if meta_or_ctrl => PaletteKey::Shell,
        _ => PaletteKey::Typing,
    }
}

/// Move the active row by `delta`, wrapping at both ends.
#[must_use]
pub fn move_active(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let len = isize::try_from(len).unwrap_or(isize::MAX);
    let current = isize::try_from(current).unwrap_or(0).min(len - 1);
    usize::try_from((current + delta).rem_euclid(len)).unwrap_or(0)
}

/// "⌘R" as written in the `ActionDef` table, in this platform's Mod key.
#[must_use]
pub fn shortcut_text(shortcut: &str) -> String {
    let modifier = crate::actions::mod_label();
    if modifier == "⌘" {
        shortcut.to_string()
    } else {
        shortcut.replace('⌘', &format!("{modifier}+"))
    }
}

fn close_palette(state: &mut AppState) {
    state
        .overlays
        .write()
        .retain(|o| !matches!(o, Overlay::Palette));
}

#[component]
pub fn PaletteScreen() -> Element {
    let state = use_context::<AppState>();
    let actions = try_use_context::<ActionsService>();
    let services = try_use_context::<Services>();

    let seed = use_hook(initial_query);
    let mut query = use_signal(|| seed.clone());
    let mut active = use_signal(|| 0usize);
    let mut epoch = use_signal(|| 0u32);
    let mut stored_recents = use_signal(Vec::<RecentKey>::new);

    // Recents survive restarts through the activity log: the actions the
    // user ran from Berth, read once when the palette opens.
    use_hook(move || {
        let Some(services) = services else { return };
        let store = std::sync::Arc::clone(&services.store);
        spawn(async move {
            let load = move || {
                store
                    .activity_query(
                        &berth_store::ActivityFilter {
                            origin: Some("berth".to_string()),
                            ..Default::default()
                        },
                        None,
                        64,
                    )
                    .map(|entries| search::recents_from_activity(&entries))
                    .unwrap_or_default()
            };
            let keys = if tokio::runtime::Handle::try_current().is_ok() {
                tokio::task::spawn_blocking(load).await.unwrap_or_default()
            } else {
                load()
            };
            stored_recents.set(keys);
        });
    });

    // Rebuilt when the engine state or the selection changes; reading the
    // signals here is what subscribes the memo to them.
    let index = use_memo(move || {
        let engines = state.engines.read();
        let containers = state.containers.read();
        let projects = state.projects.read();
        let images = state.images.read();
        let volumes = state.volumes.read();
        let networks = state.networks.read();
        let selection = state.selection.read();
        let scope = state.engine_scope.read();
        let pending = state.pending.read();
        Index::build(
            &crate::services::actions::StateView {
                engines: &engines,
                containers: &containers,
                projects: &projects,
                images: &images,
                volumes: &volumes,
                networks: &networks,
                pending: &pending,
            },
            &selection,
            &scope,
        )
    });
    let results = use_memo(move || {
        let mut recents = search::session_recents();
        for key in stored_recents.read().iter() {
            if !recents.contains(key) {
                recents.push(key.clone());
            }
        }
        index.read().search(&query.read(), &recents)
    });

    let run = use_callback(move |item: SearchItem| {
        let mut state = state;
        search::remember_in_session(item.recent_key());
        // Close first so a dialog or sheet the action opens lands on top of
        // the shell, not of the palette.
        close_palette(&mut state);
        search::run(&mut state, actions.as_ref(), &item);
    });

    // Keep the active row visible while arrow keys walk a long list.
    use_effect(move || {
        let row = active();
        spawn(async move {
            let _ = document::eval(&format!(
                "document.getElementById('palette-item-{row}')?.scrollIntoView({{block: 'nearest'}});"
            ))
            .await;
        });
    });

    // The input held focus; hand it back to the shell root on every way of
    // closing, or the shortcuts (keys.rs listens on that element) go deaf.
    use_drop(|| {
        let _ = document::eval("document.querySelector('.shell')?.focus();");
    });

    let snapshot: Results = results.read().clone();
    let flat_len = snapshot.flat().len();
    let active_row = active().min(flat_len.saturating_sub(1));
    let current_query = query.read().clone();
    let show_clear = !current_query.is_empty();

    let mut row = 0usize;
    rsx! {
        div {
            class: "palette",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Command palette",
            onclick: move |e| e.stop_propagation(),
            div { class: "palette-head",
                div { class: "search-field palette-field",
                    span { class: "field-prefix",
                        Icon { name: "search", size: IconSize::Sm }
                    }
                    // A lone keyed element that must remount sits in a
                    // one-item loop (Dioxus rule): clearing bumps the epoch.
                    for generation in [epoch()] {
                        input {
                            key: "{generation}",
                            class: "search-input palette-input",
                            r#type: "text",
                            autocomplete: "off",
                            spellcheck: "false",
                            initial_value: "{seed_for(generation, &seed)}",
                            placeholder: "Search or run a command\u{2026}",
                            "aria-label": "Search or run a command",
                            id: "palette-input",
                            // Focus with the caret after any seeded text (a
                            // plain focus() would leave it selected).
                            onmounted: move |_| {
                                spawn(async move {
                                    let _ = document::eval(
                                        "const el = document.getElementById('palette-input'); \
                                         if (el) { el.focus(); el.setSelectionRange(el.value.length, el.value.length); }",
                                    )
                                    .await;
                                });
                            },
                            oninput: move |e| {
                                query.set(e.value());
                                active.set(0);
                            },
                            onkeydown: move |e| {
                                let mods = e.data.modifiers();
                                let chord = mods.contains(dioxus::html::Modifiers::META)
                                    || mods.contains(dioxus::html::Modifiers::CONTROL);
                                let key = e.data.key().to_string();
                                match palette_key(&key, chord) {
                                    PaletteKey::Next => {
                                        e.prevent_default();
                                        e.stop_propagation();
                                        active.set(move_active(active_row, flat_len, 1));
                                    }
                                    PaletteKey::Prev => {
                                        e.prevent_default();
                                        e.stop_propagation();
                                        active.set(move_active(active_row, flat_len, -1));
                                    }
                                    PaletteKey::Run => {
                                        e.prevent_default();
                                        e.stop_propagation();
                                        let item = results.read().flat().get(active_row).map(|i| (*i).clone());
                                        if let Some(item) = item {
                                            run.call(item);
                                        }
                                    }
                                    PaletteKey::ClearOrClose => {
                                        e.prevent_default();
                                        e.stop_propagation();
                                        if query.peek().is_empty() {
                                            let mut state = state;
                                            close_palette(&mut state);
                                        } else {
                                            query.set(String::new());
                                            active.set(0);
                                            epoch += 1;
                                        }
                                    }
                                    PaletteKey::Close => {
                                        e.prevent_default();
                                        e.stop_propagation();
                                        let mut state = state;
                                        close_palette(&mut state);
                                    }
                                    PaletteKey::Typing => e.stop_propagation(),
                                    PaletteKey::Shell => {}
                                }
                            },
                        }
                    }
                    if show_clear {
                        button {
                            class: "search-clear",
                            r#type: "button",
                            "aria-label": "Clear search",
                            onclick: move |_| {
                                query.set(String::new());
                                active.set(0);
                                epoch += 1;
                            },
                            Icon { name: "x", size: IconSize::Sm }
                        }
                    }
                }
            }
            div { class: "palette-body", role: "listbox", "aria-label": "Results",
                for group in snapshot.groups.iter() {
                    div { class: "palette-group", key: "{group.group.title()}",
                        div { class: "palette-group-title", "{group.group.title()}" }
                        for item in group.items.iter() {
                            {
                                let index_of_row = row;
                                row += 1;
                                let picked = item.clone();
                                rsx! {
                                    PaletteItem {
                                        key: "{index_of_row}",
                                        id: "palette-item-{index_of_row}",
                                        icon: item.icon,
                                        title: item.title.clone(),
                                        meta: item.meta.clone(),
                                        shortcut: item.shortcut.map(shortcut_text),
                                        active: index_of_row == active_row,
                                        danger: item.danger,
                                        onhover: move |_| {
                                            if *active.peek() != index_of_row {
                                                active.set(index_of_row);
                                            }
                                        },
                                        onclick: move |_| run.call(picked.clone()),
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(typed) = snapshot.docker_command.as_ref() {
                    div { class: "palette-group",
                        div { class: "palette-group-title", "No match" }
                        PaletteItem {
                            id: "palette-item-docker",
                            icon: "terminal",
                            title: "Run as docker command?",
                            meta: "docker {typed}",
                            disabled_reason: DOCKER_COMMAND_REASON,
                        }
                    }
                }
            }
            div { class: "palette-foot",
                span { class: "palette-hint",
                    kbd { class: "kbd", "aria-label": "Up arrow",
                        Icon { name: "arrow-up", size: IconSize::Sm }
                    }
                    kbd { class: "kbd", "aria-label": "Down arrow",
                        Icon { name: "arrow-down", size: IconSize::Sm }
                    }
                    "Navigate"
                }
                span { class: "palette-hint",
                    kbd { class: "kbd", "aria-label": "Return",
                        Icon { name: "corner-down-left", size: IconSize::Sm }
                    }
                    "Run"
                }
                span { class: "palette-hint",
                    kbd { class: "kbd", "Esc" }
                    "Close"
                }
            }
        }
    }
}

/// The input's seed: the opening query the first time, empty after a clear.
fn seed_for(generation: u32, seed: &str) -> String {
    if generation == 0 {
        seed.to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_keys_map_navigation_run_and_escape() {
        assert_eq!(palette_key("ArrowDown", false), PaletteKey::Next);
        assert_eq!(palette_key("ArrowUp", false), PaletteKey::Prev);
        assert_eq!(palette_key("Enter", false), PaletteKey::Run);
        assert_eq!(palette_key("Escape", false), PaletteKey::ClearOrClose);
        // Typing never reaches the shell (J, K, Space would otherwise act).
        assert_eq!(palette_key("j", false), PaletteKey::Typing);
        assert_eq!(palette_key(" ", false), PaletteKey::Typing);
        // ⌘K again closes; other chords stay the shell's.
        assert_eq!(palette_key("k", true), PaletteKey::Close);
        assert_eq!(palette_key("1", true), PaletteKey::Shell);
    }

    #[test]
    fn palette_active_row_wraps_at_both_ends() {
        assert_eq!(move_active(0, 3, 1), 1);
        assert_eq!(move_active(2, 3, 1), 0);
        assert_eq!(move_active(0, 3, -1), 2);
        assert_eq!(move_active(0, 0, 1), 0);
        // A stale index after the list shrank still lands inside it.
        assert_eq!(move_active(9, 3, 1), 0);
    }

    #[test]
    fn palette_scene_renders_the_frame_rows_in_order() {
        let html = crate::scenes::render_scene_html("Palette / open / 1440").expect("scene");
        let at = |needle: &str| {
            html.find(needle)
                .unwrap_or_else(|| panic!("{needle} missing from {html}"))
        };
        // Groups and rows of the frame, top to bottom.
        let containers = at("palette-group-title\">Containers");
        let worker = at("Restart shop-api-worker-1");
        let projects = at("palette-group-title\">Projects");
        let project = at("Restart project shop-api");
        let engine_group = at("palette-group-title\">Engine");
        let engine = at("Restart engine default");
        assert!(containers < worker && worker < projects && projects < project);
        assert!(project < engine_group && engine_group < engine);
        assert!(html.contains("Container · Restarting"));
        assert!(html.contains("5 containers"));
        assert!(html.contains("Asks to confirm · stops 15 containers"));
        // The first row is active and carries its shortcut; the input holds
        // the frame's query and the footer shows the key hints.
        assert!(html.contains("palette-item palette-item-active"));
        assert!(html.contains("⌘R") || html.contains("Ctrl+R"));
        assert!(html.contains("initial_value=\"rest\""), "{html}");
        for hint in ["Navigate", "Run", "Close"] {
            assert!(html.contains(hint), "{hint}");
        }
        // Something found: no docker-command fallback.
        assert!(!html.contains("Run as docker command?"));
    }

    #[test]
    fn palette_no_match_shows_the_docker_row_disabled_with_its_reason() {
        #[component]
        fn Root() -> Element {
            let state = crate::state::use_app_state();
            use_context_provider(|| state);
            rsx! { PaletteScreen {} }
        }
        set_initial_query("zzzqqq");
        let mut dom = VirtualDom::new(Root);
        let _ = dom.rebuild_to_vec();
        let html = dioxus_ssr::render(&dom);
        set_initial_query("");
        assert!(html.contains("Run as docker command?"), "{html}");
        assert!(html.contains("docker zzzqqq"));
        assert!(html.contains("palette-item-disabled"));
        assert!(html.contains(DOCKER_COMMAND_REASON));
        assert!(html.contains("aria-disabled=\"true\""));
    }

    #[test]
    fn palette_shortcut_text_uses_the_platform_mod_key() {
        if cfg!(target_os = "macos") {
            assert_eq!(shortcut_text("⌘R"), "⌘R");
        } else {
            assert_eq!(shortcut_text("⌘R"), "Ctrl+R");
        }
        assert_eq!(shortcut_text("Esc"), "Esc");
    }
}
