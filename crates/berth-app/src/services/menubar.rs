//! Menu bar item (D38, D80, ADR 0010; reshaped AD34): the pure `menu_model` that turns what the
//! engines service knows into the tray menu's content, the command ids the native items carry,
//! and the commands those ids run. The tray itself is `platform::use_tray`; nothing here touches
//! a macOS API.
//!
//! The order follows the frame "Menu bar / failing / 320": a summary line, the engines, then the
//! projects with their failing containers nested under them, then Open Berth and Quit.

use berth_core::{
    Container, ContainerId, ContainerState, Engine, EngineId, EngineKind, EngineState, Health,
    Project, ProjectId, ProjectSource, ProjectStatus, RemoteTransport, StatusKind, Subject, Tone,
};
use dioxus::prelude::*;

use crate::services::actions::{ActionsService, ContainerTarget, ProjectOp};
use crate::services::engines::EngineLink;
use crate::state::{AppState, Section, Selection, Toast, ToastTone};

/// Everything the menu shows, from the signals the engines service fills.
#[derive(Debug, Clone, PartialEq)]
pub struct AppSnapshot {
    pub engines: Vec<Engine>,
    pub projects: Vec<Project>,
    pub containers: Vec<Container>,
}

/// The status mark in the menu bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuIcon {
    AllGood,
    Failing,
    EngineStopped,
}

/// What a menu item does. The subject travels with the command: a menu click acts on the item's
/// own engine, project or container, never on the window's selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuCommand {
    EngineStart(EngineId),
    /// Opens the Stop confirm (with the window shown); stopping an engine always asks first.
    EngineAskStop(EngineId),
    EngineRetry(EngineId),
    ProjectStart(ProjectId),
    ProjectStop(ProjectId),
    ProjectOpen(ProjectId),
    ShowLogs(ContainerId),
    Restart(ContainerId),
    OpenBerth,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuItem {
    Summary {
        text: String,
        tone: Tone,
    },
    Header(&'static str),
    /// One engine, project or container. Native menus hold no buttons in a row, so the platform
    /// layer draws it as a submenu whose items are `commands`.
    Row {
        label: String,
        detail: String,
        tone: Tone,
        nested: bool,
        commands: Vec<(String, MenuCommand)>,
    },
    Separator,
    Command {
        label: &'static str,
        command: MenuCommand,
    },
}

/// The tray's whole content. `PartialEq` is load-bearing: the tray rebuilds only when it changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuModel {
    pub icon: MenuIcon,
    pub items: Vec<MenuItem>,
}

// ---------------------------------------------------------------------
// Command ids
// ---------------------------------------------------------------------

/// Every tray item id starts with this, so a click on one is told apart from the app menu's.
pub const ID_PREFIX: &str = "tray:";

/// The native item id of a command: `tray:<verb>[:<subject id>]`. Subject ids may hold colons
/// (`colima:default`), so a parse splits at the first one only.
#[must_use]
pub fn command_id(command: &MenuCommand) -> String {
    let (verb, subject): (&str, Option<&str>) = match command {
        MenuCommand::EngineStart(id) => ("engine.start", Some(&id.0)),
        MenuCommand::EngineAskStop(id) => ("engine.stop", Some(&id.0)),
        MenuCommand::EngineRetry(id) => ("engine.retry", Some(&id.0)),
        MenuCommand::ProjectStart(id) => ("project.start", Some(&id.0)),
        MenuCommand::ProjectStop(id) => ("project.stop", Some(&id.0)),
        MenuCommand::ProjectOpen(id) => ("project.open", Some(&id.0)),
        MenuCommand::ShowLogs(id) => ("container.logs", Some(&id.0)),
        MenuCommand::Restart(id) => ("container.restart", Some(&id.0)),
        MenuCommand::OpenBerth => ("open", None),
        MenuCommand::Quit => ("quit", None),
    };
    match subject {
        Some(subject) => format!("{ID_PREFIX}{verb}:{subject}"),
        None => format!("{ID_PREFIX}{verb}"),
    }
}

/// The command an item id names; `None` for anything that is not a tray id.
#[must_use]
pub fn parse_command_id(id: &str) -> Option<MenuCommand> {
    let rest = id.strip_prefix(ID_PREFIX)?;
    let (verb, subject) = match rest.split_once(':') {
        Some((verb, subject)) => (verb, Some(subject)),
        None => (rest, None),
    };
    let engine = || subject.map(|s| EngineId(s.to_string()));
    let project = || subject.map(|s| ProjectId(s.to_string()));
    let container = || subject.map(|s| ContainerId(s.to_string()));
    match verb {
        "engine.start" => engine().map(MenuCommand::EngineStart),
        "engine.stop" => engine().map(MenuCommand::EngineAskStop),
        "engine.retry" => engine().map(MenuCommand::EngineRetry),
        "project.start" => project().map(MenuCommand::ProjectStart),
        "project.stop" => project().map(MenuCommand::ProjectStop),
        "project.open" => project().map(MenuCommand::ProjectOpen),
        "container.logs" => container().map(MenuCommand::ShowLogs),
        "container.restart" => container().map(MenuCommand::Restart),
        "open" if subject.is_none() => Some(MenuCommand::OpenBerth),
        "quit" if subject.is_none() => Some(MenuCommand::Quit),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------

/// The project `c` belongs to: the id the engines service stamped on it, or, for a container the
/// service did not link, Compose's project name on the same engine when exactly one project has
/// it (two folders can share a name; then neither claims it).
fn owner<'a>(c: &Container, projects: &'a [Project]) -> Option<&'a Project> {
    if let Some(id) = &c.project {
        return projects.iter().find(|p| p.id == *id);
    }
    let name = c.compose_project.as_deref()?;
    let mut named = projects
        .iter()
        .filter(|p| p.engine == c.engine && p.name == name);
    let first = named.next()?;
    named.next().is_none().then_some(first)
}

/// A container that failed: exited with an error, or dead.
fn is_failed(c: &Container) -> bool {
    status_of(c).tone() == Tone::Danger
}

/// A container the menu lists under its project: it failed or it is unwell (restarting,
/// unhealthy).
fn is_trouble(c: &Container) -> bool {
    is_failed(c) || c.state == ContainerState::Restarting || c.health == Some(Health::Unhealthy)
}

fn status_of(c: &Container) -> StatusKind {
    berth_core::status_of(c.state, c.health, c.exit_code)
}

/// Danger for failed, Warning for restarting or unhealthy.
fn container_tone(c: &Container) -> Tone {
    if is_failed(c) {
        Tone::Danger
    } else {
        Tone::Warning
    }
}

/// The projects the menu lists: pinned, or with something running (or a status that is not
/// stopped). Stopped, unpinned projects stay out of the menu and out of the failure count.
fn is_shown(p: &Project, own: &[&Container]) -> bool {
    p.pinned
        || !matches!(p.status, ProjectStatus::Stopped | ProjectStatus::NotCreated)
        || own.iter().any(|c| c.state == ContainerState::Running)
}

/// The projects with a menu row (`Loose` is the Containers screen's bucket, never a project).
fn real_projects(s: &AppSnapshot) -> Vec<Project> {
    s.projects
        .iter()
        .filter(|p| !matches!(p.source, ProjectSource::Loose))
        .cloned()
        .collect()
}

fn own_containers<'a>(p: &Project, s: &'a AppSnapshot, real: &[Project]) -> Vec<&'a Container> {
    s.containers
        .iter()
        .filter(|c| owner(c, real).is_some_and(|o| o.id == p.id))
        .collect()
}

/// AD36: the containers worth a glance. In a project the menu shows: failed (exited non-zero),
/// restarting or unhealthy. Outside every project: only restarting or unhealthy; a loose
/// container that exited never counts, however it exited. The summary, the icon and the loose
/// rows all use this one filter.
fn actionable<'a>(s: &'a AppSnapshot, real: &[Project]) -> Vec<&'a Container> {
    let shown: Vec<&ProjectId> = real
        .iter()
        .filter(|p| is_shown(p, &own_containers(p, s, real)))
        .map(|p| &p.id)
        .collect();
    s.containers
        .iter()
        .filter(|c| match owner(c, real) {
            Some(p) => shown.contains(&&p.id) && is_trouble(c),
            None => c.state == ContainerState::Restarting || c.health == Some(Health::Unhealthy),
        })
        .collect()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn engine_kind_label(kind: &EngineKind) -> &'static str {
    match kind {
        EngineKind::Colima { .. } => "Colima",
        EngineKind::Remote {
            transport: RemoteTransport::Ssh { .. },
        } => "SSH",
        EngineKind::Remote {
            transport: RemoteTransport::Tcp { .. },
        } => "TCP",
        EngineKind::Socket { .. } => "Docker",
    }
}

fn engine_row(e: &Engine) -> MenuItem {
    let kind = engine_kind_label(&e.kind);
    let colima = matches!(e.kind, EngineKind::Colima { .. });
    let (state, tone, commands) = match &e.state {
        EngineState::Running => (
            "Running",
            Tone::Success,
            if colima {
                vec![(
                    "Stop\u{2026}".to_string(),
                    MenuCommand::EngineAskStop(e.id.clone()),
                )]
            } else {
                vec![]
            },
        ),
        EngineState::Stopped => (
            "Stopped",
            Tone::Stopped,
            if colima {
                vec![("Start".to_string(), MenuCommand::EngineStart(e.id.clone()))]
            } else {
                vec![]
            },
        ),
        EngineState::Starting => ("Starting\u{2026}", Tone::Warning, vec![]),
        EngineState::Stopping => ("Stopping\u{2026}", Tone::Warning, vec![]),
        EngineState::Unreachable { .. } => (
            "Unreachable",
            Tone::Danger,
            vec![("Retry".to_string(), MenuCommand::EngineRetry(e.id.clone()))],
        ),
        EngineState::Unsupported { .. } => ("Unsupported", Tone::Warning, vec![]),
    };
    MenuItem::Row {
        label: e.name.clone(),
        detail: format!("{kind} \u{b7} {state}"),
        tone,
        nested: false,
        commands,
    }
}

fn container_row(c: &Container, nested: bool) -> MenuItem {
    MenuItem::Row {
        label: c.name.clone(),
        detail: status_of(c).label(),
        tone: container_tone(c),
        nested,
        commands: vec![
            ("Show logs".to_string(), MenuCommand::ShowLogs(c.id.clone())),
            ("Restart".to_string(), MenuCommand::Restart(c.id.clone())),
        ],
    }
}

/// One project row: tone and the "N of M running" text, from the project's own containers (the
/// project's `status` counts only what Compose declares).
fn project_row(p: &Project, own: &[&Container]) -> (MenuItem, u8) {
    let trouble = own.iter().any(|c| is_trouble(c));
    let running = own
        .iter()
        .filter(|c| c.state == ContainerState::Running)
        .count();
    let counts = |running: usize, total: usize| format!("{running} of {total} running");
    let (detail, tone, running_now) = match p.status {
        ProjectStatus::Stopped => ("Stopped".to_string(), Tone::Stopped, false),
        ProjectStatus::NotCreated => ("Not created".to_string(), Tone::Stopped, false),
        ProjectStatus::Failing => {
            let (r, t) = if own.is_empty() {
                (0, p.services.len())
            } else {
                (running, own.len())
            };
            (counts(r, t), Tone::Danger, r > 0)
        }
        ProjectStatus::Partial {
            running: r,
            total: t,
        } => {
            let (r, t) = if own.is_empty() {
                (usize::from(r), usize::from(t))
            } else {
                (running, own.len())
            };
            (counts(r, t), Tone::Warning, r > 0)
        }
        ProjectStatus::AllRunning => {
            let tone = if trouble {
                Tone::Warning
            } else {
                Tone::Success
            };
            let (r, t) = if own.is_empty() {
                (p.services.len(), p.services.len())
            } else {
                (running, own.len())
            };
            (counts(r, t), tone, true)
        }
    };
    let toggle = if running_now {
        (
            "Stop project".to_string(),
            MenuCommand::ProjectStop(p.id.clone()),
        )
    } else {
        (
            "Start project".to_string(),
            MenuCommand::ProjectStart(p.id.clone()),
        )
    };
    let rank = match tone {
        Tone::Danger => 0,
        Tone::Warning => 1,
        Tone::Success | Tone::Info => 2,
        Tone::Stopped => 3,
    };
    (
        MenuItem::Row {
            label: p.name.clone(),
            detail,
            tone,
            nested: false,
            commands: vec![
                toggle,
                (
                    "Open in Berth".to_string(),
                    MenuCommand::ProjectOpen(p.id.clone()),
                ),
            ],
        },
        rank,
    )
}

/// The summary line and its tone, and the icon.
fn summary(s: &AppSnapshot, actionable: &[&Container]) -> (String, Tone, MenuIcon) {
    let failing = actionable.iter().filter(|c| is_failed(c)).count();
    let unhealthy = actionable.len() - failing;
    let unreachable = s
        .engines
        .iter()
        .filter(|e| matches!(e.state, EngineState::Unreachable { .. }))
        .count();
    let mut parts = Vec::new();
    if failing > 0 {
        parts.push(format!("{failing} failing"));
    }
    if unhealthy > 0 {
        parts.push(format!("{unhealthy} unhealthy"));
    }
    if unreachable > 0 {
        parts.push(plural(
            unreachable,
            "engine unreachable",
            "engines unreachable",
        ));
    }
    if !parts.is_empty() {
        let tone = if failing > 0 || unreachable > 0 {
            Tone::Danger
        } else {
            Tone::Warning
        };
        return (parts.join(" \u{b7} "), tone, MenuIcon::Failing);
    }
    let live = s.engines.iter().any(|e| {
        matches!(
            e.state,
            EngineState::Running | EngineState::Starting | EngineState::Stopping
        )
    });
    if s.engines.is_empty() {
        (
            "No engines yet".to_string(),
            Tone::Stopped,
            MenuIcon::AllGood,
        )
    } else if !live {
        (
            plural(s.engines.len(), "engine stopped", "engines stopped"),
            Tone::Stopped,
            MenuIcon::EngineStopped,
        )
    } else {
        ("All good".to_string(), Tone::Success, MenuIcon::AllGood)
    }
}

/// The tray menu for what the engines know now. Failing first: a project with failing
/// containers leads the list and carries them nested; stopped projects only show when pinned.
#[must_use]
pub fn menu_model(s: &AppSnapshot) -> MenuModel {
    let real = real_projects(s);
    let actionable = actionable(s, &real);
    let (text, tone, icon) = summary(s, &actionable);
    let mut items = vec![MenuItem::Summary { text, tone }];

    if !s.engines.is_empty() {
        items.push(MenuItem::Header("Engines"));
        items.extend(s.engines.iter().map(engine_row));
    }

    // "Unassigned" is the Containers screen's bucket for loose containers, not a project you can
    // start or stop; its failures show as loose rows below.
    let mut projects: Vec<(u8, &Project, Vec<&Container>)> = real
        .iter()
        .filter_map(|p| {
            let own = own_containers(p, s, &real);
            is_shown(p, &own).then(|| (project_row(p, &own).1, p, own))
        })
        .collect();
    projects.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    let loose: Vec<&Container> = actionable
        .iter()
        .copied()
        .filter(|c| owner(c, &real).is_none())
        .collect();
    if !projects.is_empty() || !loose.is_empty() {
        items.push(MenuItem::Header("Projects"));
    }
    for (_, project, own) in &projects {
        items.push(project_row(project, own).0);
        let mut trouble: Vec<&&Container> = own.iter().filter(|c| is_trouble(c)).collect();
        // Failed before unwell, then by name.
        trouble.sort_by(|a, b| {
            is_failed(b)
                .cmp(&is_failed(a))
                .then_with(|| a.name.cmp(&b.name))
        });
        items.extend(trouble.into_iter().map(|c| container_row(c, true)));
    }
    items.extend(loose.into_iter().map(|c| container_row(c, false)));

    items.push(MenuItem::Separator);
    items.push(MenuItem::Command {
        label: "Open Berth",
        command: MenuCommand::OpenBerth,
    });
    items.push(MenuItem::Command {
        label: "Quit Berth",
        command: MenuCommand::Quit,
    });
    MenuModel { icon, items }
}

// ---------------------------------------------------------------------
// Running a command
// ---------------------------------------------------------------------

/// The two window-level effects a command can have, so a test can stand in for the platform.
pub trait MenuHost {
    fn show_window(&self);
    fn quit(&self);
}

/// The real thing: the platform layer.
pub struct PlatformHost;

impl MenuHost for PlatformHost {
    fn show_window(&self) {
        crate::platform::show_window();
    }
    fn quit(&self) {
        crate::platform::quit();
    }
}

fn toast(state: &mut AppState, text: String) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone: ToastTone::Danger,
        action: None,
    });
}

/// Run one command on its own subject. Project, container and engine-start commands work with the
/// window closed; the ones that need the user (the Stop confirm, logs, Open) show it first.
pub fn run_command(
    host: &dyn MenuHost,
    state: &mut AppState,
    actions: Option<&ActionsService>,
    link: Option<&EngineLink>,
    command: &MenuCommand,
) {
    match command {
        MenuCommand::OpenBerth => host.show_window(),
        MenuCommand::Quit => host.quit(),
        MenuCommand::EngineAskStop(id) => {
            let found = state.engines.peek().iter().find(|e| e.id == *id).cloned();
            match found {
                Some(engine) => {
                    host.show_window();
                    crate::screens::engines::ask(state, &engine, berth_core::ColimaOp::Stop);
                }
                None => toast(state, "This engine no longer exists".to_string()),
            }
        }
        MenuCommand::EngineStart(id) => {
            let found = state.engines.peek().iter().find(|e| e.id == *id).cloned();
            match found {
                Some(engine) => crate::screens::engines::start_engine(state, actions, &engine),
                None => toast(state, "This engine no longer exists".to_string()),
            }
        }
        MenuCommand::EngineRetry(id) => {
            let found = state.engines.peek().iter().find(|e| e.id == *id).cloned();
            match found {
                Some(engine) => crate::screens::engines::retry(state, link, &engine),
                None => toast(state, "This engine no longer exists".to_string()),
            }
        }
        MenuCommand::ProjectStart(id) | MenuCommand::ProjectStop(id) => {
            let op = if matches!(command, MenuCommand::ProjectStart(_)) {
                ProjectOp::Start
            } else {
                ProjectOp::Stop
            };
            let found = state.projects.peek().iter().find(|p| p.id == *id).cloned();
            let result = match (found, actions) {
                (Some(project), Some(actions)) => actions.project(state, &project, op),
                (None, _) => Err("This project no longer exists".to_string()),
                (_, None) => Err("Actions are not running".to_string()),
            };
            if let Err(reason) = result {
                toast(state, reason);
            }
        }
        MenuCommand::ProjectOpen(id) => {
            host.show_window();
            crate::services::search::open(state, &Subject::Project(id.clone()));
        }
        MenuCommand::ShowLogs(id) => {
            host.show_window();
            state.section.set(Section::Containers);
            state.selection.set(Selection::Containers(vec![id.clone()]));
            crate::keys::dispatch(state, "container.logs");
        }
        MenuCommand::Restart(id) => {
            let found = state
                .containers
                .peek()
                .iter()
                .find(|c| c.id == *id)
                .cloned();
            let result = match (found, actions) {
                (Some(container), Some(actions)) => actions.container(
                    state,
                    &ContainerTarget::from(&container),
                    berth_core::ContainerAction::Restart { timeout_s: None },
                ),
                (None, _) => Err("This container no longer exists".to_string()),
                (_, None) => Err("Actions are not running".to_string()),
            };
            if let Err(reason) = result {
                toast(state, reason);
            }
        }
    }
}

/// A click on a native item whose id starts with `tray:`. `false` when the id is not one of ours.
pub fn handle_item(
    state: &mut AppState,
    actions: Option<&ActionsService>,
    link: Option<&EngineLink>,
    id: &str,
) -> bool {
    let Some(command) = parse_command_id(id) else {
        return false;
    };
    run_command(&PlatformHost, state, actions, link, &command);
    true
}

// ---------------------------------------------------------------------
// The hook
// ---------------------------------------------------------------------

/// The App root's one call: derive the model from the signals, hand it to the tray, and keep the
/// dock icon in step with the setting. Scenes show a frame's state, so they get no tray.
pub fn use_menubar(state: AppState) {
    let model = use_memo(move || {
        menu_model(&AppSnapshot {
            engines: state.engines.read().clone(),
            projects: state.projects.read().clone(),
            containers: state.containers.read().clone(),
        })
    });
    if std::env::var_os("BERTH_SCENE").is_none() {
        crate::platform::use_tray(model.into());
        // The Dock's Quit ends the event loop without `platform::quit`; this is what its last
        // snapshot save reads (AD36).
        use_hook(move || {
            if let Some(services) = try_consume_context::<crate::services::Services>() {
                crate::platform::register_final_save(state, services.store);
            }
        });
    }
    use_effect(move || {
        let show = state.settings.read().show_dock_icon;
        crate::platform::set_dock_icon_visible(show);
    });
}

#[cfg(test)]
#[path = "menubar_tests.rs"]
mod tests;
