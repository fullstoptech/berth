//! Keyboard handling (PLAN §7.4): a pure mapping from
//! `(key, modifiers, context)` to an [`ActionId`], unit-tested without a
//! DOM, plus the one `onkeydown` handler the shell root attaches.
//!
//! `key` is the `KeyboardEvent.key` string ("k", "1", "`", " ", "Escape",
//! "Backspace", "F8", "ArrowDown", "Enter", …) — the `Display` of
//! `dioxus::html::Key`.
//!
//! Dioxus `KeyboardData` carries no target element, so the page reports
//! focus itself: [`use_focus_tracking`] installs a `focusin`/`focusout`
//! listener that sends what holds focus, and [`bare_keys_allowed`] is the one
//! rule that turns it into "may a bare key (Space, J, K, ⌫…) be a shortcut
//! now". The root handler reads the answer; nothing else decides it.

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
    /// Focus sits on a control that owns bare keys ([`bare_keys_allowed`]
    /// said no): bare-key chords do not fire, ⌘ chords still do.
    pub bare_keys_blocked: bool,
    pub in_assistant: bool,
    pub palette_open: bool,
    pub overlay_open: bool,
    pub in_folder_browser: bool,
    pub in_log_view: bool,
    pub selection: SelectionKind,
}

/// Which modifier keys a chord needs. `shift` and `alt` of `None` mean
/// either state is fine.
#[derive(Debug, Clone, Copy)]
pub struct ModMatch {
    meta: bool,
    ctrl: bool,
    shift: Option<bool>,
    alt: Option<bool>,
}

impl ModMatch {
    fn matches(self, mods: Modifiers) -> bool {
        self.meta == mods.meta
            && self.ctrl == mods.ctrl
            && self.shift.is_none_or(|shift| shift == mods.shift)
            && self.alt.is_none_or(|alt| alt == mods.alt)
    }
}

/// One key press an action answers to. Areas list their chords next to their
/// `ActionDef`s (`actions/<area>.rs`); [`map_key`] finds the first chord that
/// matches. Use the constructors, then [`Chord::when`] for a context.
#[derive(Clone, Copy)]
pub struct Chord {
    /// The `KeyboardEvent.key` strings that match ("k" and "K").
    pub keys: &'static [&'static str],
    pub id: ActionId,
    mods: ModMatch,
    /// Fires while a control owns the bare keys (⌘ chords do; bare keys never do).
    typing: bool,
    when: fn(&KeyContext) -> bool,
}

fn always(_: &KeyContext) -> bool {
    true
}

impl Chord {
    const fn new(
        keys: &'static [&'static str],
        id: ActionId,
        mods: ModMatch,
        typing: bool,
    ) -> Self {
        Self {
            keys,
            id,
            mods,
            typing,
            when: always,
        }
    }

    /// ⌘ plus the key, nothing else held.
    pub const fn cmd(keys: &'static [&'static str], id: ActionId) -> Self {
        let mods = ModMatch {
            meta: true,
            ctrl: false,
            shift: Some(false),
            alt: Some(false),
        };
        Self::new(keys, id, mods, true)
    }

    /// ⌘⇧ plus the key.
    pub const fn cmd_shift(keys: &'static [&'static str], id: ActionId) -> Self {
        let mods = ModMatch {
            meta: true,
            ctrl: false,
            shift: Some(true),
            alt: Some(false),
        };
        Self::new(keys, id, mods, true)
    }

    /// ⌃ plus the key, with or without Shift and Alt.
    pub const fn ctrl(keys: &'static [&'static str], id: ActionId) -> Self {
        let mods = ModMatch {
            meta: false,
            ctrl: true,
            shift: None,
            alt: None,
        };
        Self::new(keys, id, mods, true)
    }

    /// A bare key with no modifier. Never fires while typing.
    pub const fn bare(keys: &'static [&'static str], id: ActionId) -> Self {
        let mods = ModMatch {
            meta: false,
            ctrl: false,
            shift: Some(false),
            alt: Some(false),
        };
        Self::new(keys, id, mods, false)
    }

    /// A bare key that may carry Shift ("j" and "J"). Never fires while typing.
    pub const fn bare_either_shift(keys: &'static [&'static str], id: ActionId) -> Self {
        let mods = ModMatch {
            meta: false,
            ctrl: false,
            shift: None,
            alt: Some(false),
        };
        Self::new(keys, id, mods, false)
    }

    /// A function key with Shift held or not; it fires from a text input.
    pub const fn function(keys: &'static [&'static str], shift: bool, id: ActionId) -> Self {
        let mods = ModMatch {
            meta: false,
            ctrl: false,
            shift: Some(shift),
            alt: Some(false),
        };
        Self::new(keys, id, mods, true)
    }

    /// Only while `when` holds.
    #[must_use]
    pub const fn when(mut self, when: fn(&KeyContext) -> bool) -> Self {
        self.when = when;
        self
    }

    fn matches(&self, key: &str, mods: Modifiers, ctx: &KeyContext) -> bool {
        self.keys.contains(&key)
            && self.mods.matches(mods)
            && (self.typing || !ctx.bare_keys_blocked)
            && (self.when)(ctx)
    }
}

/// Map one key press to an action, or `None` when Berth does not handle it
/// (the caller must not `preventDefault` then).
pub fn map_key(key: &str, mods: Modifiers, ctx: KeyContext) -> Option<ActionId> {
    // The palette owns navigation keys while it is open (its input is a
    // text field, so this precedes the focus guard).
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

    // Everything else is a chord an area registered next to its actions.
    crate::actions::chords()
        .find(|chord| chord.matches(key, mods, &ctx))
        .map(|chord| chord.id)
}

/// The context the root handler derives from state and from what holds
/// focus.
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
        bare_keys_blocked: crate::focus::bare_keys_blocked(),
        in_assistant,
        palette_open,
        overlay_open,
        in_folder_browser,
        in_log_view,
        selection,
    }
}

/// Run the handler the action's area registered (`actions/<area>.rs`).
/// Actions without one are handled keys that change nothing (so the
/// webview never reloads on ⌘R).
pub fn dispatch(state: &mut AppState, action: ActionId) {
    if let Some(handler) = crate::actions::handler(action) {
        handler(state, action);
    }
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
    use crate::focus::{bare_keys_allowed, FocusInfo};

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
            bare_keys_blocked: true,
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
            bare_keys_blocked: true,
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
            bare_keys_blocked: true,
            ..ctx()
        };
        assert_eq!(
            map_key("=", Modifiers::META, typing),
            Some("view.text_bigger")
        );
    }

    #[test]
    fn space_j_k_and_backspace_wait_while_a_control_holds_focus() {
        let blocked = KeyContext {
            bare_keys_blocked: !bare_keys_allowed(&FocusInfo {
                tag: "button".to_string(),
                ..FocusInfo::default()
            }),
            selection: SelectionKind::Container,
            ..ctx()
        };
        for key in [" ", "j", "k", "J", "Backspace"] {
            assert_eq!(map_key(key, Modifiers::NONE, blocked), None, "{key:?}");
        }
        let free = KeyContext {
            bare_keys_blocked: !bare_keys_allowed(&FocusInfo::body()),
            selection: SelectionKind::Container,
            ..ctx()
        };
        assert_eq!(
            map_key(" ", Modifiers::NONE, free),
            Some("global.toggle_inspector")
        );
        assert_eq!(
            map_key("j", Modifiers::NONE, free),
            Some("global.select_next")
        );
        // Escape and ⌘ chords are not bare keys: they work from a button too.
        assert_eq!(
            map_key("r", Modifiers::META, blocked),
            Some("container.restart")
        );
        let overlay = KeyContext {
            overlay_open: true,
            ..blocked
        };
        assert_eq!(
            map_key("Escape", Modifiers::NONE, overlay),
            Some("overlay.close")
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

    #[test]
    fn dock_shortcut_maps_and_flips_the_inspector_between_the_two_sides() {
        use berth_store::InspectorDock::{Bottom, Right};
        let shift_meta = Modifiers {
            meta: true,
            shift: true,
            ..Modifiers::NONE
        };
        for key in ["D", "d"] {
            assert_eq!(
                map_key(key, shift_meta, KeyContext::default()),
                Some("view.dock_inspector")
            );
        }
        // ⌘D alone and ⇧D alone are not the dock key.
        assert_eq!(map_key("d", Modifiers::META, KeyContext::default()), None);
        assert_eq!(map_key("D", Modifiers::SHIFT, KeyContext::default()), None);

        let mut dom = VirtualDom::new(Harness);
        let _ = dom.rebuild_to_vec();
        let mut state = SLOT
            .with(|slot| slot.borrow_mut().take())
            .expect("harness captured AppState");
        dom.in_scope(ScopeId::APP, || {
            let dock = |state: &AppState| state.settings.peek().inspector_dock;
            assert_eq!(dock(&state), Right);
            dispatch(&mut state, "view.dock_inspector");
            assert_eq!(dock(&state), Bottom);
            dispatch(&mut state, "view.dock_inspector");
            assert_eq!(dock(&state), Right);
        });
    }
}
