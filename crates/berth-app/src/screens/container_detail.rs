//! The inspector body for one container (P3-T3, frames "Containers /
//! selected / 1440" and its light twin): header with labelled buttons,
//! tabs Overview / Logs / Terminal / Stats / Inspect (Env, Mounts, Files
//! and Limits under ⋯), and the states exited, restart loop, paused and
//! removed while open.
//!
//! The view model is in `model` (pure); `services::detail` owns the
//! inspect and log reads. This file only draws signals and sends clicks to
//! the actions service. Env values are masked until a click reveals one
//! (D79): a masked row's markup has no value, and a revealed one is
//! UI-state only and masks itself again after 30 s.

pub mod model;

use std::sync::Arc;
use std::time::Instant;

use dioxus::prelude::*;

use berth_core::{Container, ContainerAction, ContainerId, EngineAction, EnvVar, FixEffect};

use crate::screens::containers::{failure_of, select_only};
use crate::services::actions::{dismiss, ActionsService, ContainerTarget, StateView};
use crate::services::confirm;
use crate::services::detail::{DetailEntry, DetailStore, Loaded};
use crate::services::group_ops::is_active;
use crate::services::Services;
use crate::sheets::edit_recreate::{menu_click, RenameBar};
use crate::state::{AppState, Overlay, Selection};
use crate::ui::json_tree::{initially_open, match_count, node_count};
use crate::ui::{
    copy_to_clipboard, Button, ButtonVariant, ControlSize, InlineMessage, InlineTone,
    InspectorHeader, JsonTree, KeyValueList, Menu, ProblemItem, ProblemView, SearchField, Tabs,
};
use model::{
    env_rows, header_buttons, header_menu, offers_stop, overview_rows, problem_views,
    restart_loop_view, short_id, state_note, tab_strip, DetailTab, EnvRow, EnvShown, Reveals,
    REVEAL_FOR,
};

/// The screen's CSS, rendered with the panel (colours from tokens only).
pub const CONTAINER_DETAIL_CSS: &str = include_str!("../../assets/css/container_detail.css");

// ---------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------

/// Run one container action the way a row button does; a destructive one
/// opens its confirm dialog first (D33).
fn run_container(
    state: &mut AppState,
    actions: Option<&ActionsService>,
    id: &ContainerId,
    action: ContainerAction,
) {
    match action {
        ContainerAction::Remove { .. } => {
            confirm::open_delete(state, std::slice::from_ref(id));
            return;
        }
        ContainerAction::Kill { .. } => {
            if let Some(model) = confirm::kill_model(state, id) {
                state.overlays.write().push(Overlay::Confirm(model));
            }
            return;
        }
        _ => {}
    }
    let Some(service) = actions else { return };
    let target = state
        .containers
        .peek()
        .iter()
        .find(|c| c.id == *id)
        .map(ContainerTarget::from);
    let Some(target) = target else { return };
    if let Err(reason) = service.container(state, &target, action) {
        tracing::debug!(%reason, "inspector action refused");
    }
}

fn header_click(
    state: &mut AppState,
    actions: Option<&ActionsService>,
    container: &Container,
    button: &str,
) {
    let action = match button {
        "start-stop" if is_active(container.state) => ContainerAction::Stop { timeout_s: None },
        "start-stop" => ContainerAction::Start,
        "restart" => ContainerAction::Restart { timeout_s: None },
        "pause-resume" if container.state == berth_core::ContainerState::Paused => {
            ContainerAction::Unpause
        }
        "pause-resume" => ContainerAction::Pause,
        "kill" => ContainerAction::Kill { signal: None },
        "delete" => ContainerAction::Remove {
            force: is_active(container.state),
            volumes: false,
        },
        _ => return,
    };
    run_container(state, actions, &container.id, action);
}

/// A click on a problem's fix button: see `model::problem_views` for ids.
fn fix_click(state: &mut AppState, actions: Option<&ActionsService>, fix_id: &str) {
    if let Some(id) = fix_id.strip_prefix("goto:") {
        select_only(state, &ContainerId(id.to_string()));
        return;
    }
    if let Some(id) = fix_id.strip_prefix("stop-loop:") {
        run_container(
            state,
            actions,
            &ContainerId(id.to_string()),
            ContainerAction::Stop { timeout_s: None },
        );
        return;
    }
    let Some((problem, index)) = fix_id.rsplit_once('#') else {
        return;
    };
    let Ok(index) = index.parse::<usize>() else {
        return;
    };
    let fix = state
        .problems
        .peek()
        .iter()
        .find(|p| p.id.0 == problem)
        .and_then(|p| p.fixes.get(index).cloned());
    if let Some(berth_core::Fix {
        source: berth_core::FixSource::Rule,
        effect:
            FixEffect::RunAction {
                action: EngineAction::Container { id, action, .. },
                ..
            },
        ..
    }) = fix
    {
        run_container(state, actions, &id, action);
    }
}

// ---------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------

/// The inspector body for one selected container. Keyed by the caller per
/// container so tab, menus and revealed values reset on a new selection.
/// `pane_px` is the width the panel is drawn at, which sets how many tabs
/// fit before ⋯; `hide_stream_tabs` drops Logs and Terminal while the bottom
/// panel beside the inspector shows them (D22).
#[component]
pub fn ContainerDetailPanel(
    id: ContainerId,
    #[props(default)] initial_tab: DetailTab,
    #[props(default = 360)] pane_px: u32,
    #[props(default)] hide_stream_tabs: bool,
) -> Element {
    let mut state = use_context::<AppState>();
    let store = use_hook(|| try_consume_context::<DetailStore>().unwrap_or_default());
    let services = try_consume_context::<Services>();
    let actions = try_consume_context::<ActionsService>();
    let mut tab = use_signal(move || initial_tab);
    let mut menu_open = use_signal(|| false);
    let mut renaming = use_signal(|| false);
    let mut more_tabs_open = use_signal(|| false);

    // The list row without its live stats: a stats tick must not redraw
    // the whole panel (a JSON tree can be hundreds of rows).
    let row_id = id.clone();
    let container = use_memo(move || {
        state
            .containers
            .read()
            .iter()
            .find(|c| c.id == row_id)
            .cloned()
            .map(|mut c| {
                c.stats = None;
                c
            })
    });
    let first_seen = use_hook(|| container.peek().clone());

    use_effect(move || {
        if let (Some(c), Some(services)) = (container(), services.clone()) {
            store.ensure(&services, &c);
        }
    });

    let Some(c) = container() else {
        return removed_view(first_seen.as_ref(), &mut state);
    };

    let now = chrono::Utc::now();
    let entry = store.entries.read().get(&id).cloned();
    let (buttons, failure, overview_problems) = {
        let engines = state.engines.read();
        let containers = state.containers.read();
        let projects = state.projects.read();
        let pending = state.pending.read();
        let problems = state.problems.read();
        let view = StateView {
            engines: &engines,
            containers: &containers,
            projects: &projects,
            images: &[],
            volumes: &[],
            networks: &[],
            pending: &pending,
            backing: *state.backing.peek(),
        };
        let mut shown = problem_views(&c, &problems, &containers, &view, now);
        if c.state == berth_core::ContainerState::Restarting && !offers_stop(&problems, &c) {
            shown.push(restart_loop_view(&c, &view));
        }
        (header_buttons(&c, &view), failure_of(&pending, &id), shown)
    };

    let hidden: &[DetailTab] = if hide_stream_tabs {
        &DetailTab::STREAMS
    } else {
        &[]
    };
    // A tab the strip no longer offers (Logs after the panel opened beside
    // the inspector) falls back to Overview.
    let selected = if hidden.contains(&tab.read()) {
        DetailTab::Overview
    } else {
        *tab.read()
    };
    let (text_scale, dock) = {
        let settings = state.settings.read();
        (settings.text_scale, settings.inspector_dock)
    };
    #[allow(clippy::cast_precision_loss)]
    let (tab_items, overflow) = tab_strip(selected, pane_px as f32, text_scale, hidden);
    let has_overflow = !overflow.is_empty();
    let status = berth_core::status_of(c.state, c.health, c.exit_code);
    let header_actions = actions.clone();
    let fix_actions = actions.clone();
    let header_container = c.clone();
    let menu_container = c.clone();
    let menu_actions = actions.clone();
    let menu_id = id.clone();

    rsx! {
        style { "{CONTAINER_DETAIL_CSS}" }
        div { class: "cd",
            InspectorHeader {
                name: c.name.clone(),
                status,
                note: state_note(&c, now),
                image: c.image.reference.clone(),
                id_short: short_id(&c.id.0),
                id_full: c.id.0.clone(),
                buttons,
                menu: header_menu(&c, *state.backing.peek()),
                menu_open: *menu_open.read(),
                dock,
                ondock: move |side: berth_store::InspectorDock| {
                    menu_open.set(false);
                    crate::ui::dock::set_dock(&mut state, side);
                },
                onbutton: move |button: &'static str| {
                    menu_open.set(false);
                    header_click(&mut state, header_actions.as_ref(), &header_container, button);
                },
                onmenutoggle: move |()| {
                    let open = *menu_open.peek();
                    menu_open.set(!open);
                },
                onmenu: move |item: String| {
                    menu_open.set(false);
                    if item == "copy-id" {
                        copy_to_clipboard(&menu_id.0);
                    } else if menu_click(&mut state, menu_actions.as_ref(), store, &menu_container, &item) {
                        renaming.set(true);
                    }
                },
            }
            if *renaming.read() {
                RenameBar {
                    key: "{id.0}",
                    container: c.clone(),
                    onclose: move |()| renaming.set(false),
                }
            }
            if let Some((key, message)) = failure {
                div { class: "cd-failure",
                    InlineMessage { message, tone: InlineTone::Danger }
                    button {
                        class: "cd-failure-dismiss",
                        r#type: "button",
                        "aria-label": "Dismiss message",
                        onclick: move |_| dismiss(&mut state, &key),
                        "Dismiss"
                    }
                }
            }
            div { class: "cd-tabs",
                Tabs {
                    items: tab_items,
                    selected: selected.id().to_string(),
                    aria_label: "Container detail".to_string(),
                    on_select: move |tab_id: String| {
                        more_tabs_open.set(false);
                        if let Some(next) = DetailTab::from_id(&tab_id) {
                            tab.set(next);
                        }
                    },
                }
                if has_overflow {
                    div { class: "cd-more-tabs",
                        button {
                            class: "tab cd-more-button",
                            r#type: "button",
                            "aria-label": "More tabs",
                            "aria-expanded": more_tabs_open.read().to_string(),
                            onclick: move |_| {
                                let open = *more_tabs_open.peek();
                                more_tabs_open.set(!open);
                            },
                            crate::ui::Icon { name: "more-horizontal", size: crate::ui::IconSize::Sm }
                        }
                        if *more_tabs_open.read() {
                            div { class: "cd-more-menu",
                                Menu {
                                    items: overflow,
                                    aria_label: "More tabs".to_string(),
                                    on_select: move |tab_id: String| {
                                        more_tabs_open.set(false);
                                        if let Some(next) = DetailTab::from_id(&tab_id) {
                                            tab.set(next);
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
            }
            div { class: "cd-body", role: "tabpanel",
                {
                    tab_body(
                        selected,
                        &c,
                        entry,
                        overview_problems,
                        move |fix: String| fix_click(&mut state, fix_actions.as_ref(), &fix),
                    )
                }
            }
        }
    }
}

fn removed_view(first_seen: Option<&Container>, state: &mut AppState) -> Element {
    let name = first_seen.map_or_else(|| "This container".to_string(), |c| c.name.clone());
    let mut selection = state.selection;
    rsx! {
        style { "{CONTAINER_DETAIL_CSS}" }
        div { class: "cd cd-removed",
            h2 { class: "cd-removed-name", "{name}" }
            p { class: "cd-removed-text", "This container was removed." }
            Button {
                label: "Close".to_string(),
                variant: ButtonVariant::Secondary,
                size: ControlSize::Sm,
                onclick: move |_| selection.set(Selection::None),
            }
        }
    }
}

fn tab_body(
    tab: DetailTab,
    c: &Container,
    entry: Option<DetailEntry>,
    problems: Vec<ProblemView>,
    onfix: impl FnMut(String) + 'static,
) -> Element {
    if tab == DetailTab::Stats {
        return rsx! { crate::ui::metric_tile::StatsPanel { id: c.id.clone() } };
    }
    if tab == DetailTab::Logs {
        return rsx! { crate::screens::logs::LogPanel { containers: vec![c.id.clone()] } };
    }
    if tab == DetailTab::Terminal {
        return rsx! { crate::ui::terminal::TerminalPanel { container: c.id.clone() } };
    }
    if tab == DetailTab::Files {
        return rsx! { crate::ui::FilesPanel { container: c.id.clone() } };
    }
    if tab == DetailTab::Limits {
        return rsx! { crate::screens::container_limits::LimitsPanel { container: c.clone() } };
    }
    let loaded = match entry {
        None => return rsx! { p { class: "cd-note", "Loading details…" } },
        Some(DetailEntry::Failed { message, gone }) => {
            let text = if gone {
                "This container no longer exists.".to_string()
            } else {
                format!("Could not read the details: {message}")
            };
            return rsx! { InlineMessage { message: text, tone: InlineTone::Danger } };
        }
        Some(DetailEntry::Ready(loaded)) => *loaded,
    };
    match tab {
        DetailTab::Overview => rsx! {
            OverviewTab { loaded, problems, onfix: EventHandler::new(onfix) }
        },
        DetailTab::Inspect => rsx! { InspectTab { inspect: Arc::new(loaded.inspect) } },
        DetailTab::Env => rsx! { EnvTab { env: loaded.detail.env } },
        DetailTab::Mounts => rsx! { MountsTab { container: c.clone() } },
        _ => rsx! {},
    }
}

// ---------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------

#[component]
fn OverviewTab(loaded: Loaded, problems: Vec<ProblemView>, onfix: EventHandler<String>) -> Element {
    let c = &loaded.detail.container;
    let rows = overview_rows(&loaded.detail, &berth_sys::paths::home());
    // The Networks row has buttons (P4-T5): the rows before and after it are
    // two lists that read as one, with `ContainerNetworks` between them.
    let split = rows.iter().position(|r| r.label == "Networks");
    let (before, after) = match split {
        Some(at) => (rows[..at].to_vec(), rows[at + 1..].to_vec()),
        None => (rows, Vec::new()),
    };
    let exited = matches!(
        c.state,
        berth_core::ContainerState::Exited | berth_core::ContainerState::Dead
    );
    let now = chrono::Utc::now();
    rsx! {
        if c.state == berth_core::ContainerState::Paused {
            InlineMessage {
                message: "Paused. Its processes are frozen until you resume it.".to_string(),
                tone: InlineTone::Warning,
            }
        }
        KeyValueList { rows: before }
        if split.is_some() {
            crate::screens::network_detail::ContainerNetworks { id: c.id.clone() }
            KeyValueList { rows: after }
        }
        for problem in problems {
            ProblemItem { key: "{problem.title}", problem, onfix }
        }
        if exited {
            section { class: "cd-section",
                h3 { class: "cd-section-title",
                    if loaded.last_lines.is_empty() {
                        "Last lines"
                    } else {
                        "Last {loaded.last_lines.len()} lines"
                    }
                }
                if loaded.last_lines.is_empty() {
                    p { class: "cd-note", "No output." }
                } else {
                    pre { class: "cd-lines",
                        for line in &loaded.last_lines {
                            "{line}\n"
                        }
                    }
                }
            }
        }
        if !loaded.detail.health_log.is_empty() {
            section { class: "cd-section",
                h3 { class: "cd-section-title", "Health checks" }
                ul { class: "cd-health",
                    for check in loaded.detail.health_log.iter().rev() {
                        li {
                            key: "{check.start}",
                            class: if check.exit_code == 0 { "cd-health-row" } else { "cd-health-row cd-health-fail" },
                            span { class: "cd-health-when", "{berth_core::fmt::age(check.start, now)}" }
                            span { class: "cd-health-code", "exit {check.exit_code}" }
                            span { class: "cd-health-output", "{check.output.trim()}" }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// Inspect
// ---------------------------------------------------------------------

#[component]
fn InspectTab(inspect: Arc<serde_json::Value>) -> Element {
    let mut query = use_signal(String::new);
    let mut open = use_signal(initially_open);
    let mut epoch = use_signal(|| 0_u32);
    let q = query.read().clone();
    let (matched, total) = if q.trim().is_empty() {
        (None, None)
    } else {
        (Some(match_count(&inspect, &q)), Some(node_count(&inspect)))
    };
    rsx! {
        div { class: "cd-inspect",
            div { class: "cd-inspect-search",
                for generation in [*epoch.read()] {
                    SearchField {
                        key: "{generation}",
                        initial_value: query.peek().clone(),
                        placeholder: "Search the inspect".to_string(),
                        aria_label: "Search the inspect".to_string(),
                        matched,
                        total,
                        oninput: move |e: FormEvent| query.set(e.value()),
                        onclear: move |_| {
                            query.set(String::new());
                            let next = *epoch.peek() + 1;
                            epoch.set(next);
                        },
                    }
                }
            }
            JsonTree {
                value: inspect.clone(),
                query: q,
                open: open.read().clone(),
                ontoggle: move |path: String| {
                    let mut set = open.write();
                    if !set.remove(&path) {
                        set.insert(path);
                    }
                },
            }
        }
    }
}

// ---------------------------------------------------------------------
// Env
// ---------------------------------------------------------------------

/// The rows of the Env tab. A masked row has no value in its markup; a
/// click on it asks the host to reveal that one key.
#[component]
pub fn EnvList(
    rows: Vec<EnvRow>,
    onreveal: EventHandler<String>,
    onhide: EventHandler<String>,
) -> Element {
    if rows.is_empty() {
        return rsx! { p { class: "cd-note", "No environment variables." } };
    }
    rsx! {
        p { class: "cd-note",
            "Values stay hidden. Click one to show it for {REVEAL_FOR.as_secs()} seconds."
        }
        dl { class: "env-list",
            for row in rows {
                {
                    let reveal_key = row.key.clone();
                    let hide_key = row.key.clone();
                    rsx! {
                        div { key: "{row.key}", class: "env-row",
                            dt { class: "env-key", "{row.key}" }
                            dd { class: "env-cell",
                                match &row.shown {
                                    EnvShown::Masked => rsx! {
                                        button {
                                            class: "env-value env-masked",
                                            r#type: "button",
                                            "aria-label": "Show the value of {row.key}",
                                            onclick: move |_| onreveal.call(reveal_key.clone()),
                                            "{crate::services::detail::MASK}"
                                        }
                                    },
                                    EnvShown::Revealed(value) => rsx! {
                                        button {
                                            class: "env-value env-revealed",
                                            r#type: "button",
                                            "aria-label": "Hide the value of {row.key}",
                                            onclick: move |_| onhide.call(hide_key.clone()),
                                            "{value}"
                                        }
                                    },
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
fn EnvTab(env: Vec<EnvVar>) -> Element {
    let mut reveals = use_signal(Reveals::default);
    let rows = env_rows(&env, &reveals.read(), Instant::now());
    rsx! {
        EnvList {
            rows,
            onreveal: move |key: String| {
                reveals.write().reveal(&key, Instant::now());
                spawn(async move {
                    tokio::time::sleep(REVEAL_FOR).await;
                    reveals.write().expire(Instant::now());
                });
            },
            onhide: move |key: String| reveals.write().hide(&key),
        }
    }
}

// ---------------------------------------------------------------------
// Mounts
// ---------------------------------------------------------------------

#[component]
fn MountsTab(container: Container) -> Element {
    if container.mounts.is_empty() {
        return rsx! { p { class: "cd-note", "No mounts." } };
    }
    let compose_dir = container
        .labels
        .get("com.docker.compose.project.working_dir")
        .cloned();
    let home = berth_sys::paths::home();
    let rows = container
        .mounts
        .iter()
        .map(|m| {
            let kind = match m.kind {
                berth_core::MountKind::Bind => "Bind",
                berth_core::MountKind::Volume => "Volume",
                berth_core::MountKind::Tmpfs => "Tmpfs",
                berth_core::MountKind::Other => "Other",
            };
            crate::ui::KeyValue::mono(kind, model::mount_line(m, compose_dir.as_deref(), &home))
        })
        .collect();
    rsx! { KeyValueList { rows } }
}

#[cfg(test)]
#[path = "container_detail_tests.rs"]
mod tests;
