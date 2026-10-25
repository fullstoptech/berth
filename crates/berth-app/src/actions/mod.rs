//! The `ActionDef` table (PLAN §6.11): one table drives buttons,
//! shortcuts, the palette and menus. Covers every row of the PLAN §7.4
//! keyboard map and every visible button in brief §7.2. Coolify buttons
//! (Deploy, Cancel deployment…) are plugin-contributed (P9), not core.
//!
//! Each area (`global`, `container`, `image`…) owns one file with its
//! `DEFS`, the handlers `keys::dispatch` runs for them, the key chords
//! `keys::map_key` reads and the labels shown while they run. A new area
//! adds its file and one line to [`AREAS`]; nothing in `keys.rs` changes.
//!
//! Shortcut strings are unique across the table. Where one key means two
//! things by context (⌘. toggles Start/Stop, but stops the assistant turn
//! inside the Assistant tab, D77), only the primary action carries the
//! shortcut string; the chord tables do the contextual dispatch.

use std::sync::OnceLock;

pub use crate::state::ActionScope;
use crate::state::AppState;

mod assistant;
mod container;
mod engine;
mod global;
mod image;
mod logs;
mod network;
mod overlay;
mod project;
mod view;
mod volume;

/// An action's stable palette/menu id (an [`ActionDef::id`]).
pub type ActionId = &'static str;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDef {
    pub id: ActionId,
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub scope: ActionScope,
    pub danger: bool,
}

/// What `keys::dispatch` runs for an action id. Actions without a handler
/// are handled keys that change nothing (the webview must not reload on ⌘R).
pub type Handler = fn(&mut AppState, ActionId);

/// Everything one area contributes. Empty parts are left out with
/// `..Area::EMPTY`.
pub struct Area {
    pub defs: &'static [ActionDef],
    pub handlers: &'static [(ActionId, Handler)],
    pub chords: &'static [crate::keys::Chord],
    /// `(action id, label)` shown while the action runs (D34).
    pub pending: &'static [(&'static str, &'static str)],
}

impl Area {
    const EMPTY: Self = Self {
        defs: &[],
        handlers: &[],
        chords: &[],
        pending: &[],
    };
}

/// Every area, in palette order. One line per area.
const AREAS: &[&Area] = &[
    &global::AREA,
    &view::AREA,
    &overlay::AREA,
    &container::AREA,
    &project::AREA,
    &image::AREA,
    &volume::AREA,
    &network::AREA,
    &engine::AREA,
    &logs::AREA,
    &assistant::AREA,
];

const fn def(
    id: ActionId,
    label: &'static str,
    shortcut: Option<&'static str>,
    scope: ActionScope,
) -> ActionDef {
    ActionDef {
        id,
        label,
        shortcut,
        scope,
        danger: false,
    }
}

const fn danger(
    id: ActionId,
    label: &'static str,
    shortcut: Option<&'static str>,
    scope: ActionScope,
) -> ActionDef {
    ActionDef {
        id,
        label,
        shortcut,
        scope,
        danger: true,
    }
}

/// The union of every area's defs.
pub fn actions() -> &'static [ActionDef] {
    static ALL: OnceLock<Vec<ActionDef>> = OnceLock::new();
    ALL.get_or_init(|| {
        AREAS
            .iter()
            .flat_map(|area| area.defs.iter().copied())
            .collect()
    })
}

pub fn action_by_id(id: &str) -> Option<&'static ActionDef> {
    actions().iter().find(|a| a.id == id)
}

/// The handler registered for `id`, if its area has one.
pub fn handler(id: &str) -> Option<Handler> {
    AREAS
        .iter()
        .flat_map(|area| area.handlers.iter())
        .find(|(handled, _)| *handled == id)
        .map(|(_, handler)| *handler)
}

/// Every key chord of every area, in `AREAS` order.
pub fn chords() -> impl Iterator<Item = &'static crate::keys::Chord> {
    AREAS.iter().flat_map(|area| area.chords.iter())
}

/// The label shown while an action runs (D34).
#[must_use]
pub fn pending_label(id: &str) -> &'static str {
    AREAS
        .iter()
        .flat_map(|area| area.pending.iter())
        .find(|(pending, _)| *pending == id)
        .map_or("Working…", |(_, label)| label)
}

/// The Mod key's label (D40): the command symbol on macOS, "Ctrl" elsewhere.
pub fn mod_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_actions_every_shortcut_string_is_unique() {
        let mut seen: Vec<&str> = Vec::new();
        for action in actions() {
            let Some(shortcut) = action.shortcut else {
                continue;
            };
            assert!(
                !seen.contains(&shortcut),
                "shortcut {shortcut} used twice (second on {})",
                action.id
            );
            seen.push(shortcut);
        }
        assert!(seen.len() >= 25, "only {} shortcuts", seen.len());
    }

    #[test]
    fn services_actions_every_id_is_unique_and_labelled() {
        let mut seen: Vec<&str> = Vec::new();
        for action in actions() {
            assert!(!seen.contains(&action.id), "duplicate id {}", action.id);
            seen.push(action.id);
            assert!(!action.label.is_empty(), "{} has no label", action.id);
        }
    }

    #[test]
    fn services_actions_keyboard_map_rows_are_covered() {
        // Every PLAN §7.4 row maps to a def carrying its shortcut.
        for (shortcut, id) in [
            ("⌘K", "global.palette"),
            ("⌘1", "section.projects"),
            ("⌘2", "section.containers"),
            ("⌘3", "section.logs"),
            ("⌘4", "section.images"),
            ("⌘5", "section.volumes"),
            ("⌘6", "section.networks"),
            ("⌘7", "section.templates"),
            ("⌘8", "section.engines"),
            ("⌘9", "section.activity"),
            ("⌘⇧L", "global.logs_section"),
            ("⌘F", "global.focus_filter"),
            ("⌘O", "global.open_folder"),
            ("⌘N", "global.new_project"),
            ("⌘T", "container.terminal"),
            ("⌘L", "container.logs"),
            ("⌘E", "container.edit"),
            ("⌘R", "container.restart"),
            ("⌘.", "container.start_stop"),
            ("⌫", "container.delete"),
            ("Space", "global.toggle_inspector"),
            ("J", "global.select_next"),
            ("K", "global.select_prev"),
            ("⌃`", "global.toggle_bottom_panel"),
            ("F8", "logs.next_error"),
            ("⇧F8", "logs.prev_error"),
            ("⌘⇧.", "folders.toggle_hidden"),
            ("⌘⇧D", "view.dock_inspector"),
            ("Esc", "overlay.close"),
            ("⌘,", "global.settings"),
            ("↓", "palette.next"),
            ("↑", "palette.prev"),
            ("↵", "palette.run"),
        ] {
            let action = action_by_id(id).expect("def exists");
            assert_eq!(action.shortcut, Some(shortcut), "{id}");
        }
    }

    #[test]
    fn services_actions_visible_buttons_of_brief_72_are_covered() {
        for id in [
            // Container row + detail header.
            "container.start",
            "container.stop",
            "container.restart",
            "container.logs",
            "container.terminal",
            "container.delete",
            "container.pause",
            "container.resume",
            "container.kill",
            "container.edit",
            "container.duplicate",
            "container.recreate_latest",
            "container.rename",
            "container.save_as_image",
            // Project row + detail header.
            "project.start",
            "project.stop",
            "project.restart",
            "project.logs",
            "project.rebuild",
            "project.pull_images",
            "project.open_folder",
            "project.open_in_editor",
            "project.delete",
            // Bulk bar.
            "bulk.start",
            "bulk.stop",
            "bulk.restart",
            "bulk.pause",
            "bulk.delete",
            // Images, volumes, networks, engines.
            "image.run",
            "image.pull_latest",
            "image.tag",
            "image.push",
            "image.delete",
            "volume.browse",
            "volume.backup",
            "volume.delete",
            "network.connect_container",
            "network.disconnect",
            "network.delete",
            "engine.start",
            "engine.stop",
            "engine.restart",
            "engine.edit_resources",
            "engine.clean_up",
        ] {
            assert!(action_by_id(id).is_some(), "missing action {id}");
        }
        // Destructive styling per brief §7.2: Kill and Delete volume are danger.
        assert!(action_by_id("container.kill").expect("kill").danger);
        assert!(action_by_id("volume.delete").expect("voldel").danger);
        assert!(!action_by_id("container.stop").expect("stop").danger);
        assert!(!action_by_id("container.restart").expect("rst").danger);
    }

    #[test]
    fn services_actions_mod_label_is_the_platform_command_key() {
        if cfg!(target_os = "macos") {
            assert_eq!(mod_label(), "⌘");
        } else {
            assert_eq!(mod_label(), "Ctrl");
        }
    }

    #[test]
    fn services_actions_handlers_and_chords_name_real_actions_once() {
        let mut handled: Vec<&str> = Vec::new();
        for area in AREAS {
            for (id, _) in area.handlers {
                assert!(
                    action_by_id(id).is_some(),
                    "handler for unknown action {id}"
                );
                assert!(!handled.contains(id), "two handlers for {id}");
                handled.push(id);
            }
            for chord in area.chords {
                assert!(
                    action_by_id(chord.id).is_some(),
                    "chord for unknown action {}",
                    chord.id
                );
            }
        }
        assert!(handler("container.delete").is_some());
        assert!(handler("container.start").is_none());
    }

    #[test]
    fn services_actions_pending_labels_follow_the_action() {
        for (id, label) in [
            ("container.start", "Starting…"),
            ("project.start", "Starting…"),
            ("engine.start", "Starting…"),
            ("container.stop", "Stopping…"),
            ("engine.stop", "Stopping…"),
            ("container.restart", "Restarting…"),
            ("project.restart", "Restarting…"),
            ("container.pause", "Pausing…"),
            ("container.resume", "Resuming…"),
            ("container.kill", "Killing…"),
            ("container.delete", "Deleting…"),
            ("project.delete", "Deleting…"),
            ("image.delete", "Deleting…"),
            ("volume.delete", "Deleting…"),
            ("network.delete", "Deleting…"),
            ("volume.create", "Creating…"),
            ("network.create", "Creating…"),
            ("container.rename", "Renaming…"),
            ("network.connect_container", "Connecting…"),
            ("network.disconnect", "Disconnecting…"),
            ("container.apply_limits", "Applying…"),
            ("engine.edit_resources", "Applying…"),
            ("container.recreate", "Recreating…"),
            ("container.duplicate", "Duplicating…"),
            ("project.rebuild", "Rebuilding…"),
            ("project.pull_images", "Pulling…"),
            ("image.pull", "Working…"),
            ("no.such_action", "Working…"),
        ] {
            assert_eq!(pending_label(id), label, "{id}");
        }
    }
}
