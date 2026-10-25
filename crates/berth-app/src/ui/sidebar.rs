use dioxus::prelude::*;

use super::engine_switcher::scope_includes;
use super::sidebar_item::{project_dot, DotSpec, SidebarItem};
use crate::state::{AppState, EngineScope, Section, Selection};

/// One project row under PROJECTS.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarProject {
    pub id: berth_core::ProjectId,
    pub name: String,
    pub dot: DotSpec,
    /// Service count; `None` for a project with no services (pinned only).
    pub services: Option<usize>,
}

/// Everything the sidebar shows, derived from the signals by a pure
/// function so it is tested without a DOM.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarModel {
    pub projects: Vec<SidebarProject>,
    pub containers: usize,
    pub images: usize,
    pub volumes: usize,
    pub engines: usize,
}

/// Counts follow the engine scope (D81). Projects exclude the one
/// "Unassigned" bucket of loose containers: it is not something the user
/// opened, and the Containers table shows it as a group. Image and volume
/// counts come from each engine's own counts because the flat lists carry
/// no engine.
pub fn sidebar_model(
    engines: &[berth_core::Engine],
    scope: &EngineScope,
    projects: &[berth_core::Project],
    containers: &[berth_core::Container],
) -> SidebarModel {
    let projects = projects
        .iter()
        .filter(|p| scope_includes(scope, &p.engine))
        .filter(|p| !matches!(p.source, berth_core::ProjectSource::Loose))
        .map(|p| SidebarProject {
            id: p.id.clone(),
            name: p.name.clone(),
            dot: project_dot(&p.status),
            services: (!p.services.is_empty()).then_some(p.services.len()),
        })
        .collect();
    let in_scope = || engines.iter().filter(|e| scope_includes(scope, &e.id));
    SidebarModel {
        projects,
        containers: containers
            .iter()
            .filter(|c| scope_includes(scope, &c.engine))
            .count(),
        images: in_scope().map(|e| e.counts.images as usize).sum(),
        volumes: in_scope().map(|e| e.counts.volumes as usize).sum(),
        engines: engines.len(),
    }
}

#[component]
fn SectionItem(
    icon: &'static str,
    label: &'static str,
    target: Section,
    #[props(default)] count: Option<usize>,
) -> Element {
    let mut state = use_context::<AppState>();
    let selected = *state.section.read() == target;
    rsx! {
        SidebarItem {
            label: label.to_string(),
            icon,
            count,
            selected,
            onclick: move |_| state.section.set(target),
        }
    }
}

/// The 220 px navigation column (PLAN 7.1): sections in ⌘1…⌘9 order, the
/// project list with status dots, and the manage group.
#[component]
pub fn Sidebar() -> Element {
    let mut state = use_context::<AppState>();
    let model = sidebar_model(
        &state.engines.read(),
        &state.engine_scope.read(),
        &state.projects.read(),
        &state.containers.read(),
    );
    let section = *state.section.read();
    let selection = state.selection.read().clone();
    rsx! {
        nav { class: "sidebar", "aria-label": "Sections",
            SectionItem {
                icon: "folder",
                label: "Projects",
                target: Section::Projects,
                count: model.projects.len(),
            }
            div { class: "sidebar-group-label", "Projects" }
            for project in model.projects.iter() {
                {
                    let id = project.id.clone();
                    let selected = section == Section::Project
                        && selection == Selection::Project(project.id.clone());
                    rsx! {
                        SidebarItem {
                            key: "{project.id.0}",
                            label: project.name.clone(),
                            dot: project.dot,
                            count: project.services,
                            indent: true,
                            selected,
                            onclick: move |_| {
                                state.selection.set(Selection::Project(id.clone()));
                                state.section.set(Section::Project);
                            },
                        }
                    }
                }
            }
            div { class: "sidebar-separator", role: "separator" }
            SectionItem {
                icon: "box",
                label: "Containers",
                target: Section::Containers,
                count: model.containers,
            }
            SectionItem { icon: "logs", label: "Logs", target: Section::Logs }
            SectionItem {
                icon: "layers",
                label: "Images",
                target: Section::Images,
                count: model.images,
            }
            SectionItem {
                icon: "hard-drive",
                label: "Volumes",
                target: Section::Volumes,
                count: model.volumes,
            }
            SectionItem { icon: "network", label: "Networks", target: Section::Networks }
            SectionItem { icon: "file-text", label: "Templates", target: Section::Templates }
            div { class: "sidebar-separator", role: "separator" }
            SectionItem {
                icon: "server",
                label: "Engines",
                target: Section::Engines,
                count: model.engines,
            }
            SectionItem { icon: "cloud", label: "Registries", target: Section::Registries }
            SectionItem { icon: "activity", label: "Activity", target: Section::Activity }
        }
    }
}
