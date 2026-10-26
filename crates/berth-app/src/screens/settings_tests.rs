use super::*;

use crate::state::use_app_state;
use crate::ui::dock::{inspector_slot, InspectorSlot};

fn defaults() -> Settings {
    Settings::default()
}

/// Every kind of change, each one moving its setting away from the default.
fn every_change() -> Vec<(&'static str, Change)> {
    vec![
        ("theme", Change::Theme(Theme::Dark)),
        ("density", Change::Density(Density::Compact)),
        ("text size", Change::TextSize(Step::Bigger)),
        (
            "inspector dock",
            Change::InspectorDock(InspectorDock::Bottom),
        ),
        ("dock icon", Change::DockIcon(false)),
        ("update check", Change::UpdateCheck(false)),
        ("terminal app", Change::TerminalApp("iTerm".to_string())),
        ("editor", Change::Editor("zed".to_string())),
        (
            "harness path",
            Change::HarnessPath {
                id: "muse".to_string(),
                path: Some(PathBuf::from("/opt/tools/muse")),
            },
        ),
        ("log buffer", Change::LogBuffer(250_000)),
    ]
}

// --- Each control round-trips through the store ---

#[test]
fn settings_every_control_survives_closing_and_reopening_the_store() {
    for (name, change) in every_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("berth.db");
        let mut settings = defaults();
        apply(&mut settings, &change);
        assert_ne!(settings, defaults(), "{name} changed nothing");
        {
            let store = berth_store::Store::open(&file).expect("open");
            store.set_settings(&settings).expect("save");
        }
        let reopened = berth_store::Store::open(&file).expect("reopen");
        assert_eq!(reopened.settings().expect("settings"), settings, "{name}");
    }
}

#[test]
fn settings_removing_a_favourite_survives_the_store_and_leaves_the_others() {
    let keep = PathBuf::from("/Users/me/Dev/keep");
    let drop = PathBuf::from("/Users/me/Dev/drop");
    let mut settings = Settings {
        favourites: vec![keep.clone(), drop.clone()],
        ..defaults()
    };
    apply(&mut settings, &Change::RemoveFavourite(drop));
    let store = berth_store::Store::open_in_memory().expect("store");
    store.set_settings(&settings).expect("save");
    assert_eq!(store.settings().expect("settings").favourites, vec![keep]);
}

#[test]
fn settings_a_chosen_harness_binary_is_kept_until_use_default_clears_it() {
    let mut settings = defaults();
    let path = PathBuf::from("/opt/tools/claude");
    apply(
        &mut settings,
        &Change::HarnessPath {
            id: "claude-code".to_string(),
            path: Some(path.clone()),
        },
    );
    assert_eq!(settings.harness_paths.get("claude-code"), Some(&path));
    apply(
        &mut settings,
        &Change::HarnessPath {
            id: "claude-code".to_string(),
            path: None,
        },
    );
    assert!(settings.harness_paths.is_empty());
}

#[test]
fn settings_terminal_and_editor_are_trimmed_and_empty_clears_them() {
    let mut settings = defaults();
    apply(&mut settings, &Change::TerminalApp("  iTerm  ".to_string()));
    assert_eq!(settings.terminal_app.as_deref(), Some("iTerm"));
    apply(&mut settings, &Change::TerminalApp("   ".to_string()));
    assert_eq!(settings.terminal_app, None);
    apply(&mut settings, &Change::Editor("code".to_string()));
    assert_eq!(settings.editor.as_deref(), Some("code"));
    apply(&mut settings, &Change::Editor(String::new()));
    assert_eq!(settings.editor, None);
}

#[test]
fn settings_log_buffer_snaps_to_an_offered_size() {
    assert_eq!(snap_log_buffer(100_000), 100_000);
    assert_eq!(snap_log_buffer(120_000), 100_000);
    assert_eq!(snap_log_buffer(1), 10_000);
    assert_eq!(snap_log_buffer(9_000_000), 500_000);
    let mut settings = defaults();
    apply(&mut settings, &Change::LogBuffer(60_000));
    assert_eq!(settings.log_buffer_lines, 50_000);
}

// --- Text size and dock apply ---

#[test]
fn settings_text_size_steps_walk_the_list_and_reach_the_page() {
    let mut settings = defaults();
    let mut seen = vec![settings.text_scale];
    for _ in 0..3 {
        apply(&mut settings, &Change::TextSize(Step::Bigger));
        seen.push(settings.text_scale);
    }
    assert_eq!(seen, [1.0, 1.1, 1.2, 1.3]);
    // The App root turns the setting into the property on <html>.
    let script = crate::state::theme_script(settings.theme, settings.density, settings.text_scale);
    assert!(
        script.contains("setProperty('--text-scale', '1.3')"),
        "{script}"
    );
    apply(&mut settings, &Change::TextSize(Step::Smaller));
    assert!((settings.text_scale - 1.2).abs() < f32::EPSILON);
    apply(&mut settings, &Change::TextSize(Step::Reset));
    assert!((settings.text_scale - 1.0).abs() < f32::EPSILON);
    let script = crate::state::theme_script(settings.theme, settings.density, settings.text_scale);
    assert!(
        script.contains("removeProperty('--text-scale')"),
        "{script}"
    );
}

#[test]
fn settings_text_size_stops_at_both_ends() {
    let mut settings = Settings {
        text_scale: 1.5,
        ..defaults()
    };
    apply(&mut settings, &Change::TextSize(Step::Bigger));
    assert!((settings.text_scale - 1.5).abs() < f32::EPSILON);
    settings.text_scale = 0.85;
    apply(&mut settings, &Change::TextSize(Step::Smaller));
    assert!((settings.text_scale - 0.85).abs() < f32::EPSILON);
}

#[test]
fn settings_theme_and_density_reach_the_page() {
    let mut settings = defaults();
    apply(&mut settings, &Change::Theme(Theme::Dark));
    apply(&mut settings, &Change::Density(Density::Compact));
    let script = crate::state::theme_script(settings.theme, settings.density, settings.text_scale);
    assert!(script.contains("dataset.theme = 'dark'"), "{script}");
    assert!(script.contains("dataset.density = 'compact'"), "{script}");
}

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Host() -> Element {
    let state = use_app_state();
    use_hook(move || SLOT.with(|slot| *slot.borrow_mut() = Some(state)));
    rsx! { div {} }
}

fn host() -> (VirtualDom, AppState) {
    let mut dom = VirtualDom::new(Host);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("host captured the state");
    (dom, state)
}

#[test]
fn settings_the_inspector_dock_changes_where_the_inspector_is_drawn() {
    let (dom, mut state) = host();
    let slot = |state: &AppState| {
        let settings = state.settings.peek().clone();
        inspector_slot(
            true,
            false,
            settings.inspector_dock,
            settings.inspector_width_px,
            1440.0,
            settings.text_scale,
        )
    };
    dom.in_scope(dioxus::core::ScopeId::APP, || {
        assert_eq!(slot(&state), InspectorSlot::Docked(360));
        make_change(&mut state, &Change::InspectorDock(InspectorDock::Bottom));
        assert_eq!(slot(&state), InspectorSlot::Bottom);
        make_change(&mut state, &Change::InspectorDock(InspectorDock::Right));
        assert_eq!(slot(&state), InspectorSlot::Docked(360));
    });
}

#[test]
fn settings_reset_layout_restores_the_dock_the_widths_and_the_panel_height() {
    let (dom, mut state) = host();
    dom.in_runtime(|| {
        {
            let mut settings = state.settings.write();
            settings.inspector_dock = InspectorDock::Bottom;
            settings.inspector_width_px = 520;
            settings.inspector_split_pct = 65;
            // What the owner asked for stays: only the layout resets.
            settings.theme = Theme::Dark;
        }
        state.bottom.write().height_px = 540;
        make_change(&mut state, &Change::ResetLayout);
        let settings = state.settings.peek().clone();
        assert_eq!(settings.inspector_dock, InspectorDock::Right);
        assert_eq!(
            settings.inspector_width_px,
            berth_store::INSPECTOR_WIDTH_DEFAULT
        );
        assert_eq!(
            settings.inspector_split_pct,
            berth_store::INSPECTOR_SPLIT_DEFAULT
        );
        assert_eq!(settings.theme, Theme::Dark);
        assert_eq!(
            state.bottom.peek().height_px,
            crate::state::BottomPanel::default().height_px
        );
    });
}

#[test]
fn settings_commit_writes_the_signal_the_saver_watches() {
    let (dom, mut state) = host();
    dom.in_runtime(|| {
        make_change(&mut state, &Change::Theme(Theme::Light));
        make_change(&mut state, &Change::DockIcon(false));
        make_change(&mut state, &Change::UpdateCheck(false));
        let settings = state.settings.peek().clone();
        assert_eq!(settings.theme, Theme::Light);
        assert!(!settings.show_dock_icon);
        assert!(!settings.update_check);
    });
}

// --- What the screen shows ---

/// Markup only: the screen carries its CSS in a `<style>` block.
fn strip_style(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find("<style>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</style>") {
            Some(end) => rest = &rest[start + end + "</style>".len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

#[component]
fn Screen(settings: Settings, harnesses: Vec<berth_core::Harness>) -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    let mut shown = state;
    use_hook(move || {
        shown.settings.set(settings);
        shown.harnesses.set(harnesses);
    });
    rsx! { SettingsScreen {} }
}

fn render(settings: Settings, harnesses: Vec<berth_core::Harness>) -> String {
    let mut dom = VirtualDom::new_with_props(
        Screen,
        ScreenProps {
            settings,
            harnesses,
        },
    );
    let _ = dom.rebuild_to_vec();
    strip_style(&dioxus_ssr::render(&dom))
}

#[test]
fn settings_the_screen_lists_every_group_and_control() {
    let html = render(defaults(), Vec::new());
    for title in [
        "Appearance",
        "Layout",
        "Engines",
        "Assistants",
        "Apps",
        "Logs",
        "Updates and privacy",
        "Shortcuts",
    ] {
        assert!(
            html.contains(&format!("aria-label=\"{title}\"")),
            "{title}: {html}"
        );
    }
    for control in [
        "Theme",
        "Density",
        "Smaller text",
        "Bigger text",
        "Actual size",
        "Show Berth in the Dock",
        "Inspector dock",
        "Reset layout",
        "Terminal app",
        "Editor",
        "Log buffer",
        "Check for updates once a day",
        "Rescan",
        "Add engine",
    ] {
        assert!(html.contains(control), "{control}: {html}");
    }
}

#[test]
fn settings_the_screen_shows_what_the_store_holds() {
    let settings = Settings {
        theme: Theme::Dark,
        density: Density::Compact,
        text_scale: 1.3,
        inspector_dock: InspectorDock::Bottom,
        show_dock_icon: false,
        update_check: false,
        terminal_app: Some("Ghostty".to_string()),
        editor: Some("zed".to_string()),
        log_buffer_lines: 250_000,
        ..defaults()
    };
    let html = render(settings, Vec::new());
    // The pressed segment of each group is the stored value.
    let pressed =
        |label: &str| html.contains(&format!("aria-pressed=\"true\"><span>{label}</span>"));
    assert!(pressed("Dark"), "{html}");
    assert!(!pressed("System"), "{html}");
    assert!(pressed("Compact"), "{html}");
    assert!(pressed("Bottom"), "{html}");
    assert!(html.contains("130%"), "{html}");
    assert!(
        html.contains("value=\"Ghostty\"") || html.contains("Ghostty"),
        "{html}"
    );
    assert!(html.contains("zed"), "{html}");
    assert!(html.contains("250,000 lines"), "{html}");
    assert!(html.contains("The check is off"), "{html}");
    // Both switches read off.
    assert_eq!(
        html.matches("role=\"switch\" aria-checked=\"false\"")
            .count(),
        2,
        "{html}"
    );
}

#[test]
fn settings_text_size_buttons_disable_at_the_ends() {
    let top = render(
        Settings {
            text_scale: 1.5,
            ..defaults()
        },
        Vec::new(),
    );
    let bigger = top.find("Bigger text").expect("bigger button");
    let start = top[..bigger].rfind("<button").expect("button tag");
    assert!(top[start..bigger].contains("disabled"), "{top}");
    let normal = render(defaults(), Vec::new());
    let bigger = normal.find("Bigger text").expect("bigger button");
    let start = normal[..bigger].rfind("<button").expect("button tag");
    assert!(!normal[start..bigger].contains("disabled"), "{normal}");
}

fn harness(id: &str, name: &str, binary: &str, version: Option<&str>) -> berth_core::Harness {
    berth_core::Harness {
        id: berth_core::HarnessId(id.to_string()),
        name: name.to_string(),
        binary: PathBuf::from(binary),
        version: version.map(str::to_string),
        transport: berth_core::HarnessTransport::ClaudeStreamJson,
        available: version.is_some(),
    }
}

#[test]
fn settings_assistants_show_their_path_and_detected_version() {
    let html = render(
        defaults(),
        vec![
            harness(
                "claude-code",
                "Claude Code",
                "/opt/homebrew/bin/claude",
                Some("2.1.309"),
            ),
            harness("muse", "Muse Code", "muse", None),
        ],
    );
    assert!(html.contains("2.1.309"), "{html}");
    assert!(html.contains("/opt/homebrew/bin/claude"), "{html}");
    assert!(html.contains("not found"), "{html}");
    assert!(html.contains("Choose binary"), "{html}");
    assert!(
        !html.contains("Use default"),
        "no override, nothing to undo: {html}"
    );
}

#[test]
fn settings_a_chosen_binary_offers_use_default() {
    let mut settings = defaults();
    settings
        .harness_paths
        .insert("muse".to_string(), PathBuf::from("/opt/tools/muse"));
    let html = render(
        settings,
        vec![harness(
            "muse",
            "Muse Code",
            "/opt/tools/muse",
            Some("1.4.12"),
        )],
    );
    assert!(html.contains("Use default"), "{html}");
    assert!(html.contains("/opt/tools/muse"), "{html}");
}

#[test]
fn settings_favourites_group_appears_only_with_favourites() {
    // The default list holds ~/Dev when this Mac has one.
    let none = Settings {
        favourites: Vec::new(),
        ..defaults()
    };
    assert!(!render(none, Vec::new()).contains("Favourite folders"));
    let with = render(
        Settings {
            favourites: vec![PathBuf::from("/srv/apps/shop")],
            ..defaults()
        },
        Vec::new(),
    );
    assert!(with.contains("Favourite folders"), "{with}");
    assert!(
        with.contains("Remove /srv/apps/shop from favourites"),
        "{with}"
    );
}

#[test]
fn settings_lists_every_external_call() {
    let html = render(defaults(), Vec::new());
    for (name, _) in EXTERNAL_CALLS {
        assert!(html.contains(name), "{name}: {html}");
    }
    assert!(html.contains("no telemetry"), "{html}");
}

#[test]
fn settings_shortcut_list_comes_from_the_action_table() {
    let rows = shortcut_rows();
    for expected in [
        ("Bigger text", "\u{2318}="),
        ("Smaller text", "\u{2318}-"),
        ("Actual text size", "\u{2318}0"),
        ("Dock inspector at the other side", "\u{2318}\u{21e7}D"),
    ] {
        assert!(rows.contains(&expected), "{expected:?} missing");
    }
    assert_eq!(
        rows.len(),
        crate::actions::actions()
            .iter()
            .filter(|d| d.shortcut.is_some())
            .count()
    );
}

// --- Small texts ---

#[test]
fn settings_labels_read_naturally() {
    assert_eq!(scale_label(1.0), "100%");
    assert_eq!(scale_label(1.1), "110%");
    assert_eq!(scale_label(0.85), "85%");
    assert_eq!(lines_label(100_000), "100,000 lines");
    assert_eq!(lines_label(10_000), "10,000 lines");
    assert_eq!(lines_label(500_000), "500,000 lines");
}

#[test]
fn settings_update_line_follows_the_switch_and_the_status() {
    let now = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap_or_default();
    let ago = now - chrono::Duration::hours(3);
    assert!(update_text(false, &UpdateStatus::UpToDate, Some(ago), now).contains("is off"));
    assert!(update_text(true, &UpdateStatus::Unknown, None, now).contains("Not checked yet"));
    let up = update_text(true, &UpdateStatus::UpToDate, Some(ago), now);
    assert!(
        up.contains("latest version") && up.contains("3 h ago"),
        "{up}"
    );
    let newer = update_text(
        true,
        &UpdateStatus::Available {
            tag: "v0.6.0".to_string(),
            url: "https://example.test/r".to_string(),
        },
        Some(ago),
        now,
    );
    assert!(newer.contains("v0.6.0 is available"), "{newer}");
    assert!(update_text(true, &UpdateStatus::Offline, None, now).contains("Could not reach GitHub"));
}

#[test]
fn settings_available_update_shows_its_release_button() {
    #[component]
    fn WithUpdate() -> Element {
        let state = use_app_state();
        use_context_provider(|| state);
        use_context_provider(|| UpdatesView {
            status: Signal::new(UpdateStatus::Available {
                tag: "v9.9.9".to_string(),
                url: "https://github.com/fullstoptech/berth/releases/tag/v9.9.9".to_string(),
            }),
        });
        rsx! { SettingsScreen {} }
    }
    let mut dom = VirtualDom::new(WithUpdate);
    let _ = dom.rebuild_to_vec();
    let html = strip_style(&dioxus_ssr::render(&dom));
    assert!(html.contains("Berth v9.9.9 is available"), "{html}");
    assert!(html.contains("Open release page"), "{html}");
    let quiet = render(defaults(), Vec::new());
    assert!(!quiet.contains("Open release page"), "{quiet}");
}

#[test]
fn settings_scene_renders_the_screen_in_the_shell() {
    let html =
        strip_style(&crate::scenes::render_scene_html("Settings / default / 1440").expect("scene"));
    assert!(html.contains("class=\"settings-screen\""), "{html}");
    assert!(html.contains("Claude Code"), "{html}");
    assert!(html.contains("2.4.1"), "{html}");
}

#[test]
fn settings_log_buffer_sets_the_size_new_log_views_are_created_with() {
    let (dom, mut state) = host();
    dom.in_runtime(|| {
        assert_eq!(
            crate::services::logs::ring_lines(),
            crate::services::logs::RING_LINES
        );
        make_change(&mut state, &Change::LogBuffer(250_000));
        assert_eq!(state.settings.peek().log_buffer_lines, 250_000);
        assert_eq!(crate::services::logs::ring_lines(), 250_000);
        make_change(&mut state, &Change::LogBuffer(10_000));
        assert_eq!(crate::services::logs::ring_lines(), 10_000);
    });
}
