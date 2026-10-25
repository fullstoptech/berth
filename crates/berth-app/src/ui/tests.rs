use super::*;

fn render(element: Element) -> String {
    dioxus_ssr::render_element(element)
}

// ---------- Button ----------

#[test]
fn button_primary_md() {
    let html = render(rsx! { Button { label: "Stop container", variant: ButtonVariant::Primary } });
    assert!(html.contains("btn-primary"), "{html}");
    assert!(html.contains("size-md"));
    assert!(html.contains("Stop container"));
    assert!(!html.contains("disabled"));
}

#[test]
fn button_secondary_is_default_variant() {
    let html = render(rsx! { Button { label: "Cancel" } });
    assert!(html.contains("btn-secondary"), "{html}");
    assert!(html.contains("Cancel"));
}

#[test]
fn button_ghost_sm() {
    let html = render(
        rsx! { Button { label: "Logs", variant: ButtonVariant::Ghost, size: ControlSize::Sm } },
    );
    assert!(html.contains("btn-ghost"));
    assert!(html.contains("size-sm"));
}

#[test]
fn button_danger_lg() {
    let html = render(
        rsx! { Button { label: "Delete", variant: ButtonVariant::Danger, size: ControlSize::Lg } },
    );
    assert!(html.contains("btn-danger"));
    assert!(html.contains("size-lg"));
}

#[test]
fn button_disabled_has_attr_and_reason_tooltip() {
    let html = render(rsx! {
        Button { label: "Stop", disabled: true, disabled_reason: "Container is not running" }
    });
    assert!(html.contains("disabled"), "{html}");
    assert!(html.contains("Container is not running"));
}

#[test]
fn button_loading_keeps_label_and_spins() {
    let html = render(rsx! { Button { label: "Stopping\u{2026}", loading: true } });
    assert!(html.contains("Stopping\u{2026}"));
    assert!(html.contains("/icons.svg#loader"), "{html}");
    assert!(html.contains("btn-spinner"));
    assert!(html.contains("disabled"));
}

#[test]
fn button_with_icon_and_kbd() {
    let html = render(rsx! { Button { label: "Restart", icon: "rotate-cw", kbd_letter: "R" } });
    assert!(html.contains("/icons.svg#rotate-cw"));
    assert!(html.contains("data-symbol=\"cmd\""), "{html}");
    assert!(html.contains(">R<") || html.contains("R"));
}

// ---------- IconButton ----------

#[test]
fn icon_button_requires_accessible_name() {
    let html = render(rsx! { IconButton { icon: "rotate-cw", label: "Restart api-db-1" } });
    assert!(html.contains("aria-label=\"Restart api-db-1\""), "{html}");
    assert!(html.contains("/icons.svg#rotate-cw"));
    assert!(html.contains("icon-btn-neutral"));
}

#[test]
fn icon_button_danger_tone() {
    let html = render(rsx! {
        IconButton { icon: "trash-2", label: "Delete api-db-1", tone: IconButtonTone::Danger }
    });
    assert!(html.contains("icon-btn-danger"));
    assert!(html.contains("aria-label=\"Delete api-db-1\""));
}

#[test]
fn icon_button_tooltip_text() {
    let html = render(rsx! {
        IconButton { icon: "rotate-cw", label: "Restart api-db-1", tooltip: "Restart \u{00b7} \u{2318}R" }
    });
    assert!(html.contains("data-tooltip"), "{html}");
    assert!(html.contains("Restart"));
}

#[test]
fn icon_button_disabled() {
    let html =
        render(rsx! { IconButton { icon: "play", label: "Start api-db-1", disabled: true } });
    assert!(html.contains("disabled"));
}

#[test]
fn icon_button_loading_swaps_icon() {
    let html = render(rsx! { IconButton { icon: "play", label: "Start api-db-1", loading: true } });
    assert!(html.contains("/icons.svg#loader"));
    assert!(!html.contains("/icons.svg#play"));
}

#[test]
fn icon_button_pressed_on_state() {
    let html =
        render(rsx! { IconButton { icon: "wrap-text", label: "Wrap lines", pressed: true } });
    assert!(html.contains("icon-btn-on"), "{html}");
    assert!(html.contains("aria-pressed=\"true\""));
}

#[test]
fn icon_button_sm_size() {
    let html = render(rsx! { IconButton { icon: "x", label: "Close", size: ControlSize::Sm } });
    assert!(html.contains("size-sm"));
}

// ---------- Kbd / Icon ----------

#[test]
fn kbd_cmd_renders_command_symbol() {
    let html = render(rsx! { Kbd { symbol: KbdSymbol::Cmd } });
    assert!(html.contains("\u{2318}"), "{html}"); // ⌘ in the accessible name
    assert!(html.contains("/icons.svg#command"));
    assert!(html.contains("<kbd"));
}

#[test]
fn kbd_letter() {
    let html = render(rsx! { Kbd { symbol: KbdSymbol::Letter("K".to_string()) } });
    assert!(html.contains("K"));
    assert!(html.contains("data-symbol=\"letter\""));
}

#[test]
fn kbd_shift_and_backspace_icons() {
    let shift = render(rsx! { Kbd { symbol: KbdSymbol::Shift } });
    assert!(shift.contains("/icons.svg#shift"));
    let back = render(rsx! { Kbd { symbol: KbdSymbol::Backspace } });
    assert!(back.contains("/icons.svg#backspace"));
    let ret = render(rsx! { Kbd { symbol: KbdSymbol::Return } });
    assert!(ret.contains("/icons.svg#corner-down-left"));
}

#[test]
fn kbd_shortcut_pair() {
    let html = render(rsx! { KbdShortcut { letter: "K" } });
    assert!(html.contains("/icons.svg#command"));
    assert!(html.contains("K"));
}

#[test]
fn icon_uses_sprite_use_element() {
    let html = render(rsx! { Icon { name: "terminal" } });
    assert!(html.contains("<use"), "{html}");
    assert!(html.contains("/icons.svg#terminal"));
    assert!(html.contains("icon-md"));
}

#[test]
fn icon_sm_size() {
    let html = render(rsx! { Icon { name: "play", size: IconSize::Sm } });
    assert!(html.contains("icon-sm"));
}

// ---------- Status ----------

#[test]
fn status_dot_tone_for_every_kind() {
    for status in all_statuses() {
        let html = render(rsx! { StatusDot { status } });
        let tone = tone_class(status.tone());
        assert!(html.contains(tone), "{status:?} missing {tone}: {html}");
        assert!(html.contains(&status.label()), "{status:?}: {html}");
    }
}

#[test]
fn status_dot_ring_for_starting_and_restarting() {
    for status in [
        berth_core::StatusKind::Starting,
        berth_core::StatusKind::Restarting,
    ] {
        let html = render(rsx! { StatusDot { status } });
        assert!(html.contains("status-dot-ring"), "{status:?}: {html}");
        // The frames draw two arcs, not two vertical bars.
        assert_eq!(html.matches("<path").count(), 2, "{status:?}: {html}");
        assert!(html.contains("status-dot-arcs"), "{status:?}: {html}");
    }
}

#[test]
fn status_dot_other_states_draw_no_arcs() {
    for status in all_statuses() {
        if matches!(
            status,
            berth_core::StatusKind::Starting | berth_core::StatusKind::Restarting
        ) {
            continue;
        }
        let html = render(rsx! { StatusDot { status } });
        assert!(!html.contains("<svg"), "{status:?}: {html}");
    }
}

#[test]
fn shared_dot_draws_the_same_arcs_for_a_restarting_service() {
    use crate::ui::{project_row::dot_for_status, Dot};
    let spec = dot_for_status(berth_core::StatusKind::Restarting);
    let html = render(rsx! { Dot { spec } });
    assert_eq!(html.matches("<path").count(), 2, "{html}");
    let spec = dot_for_status(berth_core::StatusKind::Running);
    let html = render(rsx! { Dot { spec } });
    assert!(!html.contains("<svg"), "{html}");
}

#[test]
fn status_dot_hollow_for_paused() {
    let html = render(rsx! { StatusDot { status: berth_core::StatusKind::Paused } });
    assert!(html.contains("status-dot-hollow"));
}

#[test]
fn status_badge_running_success_with_label() {
    let html = render(rsx! { StatusBadge { status: berth_core::StatusKind::Running } });
    assert!(html.contains("status-badge"));
    assert!(html.contains("tone-success"));
    assert!(html.contains("Running"));
}

#[test]
fn status_badge_exited_codes_and_tones() {
    let ok = render(rsx! { StatusBadge { status: berth_core::StatusKind::Exited(0) } });
    assert!(ok.contains("Exited (0)"));
    assert!(ok.contains("tone-stopped"));
    let err = render(rsx! { StatusBadge { status: berth_core::StatusKind::Exited(137) } });
    assert!(err.contains("Exited (137)"));
    assert!(err.contains("tone-danger"));
}

#[test]
fn status_badge_created_is_info() {
    let html = render(rsx! { StatusBadge { status: berth_core::StatusKind::Created } });
    assert!(html.contains("tone-info"));
    assert!(html.contains("Created"));
}

#[test]
fn status_badge_warning_family() {
    for status in [
        berth_core::StatusKind::Unhealthy,
        berth_core::StatusKind::Restarting,
        berth_core::StatusKind::Paused,
    ] {
        let html = render(rsx! { StatusBadge { status } });
        assert!(html.contains("tone-warning"), "{status:?}: {html}");
        assert!(html.contains(&status.label()));
    }
}

#[test]
fn badge_kinds() {
    let count = render(rsx! { Badge { kind: BadgeKind::Count, text: "14" } });
    assert!(count.contains("badge-count") && count.contains("14"));
    let tag = render(rsx! { Badge { kind: BadgeKind::Tag, text: "latest" } });
    assert!(tag.contains("badge-tag") && tag.contains("latest"));
    let arch = render(rsx! { Badge { kind: BadgeKind::Arch, text: "amd64" } });
    assert!(arch.contains("badge-arch") && arch.contains("amd64"));
}

// ---------- TextField / SearchField ----------

#[test]
fn text_field_label_help_placeholder() {
    let html = render(rsx! {
        TextField { label: "Container name", id: "n1", placeholder: "e.g. web-1", help: "Letters and dashes." }
    });
    assert!(html.contains("Container name"));
    assert!(html.contains("Letters and dashes."));
    assert!(html.contains("e.g. web-1"));
    assert!(html.contains("for=\"n1\"") || html.contains("n1"), "{html}");
}

#[test]
fn text_field_is_uncontrolled_initial_value() {
    // D59: the seed renders as the input's initial value, not a bound value.
    let html = render(rsx! { TextField { label: "Name", initial_value: "shop-api-cache-1" } });
    assert!(html.contains("shop-api-cache-1"), "{html}");
}

#[test]
fn text_field_error_replaces_help() {
    let html = render(rsx! {
        TextField { label: "Name", help: "help text", error: "Name is already used" }
    });
    assert!(html.contains("Name is already used"));
    assert!(!html.contains("help text"));
    assert!(html.contains("text-field-error"));
    assert!(html.contains("role=\"alert\""));
}

#[test]
fn text_field_disabled() {
    let html = render(rsx! { TextField { label: "Name", disabled: true } });
    assert!(html.contains("disabled"));
}

#[test]
fn text_field_prefix_icon_and_kbd_suffix() {
    let icon = render(rsx! { TextField { label: "Name", prefix_icon: "search" } });
    assert!(icon.contains("/icons.svg#search"));
    let kbd = render(rsx! { TextField { label: "Name", kbd: KbdSymbol::Return } });
    assert!(kbd.contains("data-symbol=\"return\""), "{kbd}");
}

#[test]
fn search_field_default() {
    let html = render(rsx! { SearchField { placeholder: "Filter containers" } });
    assert!(html.contains("Filter containers"));
    assert!(html.contains("/icons.svg#search"));
    assert!(html.contains("type=\"search\""));
}

#[test]
fn search_field_match_count_and_clear() {
    let html = render(rsx! { SearchField { initial_value: "api", matched: 12, total: 48 } });
    assert!(html.contains("12 of 48"), "{html}");
    assert!(html.contains("aria-label=\"Clear filter\""));
}

#[test]
fn search_field_no_matches_tone() {
    let html = render(rsx! { SearchField { initial_value: "zzz", matched: 0, total: 48 } });
    assert!(html.contains("0 of 48"));
    assert!(html.contains("search-count-none"));
}

// ---------- Select / Segmented / Checkbox / Switch / Tabs ----------

#[test]
fn select_renders_options_with_selected_value() {
    let html = render(rsx! {
        Select {
            label: "Engine",
            value: "work",
            options: vec![
                SelectOption::new("default", "default (Colima)"),
                SelectOption::new("work", "work (Colima)"),
            ],
        }
    });
    assert!(html.contains("<select"), "{html}");
    assert!(html.contains("default (Colima)"));
    assert!(html.contains("selected"), "{html}");
    assert!(html.contains("/icons.svg#chevron-down"));
}

#[test]
fn select_disabled() {
    let html = render(rsx! {
        Select { value: "a", disabled: true, options: vec![SelectOption::new("a", "A")] }
    });
    assert!(html.contains("disabled"));
}

#[test]
fn segmented_selected_segment() {
    let html = render(rsx! {
        SegmentedControl {
            selected: "failing",
            items: vec![
                SegmentedItem::with_count("all", "All", 19),
                SegmentedItem::with_count("failing", "Failing", 2),
            ],
        }
    });
    assert!(html.contains("segment-selected"), "{html}");
    assert!(html.contains("aria-pressed=\"true\""));
    assert!(html.contains("Failing"));
    assert!(html.contains("19"));
}

#[test]
fn checkbox_off() {
    let html = render(rsx! { Checkbox { state: CheckState::Off, aria_label: "Select row" } });
    assert!(html.contains("role=\"checkbox\""));
    assert!(html.contains("aria-checked=\"false\""));
    assert!(!html.contains("checkbox-on"));
}

#[test]
fn checkbox_on_shows_check() {
    let html = render(rsx! { Checkbox { state: CheckState::On, aria_label: "Select row" } });
    assert!(html.contains("aria-checked=\"true\""));
    assert!(html.contains("/icons.svg#check"));
}

#[test]
fn checkbox_mixed() {
    let html = render(rsx! { Checkbox { state: CheckState::Mixed, aria_label: "Select all" } });
    assert!(html.contains("aria-checked=\"mixed\""));
    assert!(html.contains("/icons.svg#minus"));
}

#[test]
fn checkbox_disabled_with_label() {
    let html = render(rsx! {
        Checkbox { state: CheckState::On, disabled: true, label: "Also delete volumes" }
    });
    assert!(html.contains("disabled"));
    assert!(html.contains("Also delete volumes"));
}

#[test]
fn switch_on() {
    let html = render(rsx! { Switch { on: true, label: "Follow" } });
    assert!(html.contains("role=\"switch\""));
    assert!(html.contains("aria-checked=\"true\""));
    assert!(html.contains("switch-on"));
    assert!(html.contains("Follow"));
}

#[test]
fn switch_off_disabled() {
    let html = render(rsx! { Switch { on: false, label: "Wrap", disabled: true } });
    assert!(html.contains("aria-checked=\"false\""));
    assert!(!html.contains("switch-on"));
    assert!(html.contains("disabled"));
}

#[test]
fn tabs_underline_selected() {
    let html = render(rsx! {
        Tabs {
            selected: "logs",
            items: vec![TabItem::new("overview", "Overview"), TabItem::new("logs", "Logs")],
        }
    });
    assert!(html.contains("tabs-underline"));
    assert!(html.contains("tab-selected"));
    assert!(html.contains("aria-selected=\"true\""));
    assert!(html.contains("role=\"tablist\""));
}

#[test]
fn tabs_panel_count_and_close() {
    let html = render(rsx! {
        Tabs {
            selected: "problems",
            variant: TabsVariant::Panel,
            items: vec![
                TabItem::closable("logs", "Logs"),
                TabItem::with_count("problems", "Problems", 3),
            ],
        }
    });
    assert!(html.contains("tabs-panel"));
    assert!(html.contains("aria-label=\"Close Logs\""), "{html}");
    assert!(html.contains("badge-count"));
    assert!(html.contains(">3<") || html.contains("3"));
}

// ---------- Menu / Tooltip / gallery ----------

#[test]
fn menu_items_icons_kbd_danger_separator() {
    let html = render(rsx! {
        Menu {
            items: vec![
                MenuEntry::new("terminal", "Open terminal").icon("terminal").kbd("T"),
                MenuEntry::new("delete", "Delete container\u{2026}").icon("trash-2").danger().separator_before(),
            ],
        }
    });
    assert!(html.contains("role=\"menu\""));
    assert!(html.contains("role=\"menuitem\""));
    assert!(html.contains("Open terminal"));
    assert!(html.contains("/icons.svg#terminal"));
    assert!(html.contains("menu-item-danger"));
    assert!(html.contains("menu-separator"));
    assert!(html.contains("data-symbol=\"cmd\""));
}

#[test]
fn menu_disabled_item_shows_reason() {
    let html = render(rsx! {
        Menu {
            items: vec![
                MenuEntry::new("terminal", "Open terminal").disabled("Container is not running"),
            ],
        }
    });
    assert!(html.contains("disabled"));
    assert!(html.contains("Container is not running"));
}

#[test]
fn menu_active_item() {
    let html = render(rsx! {
        Menu { active: 1, items: vec![MenuEntry::new("a", "First"), MenuEntry::new("b", "Second")] }
    });
    assert!(html.contains("menu-item-active"));
}

#[test]
fn tooltip_wraps_control_with_text_and_kbd() {
    let html = render(rsx! {
        Tooltip { text: "Restart", kbd_letter: "R",
            IconButton { icon: "rotate-cw", label: "Restart api-db-1" }
        }
    });
    assert!(html.contains("role=\"tooltip\""), "{html}");
    assert!(html.contains("Restart"));
    assert!(html.contains("aria-label=\"Restart api-db-1\""));
}

#[test]
fn gallery_renders_every_primitive() {
    let html = render(rsx! { PrimitivesGallery {} });
    for needle in [
        "Components / primitives",
        "btn-primary",
        "btn-danger",
        "icon-btn",
        "status-badge",
        "status-dot",
        "badge-arch",
        "text-field",
        "search-field",
        "select-input",
        "segmented",
        "checkbox",
        "switch",
        "tabs-panel",
        "menu-item",
        "tooltip",
        "12 of 48",
        "Exited (137)",
        "Delete container",
    ] {
        assert!(html.contains(needle), "gallery missing {needle}");
    }
}

/// The CSS rules as (selector, body) pairs, comments stripped.
fn css_rules(css: &str) -> Vec<(String, String)> {
    let mut plain = String::new();
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        plain.push_str(&rest[..start]);
        rest = rest[start..]
            .split_once("*/")
            .map_or("", |(_, after)| after);
    }
    plain.push_str(rest);
    plain
        .split('}')
        .filter_map(|rule| rule.split_once('{'))
        .map(|(selector, body)| (selector.trim().to_string(), body.trim().to_string()))
        .collect()
}

#[test]
fn fields_draw_one_focus_ring_on_the_wrapper_never_on_the_input() {
    let rules = css_rules(include_str!("../../assets/css/ui.css"));
    // No input state may draw its own ring.
    for (selector, body) in &rules {
        for sel in selector.split(',').map(str::trim) {
            let is_input_focus = ["field-input", "search-input", "select-input"]
                .iter()
                .any(|c| sel.starts_with(&format!(".{c}:focus")));
            if is_input_focus {
                assert!(
                    body.contains("outline: none"),
                    "{sel} must not draw a ring: {body}"
                );
            }
        }
    }
    // The wrapper draws the one ring, with the focus token (search fields
    // are the exception, see the next test).
    for wrapper in [".field-control", ".select-control"] {
        let ring = rules.iter().any(|(selector, body)| {
            selector
                .split(',')
                .any(|s| s.trim() == format!("{wrapper}:focus-within"))
                && body.contains("outline: 2px solid var(--color-focus)")
        });
        assert!(ring, "{wrapper}:focus-within must draw the focus ring");
    }
}

#[test]
fn search_field_focus_changes_the_border_and_draws_no_accent_ring() {
    let rules = css_rules(include_str!("../../assets/css/ui.css"));
    let focus_bodies: Vec<&String> = rules
        .iter()
        .filter(|(selector, _)| {
            selector
                .split(',')
                .any(|s| s.trim().starts_with(".search-field:focus"))
        })
        .map(|(_, body)| body)
        .collect();
    assert!(!focus_bodies.is_empty(), "search field needs a focus rule");
    for body in focus_bodies {
        for banned in [
            "--color-focus",
            "--color-accent",
            "--color-info",
            "outline",
            "box-shadow",
        ] {
            assert!(
                !body.contains(banned),
                "search focus must not use {banned}: {body}"
            );
        }
        assert!(
            body.contains("border-color: var(--color-ink)"),
            "search focus must switch the border: {body}"
        );
    }
    // The shared rule that gives other fields their ring must not name search.
    for (selector, body) in &rules {
        let names_search = selector.split(',').map(str::trim).any(|s| {
            (s.starts_with(".search-field") || s.starts_with(".search-input"))
                && s.contains(":focus")
                && !s.starts_with(".search-field:focus-within")
        });
        if names_search {
            assert!(
                !body.contains("--color-focus") && !body.contains("box-shadow: 0"),
                "{selector} draws a ring: {body}"
            );
        }
    }
}
