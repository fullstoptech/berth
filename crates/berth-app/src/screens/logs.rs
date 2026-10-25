//! The Logs section (frames "Logs / errors / 1440" and "Logs / paused / 1440"): a title, one chip
//! per followed container in its series colour, and a [`LogViewer`] over the merged streams.
//! [`LogPanel`] is the same viewer without the title and chips, for the bottom panel's Logs tab
//! and the inspector's Logs tab: hand it the containers and it owns its session.

use berth_core::{Container, ContainerId, Project, ProjectId};
use dioxus::prelude::*;

use crate::services::logs::{series_of, use_log_session, LogTarget};
use crate::state::{AppState, Selection};
use crate::ui::log_view::ViewPrefs;
use crate::ui::{EmptyState, Icon, IconSize, LogViewer, Menu, MenuEntry};

/// The chip text: the container's name without its project's prefix and the trailing replica
/// number, so `shop-api-worker-1` in `shop-api` reads `worker`.
#[must_use]
pub fn short_name(name: &str, project: Option<&str>) -> String {
    let rest = project
        .and_then(|p| name.strip_prefix(p))
        .map_or(name, |r| r.trim_start_matches(['-', '_']));
    let rest = if rest.is_empty() { name } else { rest };
    let without_replica = rest
        .rsplit_once(['-', '_'])
        .filter(|(head, tail)| {
            !head.is_empty() && !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit())
        })
        .map_or(rest, |(head, _)| head);
    without_replica.to_string()
}

/// Is this container part of the project: listed as a service container, or linked to it?
fn in_project(container: &Container, project: &Project) -> bool {
    container.project.as_ref() == Some(&project.id)
        || project
            .services
            .iter()
            .any(|service| service.containers.contains(&container.id))
}

/// The project a container belongs to.
#[must_use]
pub fn project_of<'a>(container: &Container, projects: &'a [Project]) -> Option<&'a Project> {
    projects.iter().find(|p| in_project(container, p))
}

/// The containers a view follows, in the order asked, resolved against the list (a container that
/// is gone is skipped).
#[must_use]
pub fn targets_for(containers: &[Container], ids: &[ContainerId]) -> Vec<LogTarget> {
    ids.iter()
        .filter_map(|id| containers.iter().find(|c| &c.id == id))
        .map(|c| LogTarget {
            engine: c.engine.clone(),
            id: c.id.clone(),
            name: c.name.clone(),
        })
        .collect()
}

/// "Logs · shop-api (5 containers)": the project's name when every container is in one project,
/// the container's own name for one, else a count.
#[must_use]
pub fn view_title(containers: &[Container], projects: &[Project], ids: &[ContainerId]) -> String {
    let chosen: Vec<&Container> = ids
        .iter()
        .filter_map(|id| containers.iter().find(|c| &c.id == id))
        .collect();
    match chosen.as_slice() {
        [] => "Logs".to_string(),
        [only] => format!("Logs \u{b7} {}", only.name),
        many => {
            let count = many.len();
            let project = many.first().and_then(|c| project_of(c, projects));
            let shared = project.filter(|p| many.iter().all(|c| in_project(c, p)));
            match shared {
                Some(project) => format!("Logs \u{b7} {} ({count} containers)", project.name),
                None => format!("Logs \u{b7} {count} containers"),
            }
        }
    }
}

/// What the Logs views follow when nothing was picked: the selected containers, or every
/// container of the selected project.
#[must_use]
pub fn ids_from_selection(
    selection: &Selection,
    containers: &[Container],
    projects: &[Project],
) -> Vec<ContainerId> {
    match selection {
        Selection::Containers(ids) => ids.clone(),
        Selection::Project(id) => projects
            .iter()
            .find(|p| &p.id == id)
            .map(|project| {
                containers
                    .iter()
                    .filter(|c| in_project(c, project))
                    .map(|c| c.id.clone())
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn key_of(ids: &[ContainerId]) -> String {
    ids.iter()
        .map(|id| id.0.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

#[component]
fn LogPanelBody(targets: Vec<LogTarget>, #[props(default)] prefs: ViewPrefs) -> Element {
    let handle = use_log_session(targets.clone());
    let name = match targets.as_slice() {
        [only] => only.name.clone(),
        _ => "logs".to_string(),
    };
    rsx! {
        LogViewer { handle, view_name: name, merged: targets.len() > 1, prefs }
    }
}

/// A log view over these containers, self-contained: it opens its session when mounted, closes
/// it when it goes, and reopens when the set changes. Mount it wherever logs belong (the
/// inspector's Logs tab, the bottom panel).
#[component]
pub fn LogPanel(containers: Vec<ContainerId>, #[props(default)] prefs: ViewPrefs) -> Element {
    let state = use_context::<AppState>();
    let targets = targets_for(&state.containers.read(), &containers);
    let seeded = try_consume_context::<crate::services::logs::SeededLogs>().is_some();
    if targets.is_empty() && !seeded {
        return rsx! {
            div { class: "log-panel log-panel-empty",
                EmptyState {
                    title: "Select a container to follow its logs".to_string(),
                    body: "Pick one or more containers, or a project, and their logs stream here.".to_string(),
                    icon: "logs",
                }
            }
        };
    }
    let key = key_of(&containers);
    rsx! {
        div { class: "log-panel",
            // One-item loop: a changed set must remount the view so it opens a new session.
            for k in [key] {
                LogPanelBody { key: "{k}", targets: targets.clone(), prefs: prefs.clone() }
            }
        }
    }
}

/// The starting toggles and filters a dev scene asks for (context; the live app has none).
#[derive(Clone)]
pub struct ScenePrefs(pub ViewPrefs);

/// The Logs section.
#[component]
pub fn LogsScreen() -> Element {
    let state = use_context::<AppState>();
    let initial = ids_from_selection(
        &state.selection.peek(),
        &state.containers.peek(),
        &state.projects.peek(),
    );
    let mut chosen = use_signal(move || initial);
    let mut menu_open = use_signal(|| false);
    let ids = chosen.read().clone();
    let containers = state.containers.read().clone();
    let projects = state.projects.read().clone();
    let title = view_title(&containers, &projects, &ids);
    let series = series_of(&ids);
    let chips: Vec<(ContainerId, String, u8)> = ids
        .iter()
        .filter_map(|id| containers.iter().find(|c| &c.id == id))
        .map(|c| {
            let project = project_of(c, &projects).map(|p| p.name.as_str());
            (
                c.id.clone(),
                short_name(&c.name, project),
                series.get(&c.id).copied().unwrap_or(1),
            )
        })
        .collect();
    let menu = add_entries(&containers, &projects, &ids);
    let ids_for_panel = ids.clone();
    let scene_prefs = try_consume_context::<ScenePrefs>()
        .map(|p| p.0)
        .unwrap_or_default();
    let add_containers = containers.clone();
    let add_projects = projects.clone();
    rsx! {
        div { class: "logs-screen",
            div { class: "logs-head",
                h1 { class: "logs-title", "{title}" }
                div { class: "logs-chips", role: "list", "aria-label": "Containers",
                    for (id, label, hue) in chips {
                        span {
                            key: "{id.0}",
                            class: "log-chip",
                            role: "listitem",
                            style: "color: var(--color-series-{hue});",
                            span { class: "log-chip-icon", "aria-hidden": "true",
                                Icon { name: "logs", size: IconSize::Sm }
                            }
                            span { class: "log-chip-name", "{label}" }
                            button {
                                class: "log-chip-x",
                                r#type: "button",
                                "aria-label": "Stop following {label}",
                                onclick: {
                                    let id = id.clone();
                                    move |_| chosen.write().retain(|c| c != &id)
                                },
                                Icon { name: "x", size: IconSize::Sm }
                            }
                        }
                    }
                    if !menu.is_empty() {
                        div { class: "log-add",
                            button {
                                class: "log-chip log-chip-add",
                                r#type: "button",
                                "aria-expanded": menu_open().to_string(),
                                onclick: move |_| menu_open.set(!menu_open()),
                                Icon { name: "plus", size: IconSize::Sm }
                                span { "Add" }
                            }
                            if menu_open() {
                                div {
                                    class: "log-menu-scrim",
                                    onclick: move |_| menu_open.set(false),
                                }
                                div { class: "log-menu log-add-menu",
                                    Menu {
                                        items: menu.clone(),
                                        aria_label: "Add containers".to_string(),
                                        on_select: move |entry: String| {
                                            menu_open.set(false);
                                            let mut next = chosen.peek().clone();
                                            for id in ids_for_entry(&entry, &add_containers, &add_projects) {
                                                if !next.contains(&id) {
                                                    next.push(id);
                                                }
                                            }
                                            chosen.set(next);
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "logs-body",
                LogPanel { containers: ids_for_panel, prefs: scene_prefs }
            }
        }
    }
}

/// Menu entries that add containers: a whole project first (`p:<id>`), then single containers
/// (`c:<id>`), leaving out what is already followed.
fn add_entries(
    containers: &[Container],
    projects: &[Project],
    chosen: &[ContainerId],
) -> Vec<MenuEntry> {
    let mut entries = Vec::new();
    for project in projects {
        let missing = containers
            .iter()
            .filter(|c| in_project(c, project) && !chosen.contains(&c.id))
            .count();
        if missing > 0 {
            entries.push(MenuEntry::new(
                format!("p:{}", project.id.0),
                format!("Whole project \u{b7} {} ({missing})", project.name),
            ));
        }
    }
    let mut singles: Vec<&Container> = containers
        .iter()
        .filter(|c| !chosen.contains(&c.id))
        .collect();
    singles.sort_by(|a, b| a.name.cmp(&b.name));
    for container in singles {
        let mut entry = MenuEntry::new(format!("c:{}", container.id.0), container.name.clone());
        if entries.last().is_some_and(|e| e.id.starts_with("p:")) {
            entry = entry.separator_before();
        }
        entries.push(entry);
    }
    entries
}

/// The containers an add-menu entry stands for.
fn ids_for_entry(entry: &str, containers: &[Container], projects: &[Project]) -> Vec<ContainerId> {
    if let Some(project) = entry.strip_prefix("p:") {
        let project = ProjectId(project.to_string());
        projects
            .iter()
            .find(|p| p.id == project)
            .map(|p| {
                containers
                    .iter()
                    .filter(|c| in_project(c, p))
                    .map(|c| c.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    } else if let Some(id) = entry.strip_prefix("c:") {
        vec![ContainerId(id.to_string())]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
#[path = "logs_tests.rs"]
mod tests;
