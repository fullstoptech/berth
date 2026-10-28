//! Plugin sections and the declarative views they draw (P9-T5, PLAN §6.10
//! and §6.11, D45; frames "Coolify / deployments / 1440" and "Coolify /
//! confirm redeploy / 1440").
//!
//! A plugin draws no UI: it returns a `View` and this file draws it with the
//! app's own components. Table rows go through `VirtualTable`, a Detail uses
//! the inspector's layout (header, key-value list, tabs, actions), a
//! LogStream is a log in the app's row style that opens on the failing step,
//! a Message is a banner and a Form is a settings form. The screen reads
//! `PluginViewStore` and `PluginsView`; every load, action and stream goes
//! through `services::plugin_views` (D23).
//!
//! An action that carries a `Confirm` waits for a click in a dialog drawn
//! here (no `Overlay` variant exists for it, AD51); one without runs at once.

use std::collections::BTreeMap;

use dioxus::prelude::*;

use berth_plugin_host::PluginState;
use berth_plugin_protocol::{ActionSpec, Cell, Column, ConfigField, FieldKind, Row, Tone, View};

use super::plugin_action::{request_action, select_row, FixWithHarness};
use super::plugin_view_model::{
    banner_tone, facets, failing_seq, filter_rows, grid_template, log_rows, row_context,
    row_summary, status_banner, time_text, tone_class, LogRow, RowSummary,
};
use crate::services::plugin_views::{PluginViewStore, RowContext, ViewState, STATUS_VIEW};
use crate::services::plugins::{PluginsService, PluginsView};
use crate::state::AppState;
use crate::ui::confirm_dialog::CONFIRM_DIALOG_CSS;
use crate::ui::log_line::{LogLine, RowOpts};
use crate::ui::{
    Badge, BadgeKind, Banner, Button, ButtonVariant, ControlSize, EmptyState, Icon, IconSize,
    InlineMessage, InlineTone, Inspector, InspectorFrame, KeyValue, KeyValueList, SearchField,
    Switch, TabItem, Tabs, TextField, VirtualTable,
};

/// The screen's CSS.
pub const PLUGIN_VIEW_CSS: &str = include_str!("../../assets/css/plugin_view.css");

/// Rows a table window is tall enough for (the table scrolls inside the page).
const TABLE_VIEWPORT_PX: u32 = 1100;

// ---------------------------------------------------------------------
// Running actions
// ---------------------------------------------------------------------

fn action_variant(action: &ActionSpec, primary: bool) -> ButtonVariant {
    if action.danger {
        ButtonVariant::Danger
    } else if primary {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Secondary
    }
}

/// The buttons of a set of actions.
#[component]
fn ActionButtons(
    plugin: String,
    actions: Vec<ActionSpec>,
    #[props(default)] size: ControlSize,
    #[props(default)] first_primary: bool,
    /// Row buttons stay plain even for a destructive action: the confirm
    /// carries the warning.
    #[props(default)]
    quiet: bool,
    /// The row the buttons belong to, handed to the action.
    #[props(default)]
    context: Option<RowContext>,
) -> Element {
    let store = use_context::<PluginViewStore>();
    let state = use_context::<AppState>();
    rsx! {
        for (index, action) in actions.iter().enumerate() {
            {
                let (plugin, action_clone) = (plugin.clone(), action.clone());
                let context = context.clone();
                // A row's plain buttons show on the hovered or selected row; a
                // destructive one (Cancel) always shows.
                let hover_only = quiet && !action.danger;
                rsx! {
                    span {
                        key: "{action.id}",
                        class: if hover_only { "pv-act pv-act-hover" } else { "pv-act" },
                    Button {
                        label: action.label.clone(),
                        variant: if quiet { ButtonVariant::Secondary } else { action_variant(action, first_primary && index == 0) },
                        size,
                        onclick: move |e: MouseEvent| {
                            e.stop_propagation();
                            let service = try_consume_context::<Option<PluginsService>>().flatten();
                            request_action(
                                store,
                                service.as_ref(),
                                state,
                                &plugin,
                                &action_clone,
                                context.clone(),
                            );
                        },
                    }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// The section
// ---------------------------------------------------------------------

/// The main area of a plugin's section. The section itself is keyed by the
/// plugin, so switching plugins starts its status check and its
/// `view/invalidate` watch afresh; the confirm dialog sits outside it.
#[component]
pub fn PluginViewScreen() -> Element {
    let store = use_context::<PluginViewStore>();
    let plugin = store.nav.read().plugin.clone();
    rsx! {
        style { "{CONFIRM_DIALOG_CSS}" }
        style { "{PLUGIN_VIEW_CSS}" }
        for key in [plugin.clone()] {
            PluginSection { key: "{key}", plugin: plugin.clone() }
        }
    }
}

/// A plugin's banner, the view the sidebar item opened and, beside it,
/// whatever a selected row or an action opened.
#[component]
fn PluginSection(plugin: String) -> Element {
    let store = use_context::<PluginViewStore>();
    let plugins = use_context::<PluginsView>();
    let nav = store.nav.read().clone();
    let entry = plugins.entry(&plugin);
    let available = entry
        .as_ref()
        .is_some_and(|e| e.plugin.state == PluginState::Enabled);

    // The hidden status view carries the plugin's version warning (AD51).
    {
        let plugin = plugin.clone();
        use_effect(move || {
            if plugin.is_empty() {
                return;
            }
            let service = try_consume_context::<Option<PluginsService>>().flatten();
            store.ensure(service.as_ref(), &plugin, STATUS_VIEW);
        });
    }
    // The plugin's own `view/invalidate` notices reload what is loaded.
    {
        let plugin = plugin.clone();
        use_hook(move || {
            if plugin.is_empty() {
                return;
            }
            if let Some(service) = try_consume_context::<Option<PluginsService>>().flatten() {
                crate::services::plugin_views::watch_invalidations(store, service, plugin);
            }
        });
    }

    let banner = match store.view(&plugin, STATUS_VIEW) {
        Some(ViewState::Ready(view)) => status_banner(&view),
        _ => None,
    };
    let name = entry
        .as_ref()
        .map_or_else(|| plugin.clone(), |e| e.plugin.manifest.name.clone());

    rsx! {
        section { class: "pv-screen", "aria-label": "{name}",
            if plugin.is_empty() || !available {
                div { class: "pv-empty",
                    EmptyState {
                        title: "This plugin is not running".to_string(),
                        body: "Enable it in Plugins to see its screens.".to_string(),
                        icon: "zap",
                    }
                }
            } else {
                if let Some((tone, text)) = banner {
                    div { class: "pv-banner",
                        Banner { message: text, tone }
                    }
                }
                div { class: "pv-split",
                    div { class: "pv-main",
                        // One-item loop: a new view remounts its filter and tabs.
                        for key in [nav.view.clone()] {
                            ViewBody { key: "{key}", plugin: plugin.clone(), view: nav.view.clone() }
                        }
                    }
                    if let Some(open) = nav.open.clone() {
                        for key in [open.clone()] {
                            Pane {
                                key: "{key}",
                                plugin: plugin.clone(),
                                main_view: nav.view.clone(),
                                row: nav.row.clone(),
                                open: open.clone(),
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A view loaded into its slot: Loading, the failure with Try again, or its content.
#[component]
fn ViewBody(plugin: String, view: String, #[props(default)] in_pane: bool) -> Element {
    let store = use_context::<PluginViewStore>();
    {
        let (plugin, view) = (plugin.clone(), view.clone());
        use_effect(move || {
            let service = try_consume_context::<Option<PluginsService>>().flatten();
            store.ensure(service.as_ref(), &plugin, &view);
        });
    }
    match store.view(&plugin, &view) {
        None | Some(ViewState::Loading) => rsx! {
            div { class: "pv-loading", role: "status", "Loading\u{2026}" }
        },
        Some(ViewState::Failed(reason)) => {
            let (plugin, view) = (plugin.clone(), view.clone());
            rsx! {
                div { class: "pv-failed",
                    InlineMessage { message: reason, tone: InlineTone::Danger }
                    Button {
                        label: "Try again".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Sm,
                        onclick: move |_| {
                            let service = try_consume_context::<Option<PluginsService>>().flatten();
                            store.reload(service.as_ref(), &plugin, &view);
                        },
                    }
                }
            }
        }
        Some(ViewState::Ready(content)) => rsx! {
            ViewContent { plugin, content, in_pane }
        },
    }
}

/// One `View`, drawn by its kind.
#[component]
pub fn ViewContent(plugin: String, content: View, #[props(default)] in_pane: bool) -> Element {
    match content {
        View::Table {
            title,
            columns,
            rows,
            empty,
            ..
        } => rsx! {
            TableView { plugin, title, columns, rows, empty, in_pane }
        },
        View::Detail {
            title,
            status,
            fields,
            tabs,
            actions,
        } => rsx! {
            DetailView { plugin, title, status, fields, tabs, actions }
        },
        View::LogStream { stream, title } => rsx! {
            LogStreamView { key: "{stream}", plugin, stream, title }
        },
        View::Message { tone, text, action } => rsx! {
            MessageView { plugin, tone, text, action }
        },
        View::Form {
            title,
            fields,
            submit,
        } => rsx! {
            FormView { plugin, title, fields, submit }
        },
    }
}

// ---------------------------------------------------------------------
// Table
// ---------------------------------------------------------------------

#[component]
fn CellView(cell: Cell) -> Element {
    match cell {
        Cell::Text(text) => rsx! { span { class: "pv-text", title: "{text}", "{text}" } },
        Cell::Mono(text) => rsx! { span { class: "pv-mono", title: "{text}", "{text}" } },
        Cell::Time(iso) => {
            let shown = time_text(&iso, &chrono::Local);
            rsx! { span { class: "pv-time", title: "{iso}", "{shown}" } }
        }
        Cell::Badge(badge) => {
            let class = format!("status-badge {}", tone_class(&badge.tone));
            rsx! {
                span { class,
                    span { class: "status-dot" }
                    span { "{badge.text}" }
                }
            }
        }
    }
}

#[component]
fn TableView(
    plugin: String,
    title: String,
    columns: Vec<Column>,
    rows: Vec<Row>,
    empty: String,
    #[props(default)] in_pane: bool,
) -> Element {
    let store = use_context::<PluginViewStore>();
    let state = use_context::<AppState>();
    let mut filter = use_signal(String::new);
    let mut epoch = use_signal(|| 0_u32);
    let mut chips_hidden = use_signal(|| false);
    let chips = facets(&columns, &rows);
    let selected = store.nav.read().row.clone();

    let total = rows.len();
    let shown: Vec<Row> = filter_rows(&rows, &filter.read())
        .into_iter()
        .cloned()
        .collect();
    let filtering = !filter.read().trim().is_empty();
    let with_actions = rows.iter().any(|r| !r.actions.is_empty());
    let grid = grid_template(&columns, &rows, with_actions);
    let count = shown.len();
    let row_height_px = {
        let settings = state.settings.read();
        crate::text_scale::row_height_px(settings.density, settings.text_scale)
    };

    let render_row = {
        let (plugin, columns, shown, selected) =
            (plugin.clone(), columns.clone(), shown.clone(), selected);
        use_callback(move |i: usize| -> Element {
            let Some(row) = shown.get(i) else {
                return rsx! {};
            };
            let is_selected = selected.as_deref() == Some(row.id.as_str());
            let class = if is_selected {
                "pv-row pv-selected"
            } else {
                "pv-row"
            };
            let context = row_context(&columns, row);
            let select = {
                let (context, open) = (context.clone(), row.open.clone());
                let plugin = plugin.clone();
                move |_| {
                    if !in_pane {
                        select_row(store, &plugin, &context, open.clone());
                    }
                }
            };
            rsx! {
                div {
                    key: "{row.id}",
                    class,
                    role: "row",
                    "aria-selected": "{is_selected}",
                    onclick: select,
                    for column in columns.iter() {
                        div {
                            key: "{column.key}",
                            class: if column.align_right { "pv-cell pv-right" } else { "pv-cell" },
                            role: "cell",
                            if let Some(cell) = row.cells.get(&column.key) {
                                CellView { cell: cell.clone() }
                            }
                        }
                    }
                    if with_actions {
                        div { class: "pv-actions", role: "cell",
                            ActionButtons {
                                plugin: plugin.clone(),
                                actions: row.actions.clone(),
                                size: ControlSize::Sm,
                                quiet: true,
                                context: Some(context.clone()),
                            }
                        }
                    }
                }
            }
        })
    };

    rsx! {
        div { class: if in_pane { "pv-table pv-table-pane" } else { "pv-table" },
            div { class: "pv-header",
                if in_pane {
                    h2 { class: "pv-subtitle", "{title}" }
                } else {
                    h1 { class: "pv-title", "{title}" }
                }
                Badge { kind: BadgeKind::Count, text: "{total}" }
                span { class: "pv-spacer" }
                div { class: "pv-filter",
                    for key in [*epoch.read()] {
                        SearchField {
                            key: "{key}",
                            initial_value: filter.peek().clone(),
                            placeholder: format!("Filter {}", title.to_lowercase()),
                            aria_label: format!("Filter {}", title.to_lowercase()),
                            matched: if filtering { Some(count) } else { None },
                            total: if filtering { Some(total) } else { None },
                            oninput: move |e: FormEvent| filter.set(e.value()),
                            onclear: move |_| {
                                filter.set(String::new());
                                let next = *epoch.peek() + 1;
                                epoch.set(next);
                            },
                        }
                    }
                }
            }
            if !chips.is_empty() && !in_pane && !*chips_hidden.read() {
                div { class: "pv-chips",
                    for chip in chips.iter() {
                        span { key: "{chip.label}", class: "pv-chip",
                            Icon { name: "filter", size: IconSize::Sm }
                            span { "{chip.label}: {chip.value}" }
                            button {
                                class: "pv-chip-x",
                                r#type: "button",
                                "aria-label": "Remove the {chip.label} filter",
                                onclick: move |_| chips_hidden.set(true),
                                Icon { name: "x", size: IconSize::Sm }
                            }
                        }
                    }
                }
            }
            if total == 0 {
                div { class: "pv-empty",
                    EmptyState { title: empty.clone(), body: String::new(), icon: "layers" }
                }
            } else if count == 0 {
                div { class: "pv-empty",
                    EmptyState {
                        title: "No row matches".to_string(),
                        body: "Nothing here matches the filter.".to_string(),
                        icon: "search",
                        action_label: Some("Clear filter".to_string()),
                        onaction: move |()| {
                            filter.set(String::new());
                            let next = *epoch.peek() + 1;
                            epoch.set(next);
                        },
                    }
                }
            } else {
                div {
                    class: "pv-grid",
                    role: "table",
                    "aria-label": "{title}",
                    style: "--pv-cols: {grid};",
                    div { class: "pv-head", role: "row",
                        for column in columns.iter() {
                            div {
                                key: "{column.key}",
                                class: if column.align_right { "pv-head-cell pv-right" } else { "pv-head-cell" },
                                role: "columnheader",
                                "{column.label}"
                            }
                        }
                        if with_actions {
                            div { class: "pv-head-cell", role: "columnheader" }
                        }
                    }
                    VirtualTable {
                        total: count,
                        render_row,
                        row_height_px,
                        viewport_height_px: TABLE_VIEWPORT_PX,
                        overscan: 10_usize,
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// The pane beside a table
// ---------------------------------------------------------------------

/// What a selected row (or an action's result) opened, in the inspector's
/// frame: a Detail draws its own header; anything else gets the row's, with
/// the linked local folder, "Fix with <harness>" on a failed row and the
/// row's buttons.
#[component]
fn Pane(plugin: String, main_view: String, row: Option<String>, open: String) -> Element {
    let store = use_context::<PluginViewStore>();
    let found = row.as_deref().and_then(|id| {
        let ViewState::Ready(View::Table { columns, rows, .. }) =
            store.view(&plugin, &main_view)?
        else {
            return None;
        };
        let found = rows.iter().find(|r| r.id == id)?;
        Some((
            row_summary(&columns, found),
            row_context(&columns, found),
            found.actions.clone(),
        ))
    });
    let opened_is_detail = matches!(
        store.view(&plugin, &open),
        Some(ViewState::Ready(View::Detail { .. }))
    );
    rsx! {
        Inspector { frame: InspectorFrame::Docked,
            div { class: "pv-pane",
                if let (Some((summary, context, actions)), false) = (found, opened_is_detail) {
                    RowHeader { plugin: plugin.clone(), summary, context, actions, open: open.clone() }
                }
                ViewBody { plugin: plugin.clone(), view: open, in_pane: true }
            }
        }
    }
}

#[component]
fn RowHeader(
    plugin: String,
    summary: RowSummary,
    context: RowContext,
    actions: Vec<ActionSpec>,
    open: String,
) -> Element {
    let state = use_context::<AppState>();
    // A button that only opens the view already open has nothing to do here.
    let actions: Vec<ActionSpec> = actions
        .into_iter()
        .filter(|a| crate::services::plugin_views::open_only(a) != Some(open.as_str()))
        .collect();
    let folder = context
        .repository
        .as_deref()
        .and_then(|repo| {
            crate::services::plugin_views::linked_project(&state.projects.read(), repo)
                .and_then(|p| p.folder.clone())
        })
        .map(|f| berth_core::fmt::tilde(&f, &berth_sys::paths::home()));
    let failed = context.failed;
    rsx! {
        div { class: "pv-row-header",
            div { class: "pv-row-title",
                h2 { class: "pv-pane-title", "{summary.title}" }
                if let Some(badge) = summary.badge.clone() {
                    CellView { cell: Cell::Badge(badge) }
                }
            }
            if !summary.line.is_empty() {
                p { class: "pv-muted", "{summary.line}" }
            }
            if let Some(folder) = folder {
                p { class: "pv-muted", "Linked folder {folder} \u{b7} git remote matches" }
            }
            div { class: "pv-pane-actions pv-pane-actions-col",
                if failed {
                    FixWithHarness { plugin: plugin.clone(), context: context.clone() }
                }
                if !actions.is_empty() {
                    div { class: "pv-pane-actions",
                        ActionButtons { plugin, actions, size: ControlSize::Md, context: Some(context) }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// Detail
// ---------------------------------------------------------------------

#[component]
fn DetailView(
    plugin: String,
    title: String,
    status: Option<berth_plugin_protocol::Badge>,
    fields: Vec<(String, String)>,
    tabs: Vec<berth_plugin_protocol::Tab>,
    actions: Vec<ActionSpec>,
) -> Element {
    let mut tab = use_signal(|| tabs.first().map(|t| t.id.clone()).unwrap_or_default());
    let items: Vec<TabItem> = tabs
        .iter()
        .map(|t| TabItem::new(t.id.clone(), t.title.clone()))
        .collect();
    let selected = tab.read().clone();
    let tab_view = tabs
        .iter()
        .find(|t| t.id == selected)
        .map(|t| t.view.clone());
    let rows: Vec<KeyValue> = fields
        .iter()
        .map(|(label, value)| {
            if value.len() > 40 || label == "Address" || label == "Commit" {
                KeyValue::mono(label.clone(), value.clone())
            } else {
                KeyValue::text(label.clone(), value.clone())
            }
        })
        .collect();
    rsx! {
        div { class: "pv-detail",
            div { class: "pv-row-title",
                h2 { class: "pv-pane-title", "{title}" }
                if let Some(badge) = status {
                    CellView { cell: Cell::Badge(badge) }
                }
            }
            if !actions.is_empty() {
                div { class: "pv-pane-actions",
                    ActionButtons {
                        plugin: plugin.clone(),
                        actions,
                        size: ControlSize::Md,
                        first_primary: true,
                    }
                }
            }
            if !rows.is_empty() {
                KeyValueList { rows }
            }
            if !items.is_empty() {
                Tabs {
                    items,
                    selected: selected.clone(),
                    aria_label: Some(format!("{title} sections")),
                    on_select: move |id: String| tab.set(id),
                }
                if let Some(view) = tab_view {
                    for key in [view.clone()] {
                        div { key: "{key}", class: "pv-tab-body",
                            ViewBody { plugin: plugin.clone(), view: view.clone(), in_pane: true }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// Messages and forms
// ---------------------------------------------------------------------

#[component]
fn MessageView(plugin: String, tone: Tone, text: String, action: Option<ActionSpec>) -> Element {
    rsx! {
        div { class: "pv-message",
            Banner { message: text, tone: banner_tone(&tone) }
            if let Some(action) = action {
                div { class: "pv-pane-actions",
                    ActionButtons { plugin, actions: vec![action], size: ControlSize::Md }
                }
            }
        }
    }
}

#[component]
fn FormView(
    plugin: String,
    title: String,
    fields: Vec<ConfigField>,
    submit: ActionSpec,
) -> Element {
    let store = use_context::<PluginViewStore>();
    let state = use_context::<AppState>();
    let mut values = use_signal(BTreeMap::<String, serde_json::Value>::new);
    rsx! {
        form {
            class: "pv-form",
            onsubmit: move |e| e.prevent_default(),
            h2 { class: "pv-pane-title", "{title}" }
            for field in fields.iter() {
                {
                    let key = field.key.clone();
                    let label = if field.required {
                        format!("{} (required)", field.label)
                    } else {
                        field.label.clone()
                    };
                    rsx! {
                        div { key: "{field.key}", class: "pv-field",
                            if field.kind == FieldKind::Bool {
                                Switch {
                                    on: values.read().get(&key).and_then(serde_json::Value::as_bool).unwrap_or(false),
                                    label: Some(label),
                                    on_toggle: move |on: bool| {
                                        values.write().insert(key.clone(), serde_json::Value::Bool(on));
                                    },
                                }
                            } else {
                                TextField {
                                    label: Some(label.clone()),
                                    aria_label: Some(label),
                                    input_type: match field.kind {
                                        FieldKind::Secret => "password".to_string(),
                                        FieldKind::Url => "url".to_string(),
                                        _ => "text".to_string(),
                                    },
                                    oninput: move |e: FormEvent| {
                                        values.write().insert(key.clone(), serde_json::Value::String(e.value()));
                                    },
                                }
                            }
                        }
                    }
                }
            }
            Button {
                label: submit.label.clone(),
                variant: action_variant(&submit, true),
                size: ControlSize::Md,
                onclick: move |_| {
                    let mut action = submit.clone();
                    if let serde_json::Value::Object(map) = &mut action.params {
                        map.insert("values".to_string(), serde_json::json!(values.peek().clone()));
                    }
                    let service = try_consume_context::<Option<PluginsService>>().flatten();
                    request_action(store, service.as_ref(), state, &plugin, &action, None);
                },
            }
        }
    }
}

// ---------------------------------------------------------------------
// Log stream
// ---------------------------------------------------------------------

/// The element id of a stream's scroller.
fn log_dom_id(stream: &str) -> String {
    let slug: String = stream
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("pv-log-{slug}")
}

/// Scroll the failing row to the middle of its log, or follow the end of a
/// running one.
fn scroll_script(dom_id: &str, failing: Option<u64>, running: bool) -> String {
    match (failing, running) {
        (Some(seq), _) => format!(
            "document.querySelector('#{dom_id} [data-seq=\"{seq}\"]')?.scrollIntoView({{block: 'center'}});"
        ),
        (None, true) => format!(
            "const el = document.getElementById('{dom_id}'); if (el) el.scrollTop = el.scrollHeight;"
        ),
        (None, false) => String::new(),
    }
}

fn status_text(status: Option<&str>) -> (&'static str, &'static str) {
    match status {
        None => ("Streaming\u{2026}", "tone-info"),
        Some("finished") => ("Finished", "tone-success"),
        Some("failed" | "error") => ("Failed", "tone-danger"),
        Some(_) => ("Ended", "tone-stopped"),
    }
}

#[component]
fn LogStreamView(plugin: String, stream: String, title: String) -> Element {
    let store = use_context::<PluginViewStore>();
    // The follow lives as long as this component: dropping it closes the
    // stream. A scene that seeded the stream leaves it as recorded.
    let follow = use_hook(|| {
        std::rc::Rc::new(std::cell::RefCell::new(
            None::<crate::services::plugin_views::LogFollow>,
        ))
    });
    {
        let (plugin, stream) = (plugin.clone(), stream.clone());
        let follow = follow.clone();
        use_effect(move || {
            let key = (plugin.clone(), stream.clone());
            if follow.borrow().is_some() || store.logs.peek().contains_key(&key) {
                return;
            }
            let service = try_consume_context::<Option<PluginsService>>().flatten();
            *follow.borrow_mut() = store.open_log(service.as_ref(), &plugin, &stream);
        });
    }

    let log = store
        .logs
        .read()
        .get(&(plugin.clone(), stream.clone()))
        .cloned()
        .unwrap_or_default();
    let rows = log_rows(&log);
    let failing = failing_seq(&rows, &log);
    let running = log.final_status.is_none();
    let dom_id = log_dom_id(&stream);
    let version = log.version;

    // Scroll after the rows are in the page.
    {
        let (dom_id, failing) = (dom_id.clone(), failing);
        use_effect(move || {
            let script = scroll_script(&dom_id, failing, running);
            let _ = version;
            if script.is_empty() {
                return;
            }
            spawn(async move {
                let _ = document::eval(&script).await;
            });
        });
    }

    let (status_label, status_tone) = status_text(log.final_status.as_deref());
    let dropped = log.dropped;
    rsx! {
        div { class: "pv-log",
            div { class: "pv-log-head",
                h3 { class: "pv-log-title", "{title}" }
                if running {
                    span { class: "pv-log-status {status_tone}", "{status_label}" }
                }
            }
            if dropped > 0 {
                p { class: "pv-muted", "The first {dropped} lines are not shown." }
            }
            div {
                class: "log-list log-wrap pv-log-list",
                id: "{dom_id}",
                role: "table",
                "aria-label": "{title}",
                style: "--log-row: 22px; --log-cols: calc(28px * var(--text-scale, 1)) calc(108px * var(--text-scale, 1)) 0 minmax(0, 1fr);",
                if rows.is_empty() {
                    div { class: "log-empty",
                        if running { "Waiting for the first line\u{2026}" } else { "Nothing was written" }
                    }
                }
                for LogRow { row, tone } in rows {
                    LogLine {
                        key: "{row.seq}",
                        opts: RowOpts {
                            tone,
                            time: true,
                            seconds_only: true,
                            wrap: true,
                            ..RowOpts::default()
                        },
                        row,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "plugin_view_tests.rs"]
mod tests;
