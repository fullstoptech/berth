//! The Services tab of Project detail (P5-T6): one `ContainerRow` per
//! container of the project, under the same column header as the Containers
//! table. Rows keep one order, service name then container name, whatever
//! state they are in or whichever event arrived last (AD31). A click opens
//! the container in the Containers section, where its inspector is.
//!
//! States: nothing created yet (the project has services but no containers),
//! partly running (a note names what is down), and the rows.

use dioxus::prelude::*;

use berth_core::{Container, ContainerAction, ContainerId, ContainerState, Project, ProjectId};

use crate::screens::containers::{
    copy_to_clipboard, engine_name, open_in_browser, row_data, run_container_action,
    state_view_from, target_of, with_live_stats, CONTAINERS_CSS,
};
use crate::services::actions::ActionsService;
use crate::services::stats::{use_row_stats, StatsKey, StatsService};
use crate::state::{AppState, Section, Selection};
use crate::ui::container_row::{ContainerRow, RowAction};
use crate::ui::{EmptyState, InlineMessage, InlineTone, MenuEntry, TableHeaderCell};

/// The containers of `project` in the order the tab lists them: service name,
/// then container name, each without regard to case. A container belongs to
/// the project when a service lists it or it carries the project's id.
#[must_use]
pub fn project_container_ids(project: &Project, containers: &[Container]) -> Vec<ContainerId> {
    let listed = |c: &Container| {
        c.project.as_ref() == Some(&project.id)
            || project
                .services
                .iter()
                .any(|service| service.containers.contains(&c.id))
    };
    let mut owned: Vec<&Container> = containers.iter().filter(|c| listed(c)).collect();
    owned.sort_by_cached_key(|c| {
        (
            c.service.clone().unwrap_or_default().to_lowercase(),
            c.name.to_lowercase(),
            c.id.0.clone(),
        )
    });
    owned.into_iter().map(|c| c.id.clone()).collect()
}

/// "worker and db are not running", for a project that is partly up.
#[must_use]
pub fn down_note(project: &Project, containers: &[Container]) -> Option<String> {
    let ids = project_container_ids(project, containers);
    let mut down: Vec<String> = ids
        .iter()
        .filter_map(|id| containers.iter().find(|c| &c.id == id))
        .filter(|c| c.state != ContainerState::Running)
        .map(|c| c.service.clone().unwrap_or_else(|| c.name.clone()))
        .collect();
    down.dedup();
    match down.as_slice() {
        [] => None,
        [one] => Some(format!("{one} is not running.")),
        [rest @ .., last] => Some(format!("{} and {last} are not running.", rest.join(", "))),
    }
}

/// The ⋯ menu of a row in this tab: the Containers table's entries, and for a container that is a
/// service of the project (it has a service name) "Save as template…" (P8-T4) before the last group,
/// which holds Copy container ID.
#[must_use]
pub fn service_menu(mut menu: Vec<MenuEntry>, is_service: bool) -> Vec<MenuEntry> {
    if is_service {
        let at = menu
            .iter()
            .position(|e| e.separator_before)
            .unwrap_or(menu.len());
        menu.insert(
            at,
            MenuEntry::new("save-template", "Save as template\u{2026}"),
        );
    }
    menu
}

/// Open one container in the Containers section.
fn open_container(state: &mut AppState, id: &ContainerId) {
    state.selection.set(Selection::Containers(vec![id.clone()]));
    state.section.set(Section::Containers);
}

#[component]
fn ServiceRowHost(
    id: ContainerId,
    project: ProjectId,
    menu_open: Signal<Option<ContainerId>>,
) -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_consume_context::<ActionsService>();
    let services = try_consume_context::<crate::services::Services>();
    let stats = try_consume_context::<StatsService>();
    let containers = state.containers.read();
    let listed = containers.iter().find(|c| c.id == id);
    // The row holds a stats stream while it runs (D26); the hook runs before
    // the early return so the hook order never changes.
    let key = listed
        .filter(|c| matches!(c.state, ContainerState::Running | ContainerState::Paused))
        .map(|c| StatsKey::new(&c.engine, &c.id));
    use_row_stats(stats.as_ref(), key.clone());
    let Some(listed) = listed else {
        return rsx! {};
    };
    let container = &*with_live_stats(listed, stats.as_ref(), key.as_ref());
    let engines = state.engines.read();
    let projects = state.projects.read();
    let pending = state.pending.read();
    let open = menu_open.read().as_ref() == Some(&id);
    let service_name = container.service.clone();
    let data = {
        let view = state_view_from(&state, &engines, &containers, &projects, &pending);
        let mut data = row_data(
            container,
            &view,
            engine_name(&engines, &container.engine),
            chrono::Utc::now(),
            open,
        );
        data.menu = service_menu(data.menu, service_name.is_some());
        data
    };
    let pause_action = if container.state == ContainerState::Paused {
        ContainerAction::Unpause
    } else {
        ContainerAction::Pause
    };
    let menu_actions = actions.clone();
    rsx! {
        ContainerRow {
            data,
            selected: false,
            show_uptime: true,
            show_engine: false,
            onselect: move |id: ContainerId| {
                let mut menu = menu_open;
                menu.set(None);
                open_container(&mut state, &id);
            },
            ontoggle: move |id: ContainerId| open_container(&mut state, &id),
            onaction: move |(id, action): (ContainerId, RowAction)| match action {
                RowAction::More => {
                    let mut menu = menu_open;
                    let shown = menu.peek().as_ref() == Some(&id);
                    menu.set(if shown { None } else { Some(id) });
                }
                RowAction::Logs => {
                    open_container(&mut state, &id);
                    crate::keys::dispatch(&mut state, "container.logs");
                }
                RowAction::Terminal => {
                    open_container(&mut state, &id);
                    crate::keys::dispatch(&mut state, "container.terminal");
                }
                other => run_container_action(&mut state, actions.as_ref(), &id, &other),
            },
            onport: move |port: u16| open_in_browser(port),
            onmenu: move |(id, item): (ContainerId, String)| {
                let mut menu = menu_open;
                menu.set(None);
                match item.as_str() {
                    "pause" | "resume" => {
                        let Some(service) = menu_actions.as_ref() else { return };
                        if let Some((target, _)) = target_of(&state, &id) {
                            let _ = service.container(&mut state, &target, pause_action.clone());
                        }
                    }
                    "save-template" => {
                        let found = state.projects.peek().iter().find(|p| p.id == project).cloned();
                        if let (Some(services), Some(project), Some(service)) =
                            (services.as_ref(), found, service_name.clone())
                        {
                            crate::services::templates::save(&mut state, services, project, service);
                        }
                    }
                    "copy-id" => copy_to_clipboard(&id.0),
                    _ => {}
                }
            },
        }
    }
}

/// The tab body. `start` runs Start project, offered when nothing exists yet.
#[component]
pub fn ServicesTab(
    id: ProjectId,
    on_start: EventHandler<()>,
    start_reason: Option<String>,
) -> Element {
    let state = use_context::<AppState>();
    let menu_open = use_signal(|| None::<ContainerId>);
    let projects = state.projects.read();
    let containers = state.containers.read();
    let Some(project) = projects.iter().find(|p| p.id == id) else {
        return rsx! {};
    };
    let ids = project_container_ids(project, &containers);
    let note = matches!(project.status, berth_core::ProjectStatus::Partial { .. })
        .then(|| down_note(project, &containers))
        .flatten();
    if ids.is_empty() {
        let count = project.services.len();
        let body = if project.compose_files.is_empty() {
            "This folder has no Compose file Berth knows of, so there is nothing to start yet."
                .to_string()
        } else if count == 0 {
            "Nothing has been created for this project yet.".to_string()
        } else if count == 1 {
            "Start project creates its 1 container.".to_string()
        } else {
            format!("Start project creates its {count} containers.")
        };
        return rsx! {
            div { class: "pd-empty",
                EmptyState {
                    title: "Not created yet".to_string(),
                    body,
                    icon: "box",
                    action_label: start_reason.is_none().then(|| "Start project".to_string()),
                    hint: start_reason,
                    onaction: move |()| on_start.call(()),
                }
            }
        };
    }
    rsx! {
        style { "{CONTAINERS_CSS}" }
        div { class: "pd-services",
            if let Some(note) = note {
                div { class: "pd-note",
                    InlineMessage { message: note, tone: InlineTone::Warning }
                }
            }
            div { class: "ct-table ct-grid ct-has-uptime", role: "table", "aria-label": "Services",
                div { class: "ct-head", role: "row",
                    span {}
                    span {}
                    TableHeaderCell { label: "Name".to_string() }
                    span { class: "ct-image-head", TableHeaderCell { label: "Image".to_string() } }
                    TableHeaderCell { label: "Ports".to_string() }
                    TableHeaderCell { label: "CPU".to_string(), align: crate::ui::CellAlign::Right }
                    TableHeaderCell { label: "Memory".to_string(), align: crate::ui::CellAlign::Right }
                    span { class: "ct-uptime", TableHeaderCell { label: "Uptime".to_string() } }
                    TableHeaderCell { label: "Status".to_string() }
                }
                for id in ids {
                    ServiceRowHost { key: "{id.0}", id: id.clone(), project: project.id.clone(), menu_open }
                }
            }
        }
    }
}
