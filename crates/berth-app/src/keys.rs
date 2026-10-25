//! Keyboard handling (PLAN §7.4): a pure mapping from
//! `(key, modifiers, context)` to an [`ActionId`], unit-tested without a
//! DOM, plus the one `onkeydown` handler the shell root attaches.
//!
//! `key` is the `KeyboardEvent.key` string ("k", "1", "`", " ", "Escape",
//! "Backspace", "F8", "ArrowDown", "Enter", …) — the `Display` of
//! `dioxus::html::Key`.
//!
//! Limitation: Dioxus `KeyboardData` carries no target element, so the
//! root handler cannot know focus sits in a text input. It passes
//! `in_text_input: false`; text inputs must stop keydown propagation
//! themselves (P2-T2 `TextField`) so J/K/Space/⌫ typing never reaches here.
//! The pure mapping still honours the flag for callers that can know.

use dioxus::html::ModifiersInteraction;
use dioxus::prelude::*;

use crate::actions::ActionId;
use crate::state::{AppState, BottomTab, Overlay, Section, Selection};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub meta: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        meta: false,
        ctrl: false,
        shift: false,
        alt: false,
    };
    pub const META: Self = Self {
        meta: true,
        ctrl: false,
        shift: false,
        alt: false,
    };
    pub const CTRL: Self = Self {
        meta: false,
        ctrl: true,
        shift: false,
        alt: false,
    };
    pub const SHIFT: Self = Self {
        meta: false,
        ctrl: false,
        shift: true,
        alt: false,
    };

    fn any(&self) -> bool {
        self.meta || self.ctrl || self.shift || self.alt
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SelectionKind {
    #[default]
    None,
    Container,
    Project,
    Image,
    Volume,
    Network,
    Engine,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyContext {
    pub in_text_input: bool,
    pub in_assistant: bool,
    pub palette_open: bool,
    pub overlay_open: bool,
    pub in_folder_browser: bool,
    pub in_log_view: bool,
    pub selection: SelectionKind,
}

const SECTIONS: [&str; 9] = [
    "section.projects",
    "section.containers",
    "section.logs",
    "section.images",
    "section.volumes",
    "section.networks",
    "section.templates",
    "section.engines",
    "section.activity",
];

/// Map one key press to an action, or `None` when Berth does not handle it
/// (the caller must not `preventDefault` then).
pub fn map_key(key: &str, mods: Modifiers, ctx: KeyContext) -> Option<ActionId> {
    // The palette owns navigation keys while it is open (its input is a
    // text field, so this precedes the text-input guard).
    if ctx.palette_open {
        match key {
            "ArrowDown" => return Some("palette.next"),
            "ArrowUp" => return Some("palette.prev"),
            "Enter" => return Some("palette.run"),
            "Escape" => return Some("overlay.close"),
            _ => {}
        }
    }

    if key == "Escape" {
        if ctx.overlay_open || ctx.palette_open {
            return Some("overlay.close");
        }
        return None;
    }

    // ⌃` toggles the bottom panel, even from a text input.
    if mods.ctrl && !mods.meta && key == "`" {
        return Some("global.toggle_bottom_panel");
    }

    if mods.meta && !mods.ctrl && !mods.alt {
        if mods.shift {
            return match key {
                // ⌘⇧= types "+" on most layouts; same as ⌘+.
                "+" | "=" => Some("view.text_bigger"),
                "L" | "l" => Some("global.logs_section"),
                "." if ctx.in_folder_browser => Some("folders.toggle_hidden"),
                _ => None,
            };
        }
        return match key {
            "k" | "K" => Some("global.palette"),
            "f" | "F" => Some("global.focus_filter"),
            "o" | "O" => Some("global.open_folder"),
            "n" | "N" => Some("global.new_project"),
            "t" | "T" => Some("container.terminal"),
            "l" | "L" => Some("container.logs"),
            "e" | "E" => Some("container.edit"),
            "r" | "R" => Some("container.restart"),
            "," => Some("global.settings"),
            "=" | "+" => Some("view.text_bigger"),
            "-" | "_" => Some("view.text_smaller"),
            "0" => Some("view.text_reset"),
            "." => {
                if ctx.in_assistant {
                    Some("assistant.stop_turn") // D77
                } else {
                    match ctx.selection {
                        SelectionKind::Container => Some("container.start_stop"),
                        SelectionKind::Project => Some("project.start_stop"),
                        SelectionKind::Engine => Some("engine.start_stop"),
                        _ => None,
                    }
                }
            }
            digit if digit.len() == 1 => digit
                .chars()
                .next()
                .and_then(|c| c.to_digit(10))
                .and_then(|d| usize::try_from(d).ok())
                .and_then(|d| d.checked_sub(1))
                .and_then(|i| SECTIONS.get(i).copied()),
            _ => None,
        };
    }

    if key == "F8" && !mods.meta && !mods.ctrl && !mods.alt {
        if !ctx.in_log_view {
            return None;
        }
        return Some(if mods.shift {
            "logs.prev_error"
        } else {
            "logs.next_error"
        });
    }

    // Bare keys never fire while typing.
    if ctx.in_text_input {
        return None;
    }

    match key {
        "Backspace" if !mods.any() => match ctx.selection {
            SelectionKind::Container => Some("container.delete"),
            SelectionKind::Project => Some("project.delete"),
            SelectionKind::Image => Some("image.delete"),
            SelectionKind::Volume => Some("volume.delete"),
            SelectionKind::Network => Some("network.delete"),
            SelectionKind::None | SelectionKind::Engine => None,
        },
        " " if !mods.any() => Some("global.toggle_inspector"),
        "j" | "J" if !mods.meta && !mods.ctrl && !mods.alt => Some("global.select_next"),
        "k" | "K" if !mods.meta && !mods.ctrl && !mods.alt => Some("global.select_prev"),
        _ => None,
    }
}

/// The context the root handler derives from state. `in_text_input` stays
/// false (see the module note).
pub fn context_from_state(state: &AppState) -> KeyContext {
    let overlays = state.overlays.peek();
    let palette_open = overlays.iter().any(|o| matches!(o, Overlay::Palette));
    let in_folder_browser = overlays.iter().any(|o| matches!(o, Overlay::OpenFolder));
    let overlay_open = !overlays.is_empty();
    drop(overlays);
    let bottom = state.bottom.peek();
    let in_assistant = bottom.open && bottom.tab == BottomTab::Assistant;
    let in_log_view =
        *state.section.peek() == Section::Logs || (bottom.open && bottom.tab == BottomTab::Logs);
    drop(bottom);
    let selection = match &*state.selection.peek() {
        Selection::None => SelectionKind::None,
        Selection::Containers(ids) if ids.is_empty() => SelectionKind::None,
        Selection::Containers(_) => SelectionKind::Container,
        Selection::Project(_) => SelectionKind::Project,
        Selection::Image(_) => SelectionKind::Image,
        Selection::Volume(_) => SelectionKind::Volume,
        Selection::Network(_) => SelectionKind::Network,
        Selection::Engine(_) => SelectionKind::Engine,
    };
    KeyContext {
        in_text_input: false,
        in_assistant,
        palette_open,
        overlay_open,
        in_folder_browser,
        in_log_view,
        selection,
    }
}

/// Apply the navigation/chrome subset of actions straight to state.
/// Effectful actions (restart, delete, terminal…) dispatch through the
/// actions service from P2-T5 on; until then they are still handled keys
/// (so the webview never reloads on ⌘R), they just change nothing yet.
pub fn dispatch(state: &mut AppState, action: ActionId) {
    match action {
        "global.palette" => state.overlays.write().push(Overlay::Palette),
        "overlay.close" => {
            state.overlays.write().pop();
        }
        "global.open_folder" => state.overlays.write().push(Overlay::OpenFolder),
        "global.new_project" => state.overlays.write().push(Overlay::NewProject),
        "global.settings" => state.section.set(Section::Settings),
        "view.text_bigger" => step_text_scale(state, crate::text_scale::Step::Bigger),
        "view.text_smaller" => step_text_scale(state, crate::text_scale::Step::Smaller),
        "view.text_reset" => step_text_scale(state, crate::text_scale::Step::Reset),
        "global.logs_section" | "section.logs" => state.section.set(Section::Logs),
        "section.projects" => state.section.set(Section::Projects),
        "section.containers" => state.section.set(Section::Containers),
        "section.images" => state.section.set(Section::Images),
        "section.volumes" => state.section.set(Section::Volumes),
        "section.networks" => state.section.set(Section::Networks),
        "section.templates" => state.section.set(Section::Templates),
        "section.engines" => state.section.set(Section::Engines),
        "section.activity" => state.section.set(Section::Activity),
        "global.toggle_bottom_panel" => {
            let mut bottom = state.bottom.write();
            bottom.open = !bottom.open;
        }
        "container.logs" => {
            let mut bottom = state.bottom.write();
            bottom.open = true;
            bottom.tab = BottomTab::Logs;
        }
        // ⌘T and the row's Terminal button (P3-T6).
        "container.terminal" => crate::services::exec::dispatch_open(state),
        // F8 and Shift+F8 reach the active log view (P3-T2).
        "logs.next_error" | "logs.prev_error" => crate::ui::log_view::dispatch(action),
        // ⌘E opens the edit and recreate sheet of the selected container (P3-T7).
        "container.edit" => crate::sheets::edit_recreate::open_for_selection(state),
        // ⌫ on the selected containers asks first (D33, P2-T8).
        "container.delete" | "bulk.delete" => open_delete_for_selection(state),
        // J/K and Space act on the visible list (P2-T7).
        "global.select_next" | "global.select_prev" | "global.toggle_inspector" => {
            crate::screens::containers::dispatch(state, action);
        }
        _ => {}
    }
}

/// Move the text size one step; the App root applies and saves it.
fn step_text_scale(state: &mut AppState, step: crate::text_scale::Step) {
    let current = state.settings.peek().text_scale;
    let next = crate::text_scale::next_scale(current, step);
    if (next - current).abs() > f32::EPSILON {
        state.settings.write().text_scale = next;
    }
}

/// Open the delete dialog for the selected containers that can be
/// deleted. Only on the Containers screen and never over another overlay.
fn open_delete_for_selection(state: &mut AppState) {
    if *state.section.peek() != Section::Containers || !state.overlays.peek().is_empty() {
        return;
    }
    let ids = match &*state.selection.peek() {
        Selection::Containers(ids) => ids.clone(),
        _ => return,
    };
    let deletable = crate::services::actions::with_view(state, |view| {
        crate::ui::bulk_bar::applicable(&ids, crate::ui::BulkAction::Delete, view)
    });
    crate::services::confirm::open_delete(state, &deletable);
}

/// The one root `onkeydown` handler: map, `preventDefault` when handled,
/// dispatch. Attach to the shell root element only.
pub fn handle_root_keydown(state: &mut AppState, event: &Event<KeyboardData>) {
    let mods = event.data.modifiers();
    let modifiers = Modifiers {
        meta: mods.contains(dioxus::html::Modifiers::META),
        ctrl: mods.contains(dioxus::html::Modifiers::CONTROL),
        shift: mods.contains(dioxus::html::Modifiers::SHIFT),
        alt: mods.contains(dioxus::html::Modifiers::ALT),
    };
    let key = event.data.key().to_string();
    let ctx = context_from_state(state);
    if let Some(action) = map_key(&key, modifiers, ctx) {
        event.prevent_default();
        dispatch(state, action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> KeyContext {
        KeyContext::default()
    }

    #[test]
    fn services_keys_cmd_k_opens_the_palette() {
        assert_eq!(map_key("k", Modifiers::META, ctx()), Some("global.palette"));
        // Bare k is selection movement, not the palette.
        assert_eq!(
            map_key("k", Modifiers::NONE, ctx()),
            Some("global.select_prev")
        );
        assert_eq!(map_key("q", Modifiers::NONE, ctx()), None);
    }

    #[test]
    fn services_keys_cmd_digits_switch_sections_in_sidebar_order() {
        assert_eq!(
            map_key("1", Modifiers::META, ctx()),
            Some("section.projects")
        );
        assert_eq!(
            map_key("2", Modifiers::META, ctx()),
            Some("section.containers")
        );
        assert_eq!(
            map_key("9", Modifiers::META, ctx()),
            Some("section.activity")
        );
        // ⌘0 is "Actual text size", not a section.
        assert_eq!(
            map_key("0", Modifiers::META, ctx()),
            Some("view.text_reset")
        );
    }

    #[test]
    fn services_keys_ctrl_backtick_toggles_the_bottom_panel() {
        assert_eq!(
            map_key("`", Modifiers::CTRL, ctx()),
            Some("global.toggle_bottom_panel")
        );
        assert_eq!(map_key("`", Modifiers::NONE, ctx()), None);
    }

    #[test]
    fn services_keys_j_k_and_space_move_and_toggle() {
        assert_eq!(
            map_key("j", Modifiers::NONE, ctx()),
            Some("global.select_next")
        );
        assert_eq!(
            map_key("k", Modifiers::NONE, ctx()),
            Some("global.select_prev")
        );
        assert_eq!(
            map_key("J", Modifiers::SHIFT, ctx()),
            Some("global.select_next")
        );
        assert_eq!(
            map_key(" ", Modifiers::NONE, ctx()),
            Some("global.toggle_inspector")
        );
    }

    #[test]
    fn services_keys_bare_keys_never_fire_while_typing() {
        let typing = KeyContext {
            in_text_input: true,
            selection: SelectionKind::Container,
            ..ctx()
        };
        assert_eq!(map_key("j", Modifiers::NONE, typing), None);
        assert_eq!(map_key(" ", Modifiers::NONE, typing), None);
        assert_eq!(map_key("Backspace", Modifiers::NONE, typing), None);
        // …but ⌘K still works from an input.
        assert_eq!(
            map_key("k", Modifiers::META, typing),
            Some("global.palette")
        );
    }

    #[test]
    fn services_keys_cmd_dot_is_contextual_per_d77() {
        let assistant = KeyContext {
            in_assistant: true,
            selection: SelectionKind::Container,
            ..ctx()
        };
        assert_eq!(
            map_key(".", Modifiers::META, assistant),
            Some("assistant.stop_turn")
        );
        let container = KeyContext {
            selection: SelectionKind::Container,
            ..ctx()
        };
        assert_eq!(
            map_key(".", Modifiers::META, container),
            Some("container.start_stop")
        );
        assert_eq!(map_key(".", Modifiers::META, ctx()), None);
    }

    #[test]
    fn services_keys_backspace_deletes_by_selection_kind() {
        for (selection, expected) in [
            (SelectionKind::Container, Some("container.delete")),
            (SelectionKind::Project, Some("project.delete")),
            (SelectionKind::Image, Some("image.delete")),
            (SelectionKind::Volume, Some("volume.delete")),
            (SelectionKind::Network, Some("network.delete")),
            (SelectionKind::Engine, None),
            (SelectionKind::None, None),
        ] {
            let c = KeyContext { selection, ..ctx() };
            assert_eq!(
                map_key("Backspace", Modifiers::NONE, c),
                expected,
                "{selection:?}"
            );
        }
    }

    #[test]
    fn services_keys_palette_owns_arrows_enter_and_escape() {
        let palette = KeyContext {
            palette_open: true,
            overlay_open: true,
            in_text_input: true,
            ..ctx()
        };
        assert_eq!(
            map_key("ArrowDown", Modifiers::NONE, palette),
            Some("palette.next")
        );
        assert_eq!(
            map_key("ArrowUp", Modifiers::NONE, palette),
            Some("palette.prev")
        );
        assert_eq!(
            map_key("Enter", Modifiers::NONE, palette),
            Some("palette.run")
        );
        assert_eq!(
            map_key("Escape", Modifiers::NONE, palette),
            Some("overlay.close")
        );
        // Escape with nothing open is not ours to preventDefault.
        assert_eq!(map_key("Escape", Modifiers::NONE, ctx()), None);
        let overlay = KeyContext {
            overlay_open: true,
            ..ctx()
        };
        assert_eq!(
            map_key("Escape", Modifiers::NONE, overlay),
            Some("overlay.close")
        );
    }

    #[test]
    fn services_keys_f8_needs_a_log_view_and_shift_goes_backwards() {
        assert_eq!(map_key("F8", Modifiers::NONE, ctx()), None);
        let logs = KeyContext {
            in_log_view: true,
            ..ctx()
        };
        assert_eq!(
            map_key("F8", Modifiers::NONE, logs),
            Some("logs.next_error")
        );
        assert_eq!(
            map_key("F8", Modifiers::SHIFT, logs),
            Some("logs.prev_error")
        );
    }

    #[test]
    fn services_keys_remaining_cmd_rows_of_the_map() {
        for (key, mods, expected) in [
            ("f", Modifiers::META, "global.focus_filter"),
            ("o", Modifiers::META, "global.open_folder"),
            ("n", Modifiers::META, "global.new_project"),
            ("t", Modifiers::META, "container.terminal"),
            ("l", Modifiers::META, "container.logs"),
            ("e", Modifiers::META, "container.edit"),
            ("r", Modifiers::META, "container.restart"),
            (",", Modifiers::META, "global.settings"),
            (
                "L",
                Modifiers {
                    meta: true,
                    shift: true,
                    ..Modifiers::NONE
                },
                "global.logs_section",
            ),
        ] {
            assert_eq!(map_key(key, mods, ctx()), Some(expected), "{key}");
        }
        // ⌘⇧. only means something in the folder browser.
        let meta_shift = Modifiers {
            meta: true,
            shift: true,
            ..Modifiers::NONE
        };
        assert_eq!(map_key(".", meta_shift, ctx()), None);
        let browser = KeyContext {
            in_folder_browser: true,
            ..ctx()
        };
        assert_eq!(
            map_key(".", meta_shift, browser),
            Some("folders.toggle_hidden")
        );
    }

    #[test]
    fn text_size_keys_map_to_bigger_smaller_and_actual() {
        let shift = Modifiers {
            meta: true,
            shift: true,
            ..Modifiers::NONE
        };
        for (key, mods, expected) in [
            ("=", Modifiers::META, "view.text_bigger"),
            ("+", Modifiers::META, "view.text_bigger"),
            // ⌘⇧= is the same chord as ⌘+ on a US keyboard.
            ("+", shift, "view.text_bigger"),
            ("=", shift, "view.text_bigger"),
            ("-", Modifiers::META, "view.text_smaller"),
            ("0", Modifiers::META, "view.text_reset"),
        ] {
            assert_eq!(map_key(key, mods, ctx()), Some(expected), "{key}");
        }
        // Bare keys and ⌃ chords are not text size.
        assert_eq!(map_key("=", Modifiers::NONE, ctx()), None);
        assert_eq!(map_key("-", Modifiers::CTRL, ctx()), None);
        // ⌘0 does not collide with ⌘1..⌘9.
        assert_eq!(
            map_key("1", Modifiers::META, ctx()),
            Some("section.projects")
        );
        // They work from a text input too.
        let typing = KeyContext {
            in_text_input: true,
            ..ctx()
        };
        assert_eq!(
            map_key("=", Modifiers::META, typing),
            Some("view.text_bigger")
        );
    }

    thread_local! {
        static SLOT: std::cell::RefCell<Option<AppState>> =
            const { std::cell::RefCell::new(None) };
    }

    #[component]
    fn Harness() -> Element {
        let state = crate::state::use_app_state();
        use_hook(move || SLOT.with(|slot| *slot.borrow_mut() = Some(state)));
        rsx! { div {} }
    }

    #[test]
    fn text_size_dispatch_steps_the_setting_and_stops_at_the_ends() {
        let mut dom = VirtualDom::new(Harness);
        let _ = dom.rebuild_to_vec();
        let mut state = SLOT
            .with(|slot| slot.borrow_mut().take())
            .expect("harness captured AppState");
        dom.in_runtime(|| {
            let scale = |state: &AppState| state.settings.peek().text_scale;
            assert_eq!(scale(&state), 1.0);
            dispatch(&mut state, "view.text_bigger");
            assert_eq!(scale(&state), 1.1);
            dispatch(&mut state, "view.text_bigger");
            dispatch(&mut state, "view.text_bigger");
            assert_eq!(scale(&state), 1.3);
            dispatch(&mut state, "view.text_reset");
            assert_eq!(scale(&state), 1.0);
            for _ in 0..5 {
                dispatch(&mut state, "view.text_smaller");
            }
            assert_eq!(scale(&state), 0.85);
            // A stored value that is not a step moves from its nearest step.
            state.settings.write().text_scale = 1.12;
            dispatch(&mut state, "view.text_bigger");
            assert_eq!(scale(&state), 1.2);
        });
    }
}
