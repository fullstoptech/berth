//! The Settings screen (P6-T2, brief 7.3; no frame, so it is built from the Templates header and
//! the component set). One column of groups, each a list of rows with the label (and what it
//! does) on the left and the control on the right: appearance, layout, engines, assistants, apps,
//! logs, favourites, updates and privacy, shortcuts.
//!
//! Every control is one [`Change`]: [`apply`] is pure over `berth_store::Settings`, the component
//! maps a click to a change and writes the signal, and the settings saver (`services::settings_save`)
//! persists it. The App root applies theme, density and text size to `<html>`, the dock icon to
//! the activation policy and the inspector dock to the layout, so a change takes effect at once.

use std::path::PathBuf;

use dioxus::prelude::*;

use berth_store::{Density, InspectorDock, Settings, Theme};

use crate::services::first_run::{refresh, FirstRunStore, Scan};
use crate::services::updates::{current_version, UpdateStatus, UpdatesView};
use crate::state::{AppState, Overlay};
use crate::text_scale::{self, Step};
use crate::ui::{
    Badge, BadgeKind, Button, ButtonVariant, ControlSize, IconButton, SegmentedControl,
    SegmentedItem, Select, SelectOption, Switch, TextField,
};

/// The screen's CSS.
pub const SETTINGS_CSS: &str = include_str!("../../assets/css/settings.css");

/// The log buffer sizes the select offers (lines kept per log view).
pub const LOG_BUFFER_OPTIONS: [u32; 5] = [10_000, 50_000, 100_000, 250_000, 500_000];

/// Every external call Berth can make (D61). Nothing else leaves this Mac.
pub const EXTERNAL_CALLS: [(&str, &str); 6] = [
    (
        "Docker Hub",
        "Searching for images and listing their tags (hub.docker.com).",
    ),
    (
        "Registries you add",
        "Pulling and searching images on a registry you set up.",
    ),
    (
        "GitHub releases",
        "The daily update check (api.github.com). Off with the switch above.",
    ),
    (
        "Coolify",
        "Your own Coolify server, only when the Coolify plugin is configured.",
    ),
    (
        "Git remotes",
        "Reading a project's branch and remote from its folder with git.",
    ),
    (
        "Assistants",
        "Claude Code or Muse, only when you start a session; they use your own login.",
    ),
];

/// One edit made on the screen.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Theme(Theme),
    Density(Density),
    TextSize(Step),
    InspectorDock(InspectorDock),
    ResetLayout,
    DockIcon(bool),
    UpdateCheck(bool),
    /// Empty text clears the setting.
    TerminalApp(String),
    /// Empty text clears the setting.
    Editor(String),
    /// A harness binary the user chose, or `None` to go back to the search path.
    HarnessPath {
        id: String,
        path: Option<PathBuf>,
    },
    RemoveFavourite(PathBuf),
    LogBuffer(u32),
}

fn text_or_none(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The log buffer option nearest to `lines`.
#[must_use]
pub fn snap_log_buffer(lines: u32) -> u32 {
    LOG_BUFFER_OPTIONS
        .iter()
        .copied()
        .min_by_key(|option| option.abs_diff(lines))
        .unwrap_or(100_000)
}

/// Apply one change to the settings.
pub fn apply(settings: &mut Settings, change: &Change) {
    match change {
        Change::Theme(theme) => settings.theme = *theme,
        Change::Density(density) => settings.density = *density,
        Change::TextSize(step) => {
            settings.text_scale = text_scale::next_scale(settings.text_scale, *step);
        }
        Change::InspectorDock(dock) => settings.inspector_dock = *dock,
        Change::ResetLayout => {
            settings.inspector_dock = InspectorDock::default();
            settings.inspector_width_px = berth_store::INSPECTOR_WIDTH_DEFAULT;
            settings.inspector_split_pct = berth_store::INSPECTOR_SPLIT_DEFAULT;
        }
        Change::DockIcon(shown) => settings.show_dock_icon = *shown,
        Change::UpdateCheck(on) => settings.update_check = *on,
        Change::TerminalApp(text) => settings.terminal_app = text_or_none(text),
        Change::Editor(text) => settings.editor = text_or_none(text),
        Change::HarnessPath { id, path } => match path {
            Some(path) => {
                settings.harness_paths.insert(id.clone(), path.clone());
            }
            None => {
                settings.harness_paths.remove(id);
            }
        },
        Change::RemoveFavourite(folder) => settings.favourites.retain(|f| f != folder),
        Change::LogBuffer(lines) => settings.log_buffer_lines = snap_log_buffer(*lines),
    }
}

/// Make a change on the live app: the settings signal, plus what lives outside it (the dock's
/// toggle state, the bottom panel's height, the log ring, a rescan after a binary changed).
pub fn make_change(state: &mut AppState, change: &Change) {
    match change {
        Change::InspectorDock(dock) => crate::ui::dock::set_dock(state, *dock),
        Change::ResetLayout => {
            apply(&mut state.settings.write(), change);
            state.bottom.write().height_px = crate::state::BottomPanel::default().height_px;
        }
        other => apply(&mut state.settings.write(), other),
    }
    if let Change::LogBuffer(_) = change {
        let lines = state.settings.peek().log_buffer_lines;
        crate::services::logs::set_ring_lines(lines as usize);
    }
}

fn theme_items() -> Vec<SegmentedItem> {
    vec![
        SegmentedItem::new("system", "System"),
        SegmentedItem::new("light", "Light"),
        SegmentedItem::new("dark", "Dark"),
    ]
}

fn theme_id(theme: Theme) -> &'static str {
    match theme {
        Theme::System => "system",
        Theme::Light => "light",
        Theme::Dark => "dark",
    }
}

fn theme_of(id: &str) -> Theme {
    match id {
        "light" => Theme::Light,
        "dark" => Theme::Dark,
        _ => Theme::System,
    }
}

/// "110 %" for a scale of 1.1.
#[must_use]
pub fn scale_label(scale: f32) -> String {
    format!("{:.0}%", text_scale::snap(scale) * 100.0)
}

/// "100,000 lines".
#[must_use]
pub fn lines_label(lines: u32) -> String {
    let digits = lines.to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    format!("{grouped} lines")
}

/// The line the update row shows under its switch.
#[must_use]
pub fn update_text(
    enabled: bool,
    status: &UpdateStatus,
    last_checked: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    if !enabled {
        return "The check is off. Berth never contacts GitHub.".to_string();
    }
    let last = last_checked.map_or_else(
        || "Not checked yet".to_string(),
        |at| format!("Last checked {}", berth_core::fmt::age(at, now)),
    );
    match status {
        UpdateStatus::Available { tag, .. } => format!("Berth {tag} is available. {last}."),
        UpdateStatus::UpToDate => {
            format!("Berth {} is the latest version. {last}.", current_version())
        }
        UpdateStatus::Offline => format!("Could not reach GitHub. {last}."),
        UpdateStatus::Failed(why) => format!("The check failed: {why}. {last}."),
        UpdateStatus::Unknown => format!("Berth {}. {last}.", current_version()),
    }
}

/// Open a link in the default browser; fire and forget.
fn open_link(url: String) {
    let (program, args) = crate::screens::containers::open_command(&url);
    spawn(async move {
        let search = berth_sys::SearchPath::from_process();
        let Some(path) = search.find(program) else {
            tracing::warn!(program, "cannot open a link: program not found");
            return;
        };
        let args: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
        if let Err(err) = berth_sys::run_capture(
            &path,
            &args,
            berth_sys::SpawnOpts::default(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        {
            tracing::warn!(%err, "opening a link failed");
        }
    });
}

/// The shortcuts to list: every action that carries one, grouped as the table orders them.
#[must_use]
pub fn shortcut_rows() -> Vec<(&'static str, &'static str)> {
    crate::actions::actions()
        .iter()
        .filter_map(|def| def.shortcut.map(|keys| (def.label, keys)))
        .collect()
}

#[component]
fn Group(title: String, #[props(default)] action: Option<Element>, children: Element) -> Element {
    rsx! {
        section { class: "st-group", "aria-label": "{title}",
            div { class: "st-group-head",
                h2 { class: "st-group-title", "{title}" }
                if let Some(action) = action {
                    div { class: "st-group-action", {action} }
                }
            }
            div { class: "st-card", {children} }
        }
    }
}

#[component]
fn SettingRow(label: String, #[props(default)] hint: Option<String>, children: Element) -> Element {
    rsx! {
        div { class: "st-row", role: "group", "aria-label": "{label}",
            div { class: "st-row-text",
                span { class: "st-row-label", "{label}" }
                if let Some(hint) = hint {
                    span { class: "st-row-hint", "{hint}" }
                }
            }
            div { class: "st-row-control", {children} }
        }
    }
}

#[component]
fn Appearance() -> Element {
    let mut state = use_context::<AppState>();
    let settings = state.settings.read().clone();
    let dock_label = if settings.show_dock_icon {
        "Shown"
    } else {
        "Hidden"
    };
    let density_items = vec![
        SegmentedItem::new("comfortable", "Comfortable"),
        SegmentedItem::new("compact", "Compact"),
    ];
    let density = match settings.density {
        Density::Comfortable => "comfortable",
        Density::Compact => "compact",
    };
    rsx! {
        Group { title: "Appearance".to_string(),
            SettingRow { label: "Theme".to_string(),
                SegmentedControl {
                    items: theme_items(),
                    selected: theme_id(settings.theme).to_string(),
                    aria_label: Some("Theme".to_string()),
                    on_select: move |id: String| make_change(&mut state, &Change::Theme(theme_of(&id))),
                }
            }
            SettingRow {
                label: "Density".to_string(),
                hint: Some("Compact rows are 24 px; text sizes do not change.".to_string()),
                SegmentedControl {
                    items: density_items,
                    selected: density.to_string(),
                    aria_label: Some("Density".to_string()),
                    on_select: move |id: String| {
                        let next = if id == "compact" { Density::Compact } else { Density::Comfortable };
                        make_change(&mut state, &Change::Density(next));
                    },
                }
            }
            SettingRow {
                label: "Text size".to_string(),
                hint: Some(format!(
                    "{m}= bigger, {m}\u{2212} smaller, {m}0 actual size.",
                    m = crate::actions::mod_label()
                )),
                div { class: "st-stepper",
                    Button {
                        label: "Smaller text".to_string(),
                        icon: Some("minus"),
                        size: ControlSize::Sm,
                        disabled: text_scale::next_scale(settings.text_scale, Step::Smaller) == text_scale::snap(settings.text_scale),
                        onclick: move |_| make_change(&mut state, &Change::TextSize(Step::Smaller)),
                    }
                    span { class: "st-stepper-value", role: "status", "aria-label": "Text size", "{scale_label(settings.text_scale)}" }
                    Button {
                        label: "Bigger text".to_string(),
                        icon: Some("plus"),
                        size: ControlSize::Sm,
                        disabled: text_scale::next_scale(settings.text_scale, Step::Bigger) == text_scale::snap(settings.text_scale),
                        onclick: move |_| make_change(&mut state, &Change::TextSize(Step::Bigger)),
                    }
                    Button {
                        label: "Actual size".to_string(),
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Sm,
                        disabled: text_scale::snap(settings.text_scale) == text_scale::DEFAULT,
                        onclick: move |_| make_change(&mut state, &Change::TextSize(Step::Reset)),
                    }
                }
            }
            SettingRow {
                label: "Dock icon".to_string(),
                hint: Some("Hidden, Berth stays in the menu bar.".to_string()),
                Switch {
                    on: settings.show_dock_icon,
                    aria_label: Some("Show Berth in the Dock".to_string()),
                    label: Some(dock_label.to_string()),
                    on_toggle: move |on: bool| make_change(&mut state, &Change::DockIcon(on)),
                }
            }
        }
    }
}

#[component]
fn LayoutGroup() -> Element {
    let mut state = use_context::<AppState>();
    let dock = state.settings.read().inspector_dock;
    let items = vec![
        SegmentedItem::new("right", "Right"),
        SegmentedItem::new("bottom", "Bottom"),
    ];
    let selected = match dock {
        InspectorDock::Right => "right",
        InspectorDock::Bottom => "bottom",
    };
    rsx! {
        Group { title: "Layout".to_string(),
            SettingRow {
                label: "Inspector".to_string(),
                hint: Some(format!("{}\u{21e7}D moves it to the other side.", crate::actions::mod_label())),
                SegmentedControl {
                    items,
                    selected: selected.to_string(),
                    aria_label: Some("Inspector dock".to_string()),
                    on_select: move |id: String| {
                        let next = if id == "bottom" { InspectorDock::Bottom } else { InspectorDock::Right };
                        make_change(&mut state, &Change::InspectorDock(next));
                    },
                }
            }
            SettingRow {
                label: "Reset layout".to_string(),
                hint: Some("Inspector back at the right, 360 px wide; the bottom panel at its usual height.".to_string()),
                Button {
                    label: "Reset layout".to_string(),
                    size: ControlSize::Sm,
                    onclick: move |_| make_change(&mut state, &Change::ResetLayout),
                }
            }
        }
    }
}

fn engine_kind_text(kind: &berth_core::EngineKind) -> String {
    match kind {
        berth_core::EngineKind::Colima { profile } => format!("Colima profile {profile}"),
        other => {
            let text = format!("{other:?}");
            text.split(['{', ' ', '('])
                .next()
                .unwrap_or("Engine")
                .to_string()
        }
    }
}

fn engine_state_text(state: &berth_core::EngineState) -> &'static str {
    match state {
        berth_core::EngineState::Running => "Running",
        berth_core::EngineState::Stopped => "Stopped",
        berth_core::EngineState::Starting => "Starting",
        berth_core::EngineState::Stopping => "Stopping",
        berth_core::EngineState::Unreachable { .. } => "Unreachable",
        berth_core::EngineState::Unsupported { .. } => "Not supported",
    }
}

#[component]
fn EnginesGroup() -> Element {
    let mut state = use_context::<AppState>();
    let engines = state.engines.read().clone();
    rsx! {
        Group {
            title: "Engines".to_string(),
            action: rsx! {
                Button {
                    label: "Add engine\u{2026}".to_string(),
                    size: ControlSize::Sm,
                    onclick: move |_| state.overlays.write().push(Overlay::AddEngine),
                }
            },
            if engines.is_empty() {
                div { class: "st-empty", "No engine found yet." }
            }
            for engine in engines {
                div { key: "{engine.id.0}", class: "st-row", role: "row", "aria-label": "{engine.name}",
                    div { class: "st-row-text",
                        span { class: "st-row-label", "{engine.name}" }
                        span { class: "st-row-hint", "{engine_kind_text(&engine.kind)}" }
                    }
                    div { class: "st-row-control",
                        span { class: "st-muted", "{engine_state_text(&engine.state)}" }
                    }
                }
            }
        }
    }
}

/// Pick a binary for a harness; an unchanged answer (the dialog cancelled) changes nothing.
fn choose_binary(mut state: AppState, store: Option<FirstRunStore>, mock: bool, id: String) {
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("Choose the program")
            .pick_file()
            .await
        else {
            return;
        };
        make_change(
            &mut state,
            &Change::HarnessPath {
                id,
                path: Some(file.path().to_path_buf()),
            },
        );
        if let Some(store) = store {
            refresh(state, store, mock, Scan::Again);
        }
    });
}

#[component]
fn HarnessRow(harness: berth_core::Harness, overridden: bool) -> Element {
    let state = use_context::<AppState>();
    let store = try_use_context::<FirstRunStore>();
    let mock = *state.backing.peek() == crate::services::backing::Backing::Mock;
    let id = harness.id.0.clone();
    let reset_id = id.clone();
    let tilde = berth_core::fmt::tilde(&harness.binary, &berth_sys::paths::home());
    let (status, tone) = if harness.available {
        (
            harness
                .version
                .clone()
                .unwrap_or_else(|| "found".to_string()),
            "tone-success",
        )
    } else {
        ("not found".to_string(), "tone-stopped")
    };
    rsx! {
        div { class: "st-row", role: "row", "aria-label": "{harness.name}",
            div { class: "st-row-text",
                span { class: "st-row-label", "{harness.name}" }
                span { class: "st-row-hint st-mono", title: "{harness.binary.display()}",
                    if harness.available || overridden { "{tilde}" } else { "Not on this Mac's search path" }
                }
            }
            div { class: "st-row-control",
                span { class: "st-version {tone}", "{status}" }
                if overridden {
                    Button {
                        label: "Use default".to_string(),
                        variant: ButtonVariant::Ghost,
                        size: ControlSize::Sm,
                        onclick: move |_| {
                            let mut state = state;
                            make_change(&mut state, &Change::HarnessPath { id: reset_id.clone(), path: None });
                            if let Some(store) = store {
                                refresh(state, store, mock, Scan::Again);
                            }
                        },
                    }
                }
                Button {
                    label: "Choose binary\u{2026}".to_string(),
                    size: ControlSize::Sm,
                    onclick: move |_| choose_binary(state, store, mock, id.clone()),
                }
            }
        }
    }
}

#[component]
fn Assistants() -> Element {
    let state = use_context::<AppState>();
    let store = try_use_context::<FirstRunStore>();
    let mock = *state.backing.peek() == crate::services::backing::Backing::Mock;
    let harnesses = state.harnesses.read().clone();
    let overrides = state.settings.read().harness_paths.clone();
    let scanning = store.is_some_and(|s| *s.scanning.read());
    rsx! {
        Group {
            title: "Assistants".to_string(),
            action: rsx! {
                Button {
                    label: "Rescan".to_string(),
                    size: ControlSize::Sm,
                    icon: Some("rotate-cw"),
                    loading: scanning,
                    onclick: move |_| {
                        if let Some(store) = store {
                            refresh(state, store, mock, Scan::Again);
                        }
                    },
                }
            },
            if harnesses.is_empty() {
                div { class: "st-empty", if scanning { "Looking for Claude Code and Muse\u{2026}" } else { "Press Rescan to look for Claude Code and Muse." } }
            }
            for harness in harnesses {
                HarnessRow {
                    key: "{harness.id.0}",
                    overridden: overrides.contains_key(&harness.id.0),
                    harness,
                }
            }
        }
    }
}

#[component]
fn AppsGroup() -> Element {
    let mut state = use_context::<AppState>();
    let settings = state.settings.read().clone();
    rsx! {
        Group { title: "Apps".to_string(),
            SettingRow {
                label: "Terminal app".to_string(),
                hint: Some("Used by Open in Terminal. Empty uses Terminal.".to_string()),
                div { class: "st-field",
                    TextField {
                        initial_value: settings.terminal_app.clone().unwrap_or_default(),
                        placeholder: Some("Terminal".to_string()),
                        aria_label: Some("Terminal app".to_string()),
                        onchange: move |e: FormEvent| make_change(&mut state, &Change::TerminalApp(e.value())),
                    }
                }
            }
            SettingRow {
                label: "Editor".to_string(),
                hint: Some("The command that opens a file in your editor. Empty uses VS Code when it is installed.".to_string()),
                div { class: "st-field",
                    TextField {
                        initial_value: settings.editor.clone().unwrap_or_default(),
                        placeholder: Some("code".to_string()),
                        aria_label: Some("Editor".to_string()),
                        onchange: move |e: FormEvent| make_change(&mut state, &Change::Editor(e.value())),
                    }
                }
            }
        }
    }
}

#[component]
fn LogsGroup() -> Element {
    let mut state = use_context::<AppState>();
    let current = snap_log_buffer(state.settings.read().log_buffer_lines);
    let options: Vec<SelectOption> = LOG_BUFFER_OPTIONS
        .iter()
        .map(|lines| SelectOption::new(lines.to_string(), lines_label(*lines)))
        .collect();
    rsx! {
        Group { title: "Logs".to_string(),
            SettingRow {
                label: "Log buffer".to_string(),
                hint: Some("Lines kept per log view; the oldest go first. Applies to the next view you open.".to_string()),
                div { class: "st-select",
                    Select {
                        options,
                        value: current.to_string(),
                        aria_label: Some("Log buffer".to_string()),
                        onchange: move |e: FormEvent| {
                            if let Ok(lines) = e.value().parse::<u32>() {
                                make_change(&mut state, &Change::LogBuffer(lines));
                            }
                        },
                    }
                }
            }
        }
    }
}

#[component]
fn Favourites() -> Element {
    let mut state = use_context::<AppState>();
    let favourites = state.settings.read().favourites.clone();
    if favourites.is_empty() {
        return rsx! {};
    }
    let home = berth_sys::paths::home();
    rsx! {
        Group { title: "Favourite folders".to_string(),
            for folder in favourites {
                {
                    let shown = berth_core::fmt::tilde(&folder, &home);
                    let name = shown.clone();
                    let target = folder.clone();
                    rsx! {
                        div { key: "{shown}", class: "st-row", role: "row", "aria-label": "{shown}",
                            div { class: "st-row-text",
                                span { class: "st-row-label st-mono", "{shown}" }
                            }
                            div { class: "st-row-control",
                                IconButton {
                                    icon: "x",
                                    label: format!("Remove {name} from favourites"),
                                    size: ControlSize::Sm,
                                    onclick: move |_| make_change(&mut state, &Change::RemoveFavourite(target.clone())),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Privacy() -> Element {
    let mut state = use_context::<AppState>();
    let updates = try_use_context::<UpdatesView>();
    let settings = state.settings.read().clone();
    let check_label = if settings.update_check { "On" } else { "Off" };
    let status = updates.map_or(UpdateStatus::Unknown, |view| view.status.read().clone());
    let text = update_text(
        settings.update_check,
        &status,
        settings.update_last_checked,
        chrono::Utc::now(),
    );
    let release = match &status {
        UpdateStatus::Available { url, .. } => Some(url.clone()),
        _ => None,
    };
    rsx! {
        Group { title: "Updates and privacy".to_string(),
            SettingRow { label: "Check for updates".to_string(), hint: Some(text),
                div { class: "st-inline",
                    if let Some(url) = release {
                        Button {
                            label: "Open release page".to_string(),
                            variant: ButtonVariant::Primary,
                            size: ControlSize::Sm,
                            onclick: move |_| open_link(url.clone()),
                        }
                    }
                    Switch {
                        on: settings.update_check,
                        aria_label: Some("Check for updates once a day".to_string()),
                        label: Some(check_label.to_string()),
                        on_toggle: move |on: bool| make_change(&mut state, &Change::UpdateCheck(on)),
                    }
                }
            }
            div { class: "st-subhead", "External calls" }
            p { class: "st-note", "Berth has no telemetry. These are the only things it can contact, and only when you use them." }
            for (name, what) in EXTERNAL_CALLS {
                div { key: "{name}", class: "st-row st-row-call", role: "row", "aria-label": "{name}",
                    div { class: "st-row-text",
                        span { class: "st-row-label", "{name}" }
                        span { class: "st-row-hint", "{what}" }
                    }
                }
            }
        }
    }
}

#[component]
fn Shortcuts() -> Element {
    let rows = shortcut_rows();
    rsx! {
        Group { title: "Shortcuts".to_string(),
            div { class: "st-shortcuts",
                for (label, keys) in rows {
                    div { key: "{label}", class: "st-shortcut", role: "row",
                        span { class: "st-shortcut-label", "{label}" }
                        span { class: "st-shortcut-keys", "{keys}" }
                    }
                }
            }
        }
    }
}

#[component]
pub fn SettingsScreen() -> Element {
    rsx! {
        style { "{SETTINGS_CSS}" }
        section { class: "settings-screen", "aria-label": "Settings",
            div { class: "st-header",
                h1 { class: "st-title", "Settings" }
                Badge { kind: BadgeKind::Tag, text: format!("Berth {}", current_version()) }
            }
            div { class: "st-body",
                Appearance {}
                LayoutGroup {}
                EnginesGroup {}
                Assistants {}
                AppsGroup {}
                LogsGroup {}
                Favourites {}
                Privacy {}
                Shortcuts {}
            }
        }
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
