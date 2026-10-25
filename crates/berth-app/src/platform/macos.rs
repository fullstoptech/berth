use dioxus::desktop::muda::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use dioxus::desktop::{Config, LogicalPosition, LogicalSize, WindowBuilder};
use dioxus::prelude::use_hook;

use crate::actions::ActionId;
use crate::assets;
use crate::state::AppState;

const TRAFFIC_LIGHT_INSET: (f64, f64) = (12.0, 19.0);

pub fn config() -> Config {
    use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;
    let window = WindowBuilder::new()
        .with_title("Berth")
        .with_inner_size(LogicalSize::new(
            super::window_size().0,
            super::window_size().1,
        ))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0))
        .with_titlebar_transparent(true)
        .with_fullsize_content_view(true)
        .with_title_hidden(true);
    Config::new()
        .with_window(window)
        .with_menu(build_menu())
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

// ---------------------------------------------------------------------
// App menu (D39). `menu_spec` is the data; `build_menu` turns it into
// muda objects on the main thread; `menu_action` maps a clicked item id
// back to what Berth does. Edit carries the predefined items so Cmd+C,
// V, X, A and Z reach text inputs in WKWebView.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Predefined {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    CloseWindow,
    Fullscreen,
    Minimize,
    Zoom,
    BringAllToFront,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// A Berth action: `id` is its menu id; `accelerator` in muda syntax.
    Item {
        id: &'static str,
        label: &'static str,
        accelerator: Option<&'static str>,
    },
    Predefined(Predefined),
    Separator,
}

struct SubmenuSpec {
    title: &'static str,
    entries: Vec<Entry>,
}

const QUIT_ID: &str = "app.quit";
const HELP_SHORTCUTS_ID: &str = "help.shortcuts";

const fn item(id: &'static str, label: &'static str, accelerator: Option<&'static str>) -> Entry {
    Entry::Item {
        id,
        label,
        accelerator,
    }
}

fn menu_spec() -> Vec<SubmenuSpec> {
    use Entry::{Predefined as P, Separator as Sep};
    use Predefined as K;
    vec![
        SubmenuSpec {
            title: "Berth",
            entries: vec![
                P(K::About),
                Sep,
                item("global.settings", "Settings\u{2026}", Some("Cmd+,")),
                Sep,
                P(K::Services),
                Sep,
                P(K::Hide),
                P(K::HideOthers),
                P(K::ShowAll),
                Sep,
                // Not the predefined Quit: `NSApp terminate` exits the
                // process without dropping the app, so the final
                // snapshot save (D23) would never run.
                item(QUIT_ID, "Quit Berth", Some("Cmd+Q")),
            ],
        },
        SubmenuSpec {
            title: "File",
            entries: vec![
                item("global.open_folder", "Open Folder\u{2026}", Some("Cmd+O")),
                item("global.new_project", "New Project\u{2026}", Some("Cmd+N")),
                Sep,
                P(K::CloseWindow),
            ],
        },
        SubmenuSpec {
            title: "Edit",
            entries: vec![
                P(K::Undo),
                P(K::Redo),
                Sep,
                P(K::Cut),
                P(K::Copy),
                P(K::Paste),
                P(K::SelectAll),
            ],
        },
        SubmenuSpec {
            title: "View",
            entries: vec![
                item("global.palette", "Command Palette", Some("Cmd+K")),
                item(
                    "global.toggle_bottom_panel",
                    "Toggle Bottom Panel",
                    Some("Ctrl+`"),
                ),
                item("global.toggle_inspector", "Toggle Inspector", None),
                Sep,
                P(K::Fullscreen),
            ],
        },
        SubmenuSpec {
            title: "Window",
            entries: vec![P(K::Minimize), P(K::Zoom), Sep, P(K::BringAllToFront)],
        },
        SubmenuSpec {
            title: "Help",
            entries: vec![item(HELP_SHORTCUTS_ID, "Keyboard Shortcuts", None)],
        },
    ]
}

/// What a clicked menu item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Run an `ActionDef` through `keys::dispatch`.
    Dispatch(ActionId),
    /// Close the window so the app drops cleanly (final snapshot save).
    Quit,
}

pub fn menu_action(id: &str) -> Option<MenuAction> {
    match id {
        QUIT_ID => Some(MenuAction::Quit),
        HELP_SHORTCUTS_ID => Some(MenuAction::Dispatch("global.palette")),
        other => menu_spec()
            .into_iter()
            .flat_map(|s| s.entries)
            .find_map(|e| match e {
                Entry::Item { id, .. } if id == other => {
                    crate::actions::action_by_id(id).map(|def| MenuAction::Dispatch(def.id))
                }
                _ => None,
            }),
    }
}

fn predefined(kind: Predefined) -> PredefinedMenuItem {
    match kind {
        Predefined::About => PredefinedMenuItem::about(
            Some("About Berth"),
            Some(AboutMetadata {
                name: Some("Berth".to_string()),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                ..AboutMetadata::default()
            }),
        ),
        Predefined::Services => PredefinedMenuItem::services(None),
        Predefined::Hide => PredefinedMenuItem::hide(None),
        Predefined::HideOthers => PredefinedMenuItem::hide_others(None),
        Predefined::ShowAll => PredefinedMenuItem::show_all(None),
        Predefined::Undo => PredefinedMenuItem::undo(None),
        Predefined::Redo => PredefinedMenuItem::redo(None),
        Predefined::Cut => PredefinedMenuItem::cut(None),
        Predefined::Copy => PredefinedMenuItem::copy(None),
        Predefined::Paste => PredefinedMenuItem::paste(None),
        Predefined::SelectAll => PredefinedMenuItem::select_all(None),
        Predefined::CloseWindow => PredefinedMenuItem::close_window(None),
        Predefined::Fullscreen => PredefinedMenuItem::fullscreen(None),
        Predefined::Minimize => PredefinedMenuItem::minimize(None),
        Predefined::Zoom => PredefinedMenuItem::maximize(Some("Zoom")),
        Predefined::BringAllToFront => PredefinedMenuItem::bring_all_to_front(None),
    }
}

/// Build the menu bar. muda requires the main thread: call from `main`
/// through `config()`.
fn build_menu() -> Menu {
    let menu = Menu::new();
    for spec in menu_spec() {
        let submenu = Submenu::new(spec.title, true);
        for entry in spec.entries {
            let added = match entry {
                Entry::Item {
                    id,
                    label,
                    accelerator,
                } => {
                    let accelerator = accelerator.and_then(|a| a.parse().ok());
                    submenu.append(&MenuItem::with_id(id, label, true, accelerator))
                }
                Entry::Predefined(kind) => submenu.append(&predefined(kind)),
                Entry::Separator => submenu.append(&PredefinedMenuItem::separator()),
            };
            if let Err(err) = added {
                tracing::warn!(%err, menu = spec.title, "menu item failed");
            }
        }
        match spec.title {
            "Window" => submenu.set_as_windows_menu_for_nsapp(),
            "Help" => submenu.set_as_help_menu_for_nsapp(),
            _ => {}
        }
        if let Err(err) = menu.append(&submenu) {
            tracing::warn!(%err, menu = spec.title, "menu failed");
        }
    }
    menu
}

/// Route clicks on Berth's own menu items (everything but the predefined
/// ones) to the same dispatcher the keyboard uses.
pub fn use_menu_events(state: AppState) {
    dioxus::desktop::use_muda_event_handler(move |event| {
        let mut state = state;
        match menu_action(&event.id().0) {
            Some(MenuAction::Dispatch(action)) => crate::keys::dispatch(&mut state, action),
            Some(MenuAction::Quit) => dioxus::desktop::window().close(),
            None => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_menu_bar_has_the_six_menus_of_d39() {
        let titles: Vec<_> = menu_spec().iter().map(|s| s.title).collect();
        assert_eq!(titles, ["Berth", "File", "Edit", "View", "Window", "Help"]);
    }

    #[test]
    fn shell_menu_edit_has_every_predefined_clipboard_item() {
        let spec = menu_spec();
        let edit = spec.iter().find(|s| s.title == "Edit").expect("Edit menu");
        for kind in [
            Predefined::Undo,
            Predefined::Redo,
            Predefined::Cut,
            Predefined::Copy,
            Predefined::Paste,
            Predefined::SelectAll,
        ] {
            assert!(
                edit.entries.contains(&Entry::Predefined(kind)),
                "{kind:?} missing from Edit"
            );
        }
    }

    /// "Cmd+K" -> "⌘K": the glyph form the ActionDef table uses.
    fn glyphs(accelerator: &str) -> String {
        accelerator
            .replace("Cmd+", "\u{2318}")
            .replace("Ctrl+", "\u{2303}")
    }

    #[test]
    fn shell_menu_items_are_real_actions_with_the_actions_shortcuts() {
        for spec in menu_spec() {
            for entry in spec.entries {
                let Entry::Item {
                    id, accelerator, ..
                } = entry
                else {
                    continue;
                };
                if id == QUIT_ID || id == HELP_SHORTCUTS_ID {
                    continue;
                }
                let def = crate::actions::action_by_id(id)
                    .unwrap_or_else(|| panic!("menu item {id} is not an ActionDef"));
                if let Some(accelerator) = accelerator {
                    assert!(
                        accelerator
                            .parse::<dioxus::desktop::muda::accelerator::Accelerator>()
                            .is_ok(),
                        "{accelerator} must parse"
                    );
                    assert_eq!(def.shortcut, Some(glyphs(accelerator).as_str()), "{id}");
                }
            }
        }
    }

    #[test]
    fn shell_menu_quit_closes_the_window_and_items_dispatch() {
        assert_eq!(menu_action(QUIT_ID), Some(MenuAction::Quit));
        assert_eq!(
            menu_action("global.palette"),
            Some(MenuAction::Dispatch("global.palette"))
        );
        assert_eq!(
            menu_action(HELP_SHORTCUTS_ID),
            Some(MenuAction::Dispatch("global.palette"))
        );
        assert_eq!(menu_action("not.an.item"), None);
    }
}
