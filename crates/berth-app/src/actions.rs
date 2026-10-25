//! The `ActionDef` table (PLAN §6.11): one table drives buttons,
//! shortcuts, the palette and menus. Covers every row of the PLAN §7.4
//! keyboard map and every visible button in brief §7.2. Coolify buttons
//! (Deploy, Cancel deployment…) are plugin-contributed (P9), not core.
//!
//! Shortcut strings are unique across the table. Where one key means two
//! things by context (⌘. toggles Start/Stop, but stops the assistant turn
//! inside the Assistant tab, D77), only the primary action carries the
//! shortcut string; `keys::map_key` does the contextual dispatch.

pub use crate::state::ActionScope;
use crate::state::ActionScope as S;

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

const ACTIONS: &[ActionDef] = &[
    // Global / shell (PLAN §7.4).
    def("global.palette", "Command palette", Some("⌘K"), S::Global),
    def("section.projects", "Projects", Some("⌘1"), S::Global),
    def("section.containers", "Containers", Some("⌘2"), S::Global),
    def("section.logs", "Logs", Some("⌘3"), S::Global),
    def("section.images", "Images", Some("⌘4"), S::Global),
    def("section.volumes", "Volumes", Some("⌘5"), S::Global),
    def("section.networks", "Networks", Some("⌘6"), S::Global),
    def("section.templates", "Templates", Some("⌘7"), S::Global),
    def("section.engines", "Engines", Some("⌘8"), S::Global),
    def("section.activity", "Activity", Some("⌘9"), S::Global),
    def(
        "global.logs_section",
        "Logs section",
        Some("⌘⇧L"),
        S::Global,
    ),
    def(
        "global.focus_filter",
        "Filter current list",
        Some("⌘F"),
        S::Global,
    ),
    def("global.open_folder", "Open folder…", Some("⌘O"), S::Global),
    def("global.new_project", "New project", Some("⌘N"), S::Global),
    def("global.settings", "Settings", Some("⌘,"), S::Global),
    def(
        "global.toggle_inspector",
        "Toggle inspector",
        Some("Space"),
        S::Global,
    ),
    def(
        "global.toggle_bottom_panel",
        "Toggle bottom panel",
        Some("⌃`"),
        S::Global,
    ),
    def("global.select_next", "Select next", Some("J"), S::Global),
    def(
        "global.select_prev",
        "Select previous",
        Some("K"),
        S::Global,
    ),
    // View > text size (owner request 2026-10-25). Bigger also answers to
    // ⌘+ and ⌘⇧= (`keys::map_key`); only the ⌘= string is carried here.
    def("view.text_bigger", "Bigger text", Some("⌘="), S::Global),
    def("view.text_smaller", "Smaller text", Some("⌘-"), S::Global),
    def("view.text_reset", "Actual text size", Some("⌘0"), S::Global),
    def("overlay.close", "Close", Some("Esc"), S::Global),
    def("palette.next", "Next result", Some("↓"), S::Global),
    def("palette.prev", "Previous result", Some("↑"), S::Global),
    def("palette.run", "Run selected", Some("↵"), S::Global),
    def(
        "folders.toggle_hidden",
        "Show hidden files",
        Some("⌘⇧."),
        S::Global,
    ),
    // Container (brief §7.2 row + detail header).
    def("container.start", "Start", None, S::Container),
    def("container.stop", "Stop", None, S::Container),
    def(
        "container.start_stop",
        "Start/Stop",
        Some("⌘."),
        S::Container,
    ),
    def("container.restart", "Restart", Some("⌘R"), S::Container),
    def("container.pause", "Pause", None, S::Container),
    def("container.resume", "Resume", None, S::Container),
    danger("container.kill", "Kill", None, S::Container),
    danger("container.delete", "Delete", Some("⌫"), S::Container),
    def("container.logs", "Logs", Some("⌘L"), S::Container),
    def("container.terminal", "Terminal", Some("⌘T"), S::Container),
    def(
        "container.edit",
        "Edit and recreate",
        Some("⌘E"),
        S::Container,
    ),
    def("container.duplicate", "Duplicate", None, S::Container),
    def(
        "container.recreate_latest",
        "Recreate with latest image",
        None,
        S::Container,
    ),
    def("container.rename", "Rename", None, S::Container),
    def(
        "container.save_as_image",
        "Save as image",
        None,
        S::Container,
    ),
    // Bulk bar (brief §7.2); labels gain counts at render time.
    def("bulk.start", "Start selected", None, S::Container),
    def("bulk.stop", "Stop selected", None, S::Container),
    def("bulk.restart", "Restart selected", None, S::Container),
    def("bulk.pause", "Pause selected", None, S::Container),
    danger("bulk.delete", "Delete selected", None, S::Container),
    // Project (brief §7.2 row + detail header).
    def("project.start", "Start project", None, S::Project),
    def("project.stop", "Stop project", None, S::Project),
    def("project.start_stop", "Start/Stop project", None, S::Project),
    def("project.restart", "Restart project", None, S::Project),
    def("project.rebuild", "Rebuild", None, S::Project),
    def("project.pull_images", "Pull images", None, S::Project),
    def("project.logs", "Logs", None, S::Project),
    def("project.open_folder", "Open folder", None, S::Project),
    def("project.open_in_editor", "Open in editor", None, S::Project),
    danger("project.delete", "Delete project", None, S::Project),
    // Image (brief §7.2).
    def("image.run", "Run", None, S::Image),
    def("image.pull", "Pull image", None, S::Image),
    def("image.pull_latest", "Pull latest", None, S::Image),
    def("image.tag", "Tag", None, S::Image),
    def("image.push", "Push", None, S::Image),
    danger("image.delete", "Delete", None, S::Image),
    // Volume (brief §7.2).
    def("volume.browse", "Browse", None, S::Volume),
    def("volume.backup", "Back up", None, S::Volume),
    def("volume.create", "Create volume", None, S::Volume),
    danger("volume.delete", "Delete", None, S::Volume),
    // Network (brief §7.2).
    def(
        "network.connect_container",
        "Connect container",
        None,
        S::Network,
    ),
    def("network.disconnect", "Disconnect", None, S::Network),
    def("network.create", "Create network", None, S::Network),
    danger("network.delete", "Delete", None, S::Network),
    // Engine (brief §7.2 row + dashboard; menu bar reuses these).
    def("engine.start", "Start engine", None, S::Engine),
    def("engine.stop", "Stop engine", None, S::Engine),
    def("engine.start_stop", "Start/Stop engine", None, S::Engine),
    def("engine.restart", "Restart engine", None, S::Engine),
    def("engine.edit_resources", "Edit resources", None, S::Engine),
    def("engine.clean_up", "Clean up", None, S::Engine),
    def("engine.add", "Add engine", None, S::Engine),
    // Logs view (PLAN §7.4 F8; brief §7.3 Logs actions).
    def("logs.next_error", "Next error", Some("F8"), S::Logs),
    def("logs.prev_error", "Previous error", Some("⇧F8"), S::Logs),
    def("logs.follow", "Follow", None, S::Logs),
    def("logs.explain", "Explain", None, S::Logs),
    def("logs.ask", "Ask about these lines", None, S::Logs),
    def("logs.copy", "Copy", None, S::Logs),
    def("logs.save", "Save logs", None, S::Logs),
    // Assistant (D77: ⌘. here, but container.start_stop carries the string).
    def(
        "assistant.stop_turn",
        "Stop the assistant turn",
        None,
        S::Assistant,
    ),
    def("assistant.approve_tool", "Approve", None, S::Assistant),
    def("assistant.deny_tool", "Deny", None, S::Assistant),
];

pub fn actions() -> &'static [ActionDef] {
    ACTIONS
}

pub fn action_by_id(id: &str) -> Option<&'static ActionDef> {
    ACTIONS.iter().find(|a| a.id == id)
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
}
