//! Project detail (P5-T6, frame "Project / files / 1440"): one project's
//! own screen, reached from the sidebar and the Projects table.
//!
//! The header names the project and its state, then the folder, the git
//! branch and what runs; its buttons are Start or Stop project, Restart
//! project, Rebuild and a ⋯ menu (Pull images, Open folder, Open in editor,
//! Reveal in Finder, Delete project). Four tabs sit under it: Services (a
//! `ContainerRow` each), Files (the folder, with `.env` values hidden), Logs
//! (every container's logs, merged) and Activity (the feed for this project).
//!
//! The header's model is [`detail_model`], pure over borrowed state like the
//! Projects row it reuses (`ui::project_row::row_model`), so the buttons,
//! reasons and pending labels agree with the table. Every button runs
//! through the actions service (D34); nothing here calls an engine (D23).
//!
//! States: loading (rows are skeletons), gone (the project left the engine's
//! list), not created (services but no containers), partly running (a note
//! names what is down) and a Compose file that does not load (a banner with
//! the line and message `docker compose config` gives, and the line marked
//! in the Files tab).

use dioxus::prelude::*;

use berth_core::{Project, ProjectId, ProjectStatus};

mod activity_tab;
mod files;
mod git_panel;
mod services_tab;
pub(crate) mod tree;

pub use services_tab::{down_note, project_container_ids};

use crate::screens::logs::LogPanel;
use crate::services::actions::{ActionsService, ProjectOp, StateView};
use crate::services::project_files::{
    default_editor_target, editor_for, launch, open_folder_command, open_in_editor_command,
    parse_compose_problem, reveal_command, search_path_finder, ComposeProblem, Editor,
};
use crate::state::{AppState, Overlay, PendingKey, Section, Selection};
use crate::ui::project_row::{row_model, run_row_op, ProjectRowModel};
use crate::ui::sidebar_item::{project_dot, Dot, DotSpec};
use crate::ui::{
    tone_class, Banner, BannerTone, Button, ButtonVariant, EmptyState, IconButton, InlineMessage,
    Menu, MenuEntry, SkeletonRows, TabItem, Tabs,
};

/// The screen's CSS: this screen's own rules, and the Containers table's for
/// the Services tab's rows.
pub const PROJECT_DETAIL_CSS: &str = include_str!("../../assets/css/project_detail.css");

/// The four tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Services,
    Files,
    Logs,
    Activity,
}

impl Tab {
    pub const ALL: [Self; 4] = [Self::Services, Self::Files, Self::Logs, Self::Activity];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Services => "services",
            Self::Files => "files",
            Self::Logs => "logs",
            Self::Activity => "activity",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Services => "Services",
            Self::Files => "Files",
            Self::Logs => "Logs",
            Self::Activity => "Activity",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|t| t.id() == id)
            .unwrap_or_default()
    }
}

/// Whether the project's Compose file loads.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Validity {
    /// Not asked yet, or nothing to ask (no folder, no Compose file, no service to ask).
    #[default]
    Unknown,
    Checking,
    Valid,
    Invalid(ComposeProblem),
}

/// What a scene or a test puts in place of the live sources: the tab to open
/// on, the answer to the Compose check, the editor and the activity feed.
#[derive(Clone, Default)]
pub struct ProjectDetailSeed {
    pub tab: Tab,
    pub validity: Option<Validity>,
    pub editor: Option<Editor>,
    pub activity: Option<Vec<berth_core::ActivityEntry>>,
}

/// What the main area draws for the selected project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenState {
    /// The first list has not arrived.
    Loading,
    /// The engine's list does not hold this project (any more).
    Gone,
    Ready,
}

#[must_use]
pub fn screen_state(
    project: Option<&Project>,
    engines: &[berth_core::Engine],
    snapshot_stale: bool,
) -> ScreenState {
    match project {
        Some(_) => ScreenState::Ready,
        None if engines.is_empty() || snapshot_stale => ScreenState::Loading,
        None => ScreenState::Gone,
    }
}

/// The status badge's words.
#[must_use]
pub fn status_label(status: ProjectStatus) -> String {
    match status {
        ProjectStatus::AllRunning => "Running".to_string(),
        ProjectStatus::Partial { running, total } => format!("{running} of {total} running"),
        ProjectStatus::Stopped => "Stopped".to_string(),
        ProjectStatus::Failing => "Failing".to_string(),
        ProjectStatus::NotCreated => "Not created".to_string(),
    }
}

/// Everything the header draws, from the project and the state around it.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailModel {
    pub row: ProjectRowModel,
    pub status: DotSpec,
    pub status_label: String,
    /// `feat/checkout-v2 *`: the branch, with a star when the tree is dirty.
    pub branch: Option<String>,
    /// `default engine · 5 services · 2 problems`.
    pub subline: String,
    pub folder: Option<std::path::PathBuf>,
}

#[must_use]
pub fn detail_model(
    project: &Project,
    view: &StateView<'_>,
    problems: &[berth_core::Problem],
    home: &std::path::Path,
) -> DetailModel {
    let row = row_model(project, view, problems, home);
    let branch = project.git.as_ref().and_then(|git| {
        git.branch.as_ref().map(|b| {
            if git.dirty {
                format!("{b} *")
            } else {
                b.clone()
            }
        })
    });
    let mut parts = vec![
        format!("{} engine", row.engine_name),
        row.services_label.clone(),
    ];
    if row.problem_count > 0 {
        parts.push(if row.problem_count == 1 {
            "1 problem".to_string()
        } else {
            format!("{} problems", row.problem_count)
        });
    }
    DetailModel {
        status: project_dot(&project.status),
        status_label: status_label(project.status),
        branch,
        subline: parts.join(" \u{b7} "),
        folder: project.folder.clone(),
        row,
    }
}

/// "Compose file `compose.yaml` line 4: …" for the banner.
#[must_use]
pub fn invalid_message(problem: &ComposeProblem, compose_files: &[std::path::PathBuf]) -> String {
    let file = problem.file.clone().or_else(|| {
        compose_files
            .first()
            .and_then(|f| f.file_name())
            .map(|n| n.to_string_lossy().into_owned())
    });
    match file {
        Some(file) => format!("{file} does not load, {}", problem.text()),
        None => format!("The Compose file does not load, {}", problem.text()),
    }
}

/// The ⋯ menu of the header.
#[must_use]
pub fn menu_entries(model: &DetailModel, editor: &Editor) -> Vec<MenuEntry> {
    let row = &model.row;
    let no_folder = || "This project has no folder on this Mac".to_string();
    let with = |entry: MenuEntry, reason: Option<String>| match reason {
        Some(reason) => entry.disabled(reason),
        None => entry,
    };
    let folder_reason = model.folder.is_none().then(no_folder);
    let delete = MenuEntry::new("delete", "Delete project\u{2026}")
        .icon("trash-2")
        .danger()
        .separator_before();
    vec![
        with(
            MenuEntry::new("pull", "Pull images").icon("arrow-down-to-line"),
            row.pull_reason.clone(),
        ),
        with(
            MenuEntry::new("open-folder", "Open folder")
                .icon("folder")
                .separator_before(),
            folder_reason.clone(),
        ),
        with(
            MenuEntry::new("open-editor", format!("Open in {}", short_editor(editor)))
                .icon("open-external"),
            folder_reason.clone(),
        ),
        with(
            MenuEntry::new("reveal", "Reveal in Finder").icon("folder"),
            folder_reason,
        ),
        with(delete, row.delete_reason.clone()),
    ]
}

/// "Visual Studio Code" from "Visual Studio Code (detected)"; the default app stays "the default app".
fn short_editor(editor: &Editor) -> String {
    editor
        .label
        .strip_suffix(" (detected)")
        .unwrap_or(&editor.label)
        .to_string()
}

// ---------------------------------------------------------------------
// Open folder, Open in editor (actions `project.open_folder`, `project.open_in_editor`)
// ---------------------------------------------------------------------

/// Show the selected project's folder in Finder.
pub fn open_selected_folder(state: &AppState) {
    let Some(folder) =
        crate::services::project_files::selected_project(state).and_then(|p| p.folder)
    else {
        return;
    };
    if let Some(command) = open_folder_command(&search_path_finder(), &folder) {
        launch(command);
    }
}

/// Open the selected project's Compose file (else its folder) in the editor.
pub fn open_selected_in_editor(state: &AppState) {
    let Some(project) = crate::services::project_files::selected_project(state) else {
        return;
    };
    let Some(folder) = project.folder.clone() else {
        return;
    };
    let Some(file) = default_editor_target(&project.compose_files, &folder) else {
        return;
    };
    let setting = state.settings.peek().editor.clone();
    if let Some(command) =
        open_in_editor_command(&search_path_finder(), setting.as_deref(), &folder, &file)
    {
        launch(command);
    }
}

fn run_menu_entry(
    state: &mut AppState,
    service: Option<&ActionsService>,
    model: &DetailModel,
    editor_setting: Option<&str>,
    entry: &str,
) {
    let id = &model.row.id;
    match entry {
        "pull" => run_row_op(
            state,
            service,
            id,
            model.row.loose.as_ref(),
            ProjectOp::PullImages,
        ),
        "open-folder" => open_selected_folder(state),
        "open-editor" => {
            if let Some(folder) = &model.folder {
                let find = search_path_finder();
                let files = state
                    .projects
                    .peek()
                    .iter()
                    .find(|p| p.id == *id)
                    .map(|p| p.compose_files.clone())
                    .unwrap_or_default();
                if let Some(file) = default_editor_target(&files, folder) {
                    if let Some(command) =
                        open_in_editor_command(&find, editor_setting, folder, &file)
                    {
                        launch(command);
                    }
                }
            }
        }
        "reveal" => {
            if let Some(command) = model
                .folder
                .as_deref()
                .and_then(|folder| reveal_command(&search_path_finder(), folder))
            {
                launch(command);
            }
        }
        "delete" => state
            .overlays
            .write()
            .push(Overlay::DeleteProject(id.clone())),
        _ => {}
    }
}

// ---------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------

#[component]
fn StatusBadgeFor(spec: DotSpec, label: String) -> Element {
    let class = format!("status-badge {}", tone_class(spec.tone));
    rsx! {
        span { class,
            Dot { spec }
            span { "{label}" }
        }
    }
}

#[component]
pub fn ProjectDetailScreen(id: ProjectId) -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_consume_context::<ActionsService>();
    let seed = use_hook(|| try_consume_context::<ProjectDetailSeed>().unwrap_or_default());
    let mut tab = use_signal(|| seed.tab);
    let mut menu_open = use_signal(|| false);
    let mut validity = use_signal(|| seed.validity.clone().unwrap_or_default());
    let editor_setting = state.settings.read().editor.clone();
    let terminal_setting = state.settings.read().terminal_app.clone();
    let editor = use_hook({
        let seeded = seed.editor.clone();
        let setting = editor_setting.clone();
        move || seeded.unwrap_or_else(|| editor_for(&search_path_finder(), setting.as_deref()))
    });

    // Does the Compose file load? Asked when the screen opens and when the
    // Files tab is refreshed; a scene or test answers for it.
    let check = use_callback({
        let id = id.clone();
        let service = actions.clone();
        move |()| {
            let Some(service) = service.clone() else {
                return;
            };
            let found = state.projects.peek().iter().find(|p| p.id == id).cloned();
            let Some(project) = found else { return };
            let (Some(folder), false) = (project.folder.clone(), project.compose_files.is_empty())
            else {
                return;
            };
            if *validity.peek() == Validity::Unknown {
                validity.set(Validity::Checking);
            }
            spawn(async move {
                let answer = service
                    .validate_compose(&folder, &project.compose_files)
                    .await;
                validity.set(match answer {
                    Ok(()) => Validity::Valid,
                    Err(raw) => Validity::Invalid(parse_compose_problem(&raw, &folder)),
                });
            });
        }
    });
    use_hook({
        let seeded = seed.validity.is_some();
        move || {
            if !seeded {
                check.call(());
            }
        }
    });

    let engines = state.engines.read();
    let containers = state.containers.read();
    let projects = state.projects.read();
    let pending = state.pending.read();
    let problems = state.problems.read();
    let project = projects.iter().find(|p| p.id == id);
    let screen = screen_state(project, &engines, *state.snapshot_stale.read());
    let Some(project) = project.filter(|_| screen == ScreenState::Ready) else {
        return rsx! {
            style { "{PROJECT_DETAIL_CSS}" }
            section { class: "project-detail", "aria-label": "Project",
                if screen == ScreenState::Loading {
                    div { class: "pd-loading", SkeletonRows { count: 6_usize } }
                } else {
                    div { class: "pd-empty",
                        EmptyState {
                            title: "This project is not here any more".to_string(),
                            body: "The engine no longer lists it. Its folder, if there is one, was not touched.".to_string(),
                            icon: "folder",
                            action_label: "Back to Projects".to_string(),
                            onaction: move |()| {
                                state.selection.set(Selection::None);
                                state.section.set(Section::Projects);
                            },
                        }
                    }
                }
            }
        };
    };
    let home = berth_sys::paths::home();
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
    let mut sorted_problems: Vec<berth_core::Problem> = problems.clone();
    sorted_problems.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(b.first_seen.cmp(&a.first_seen))
    });
    let model = detail_model(project, &view, &sorted_problems, &home);
    let row = &model.row;
    let container_ids = project_container_ids(project, &containers);
    let problem = match &*validity.read() {
        Validity::Invalid(problem) => Some(problem.clone()),
        _ => None,
    };
    let banner = problem
        .as_ref()
        .map(|p| invalid_message(p, &project.compose_files));
    let busy = row.pending.is_some();
    let pending_of = |action: &str| row.pending.as_ref().filter(|p| p.action == action);
    let restart_pending = pending_of("project.restart");
    let primary_pending = pending_of(row.primary.op().action_id());
    let rebuild_pending = pending_of("project.rebuild");
    let entries = menu_entries(&model, &editor);
    let selected_tab = *tab.read();
    let tabs: Vec<TabItem> = Tab::ALL
        .into_iter()
        .map(|t| TabItem::new(t.id(), t.label()))
        .collect();
    let start_reason = row.primary_reason.clone();
    let failure_key = row.failure.as_ref().map(|f| PendingKey {
        subject: f.subject.clone(),
        action: f.action,
    });

    let run = {
        let id = id.clone();
        let service = actions.clone();
        let loose = row.loose.clone();
        move |state: &mut AppState, op: ProjectOp| {
            run_row_op(state, service.as_ref(), &id, loose.as_ref(), op);
        }
    };
    let run_primary = {
        let run = run.clone();
        let op = row.primary.op();
        move |_| run(&mut state, op)
    };
    let run_restart = {
        let run = run.clone();
        move |_| run(&mut state, ProjectOp::Restart)
    };
    let run_rebuild = {
        let run = run.clone();
        move |_| run(&mut state, ProjectOp::Rebuild)
    };
    let run_start = {
        let run = run.clone();
        move |()| run(&mut state, ProjectOp::Start)
    };
    let menu_model = model.clone();
    let menu_service = actions.clone();
    let menu_setting = editor_setting.clone();
    let primary_icon = match row.primary {
        crate::ui::project_row::PrimaryOp::Start => "play",
        crate::ui::project_row::PrimaryOp::Stop => "square",
    };
    let primary_variant = match row.primary {
        crate::ui::project_row::PrimaryOp::Start => ButtonVariant::Primary,
        crate::ui::project_row::PrimaryOp::Stop => ButtonVariant::Secondary,
    };
    let folder_text = row.path_full.clone();

    rsx! {
        style { "{PROJECT_DETAIL_CSS}" }
        section { class: "project-detail", "aria-label": "Project {project.name}",
            div { class: "pd-header",
                div { class: "pd-title-row",
                    h1 { class: "pd-title", "{project.name}" }
                    StatusBadgeFor { spec: model.status, label: model.status_label.clone() }
                    span { class: "pd-spacer" }
                    Button {
                        label: primary_pending.map_or_else(|| row.primary_label.clone(), |p| p.label.clone()),
                        icon: primary_icon,
                        variant: primary_variant,
                        loading: primary_pending.is_some(),
                        disabled: busy || row.primary_reason.is_some(),
                        disabled_reason: row.primary_reason.clone(),
                        onclick: run_primary,
                    }
                    Button {
                        label: restart_pending.map_or_else(|| "Restart project".to_string(), |p| p.label.clone()),
                        icon: "rotate-cw",
                        variant: ButtonVariant::Secondary,
                        loading: restart_pending.is_some(),
                        disabled: busy || row.restart_reason.is_some(),
                        disabled_reason: row.restart_reason.clone(),
                        onclick: run_restart,
                    }
                    Button {
                        label: rebuild_pending.map_or_else(|| "Rebuild".to_string(), |p| p.label.clone()),
                        icon: "box",
                        variant: ButtonVariant::Secondary,
                        loading: rebuild_pending.is_some(),
                        disabled: busy || row.rebuild_reason.is_some(),
                        disabled_reason: row.rebuild_reason.clone(),
                        onclick: run_rebuild,
                    }
                    span { class: "pd-more",
                        IconButton {
                            icon: "more-horizontal",
                            label: format!("More actions for {}", project.name),
                            pressed: Some(*menu_open.read()),
                            onclick: move |_| {
                                let open = *menu_open.read();
                                menu_open.set(!open);
                            },
                        }
                        if *menu_open.read() {
                            div { class: "pd-menu-backdrop", onclick: move |_| menu_open.set(false) }
                            div { class: "pd-menu",
                                Menu {
                                    aria_label: format!("Actions for {}", project.name),
                                    items: entries,
                                    on_select: move |entry: String| {
                                        menu_open.set(false);
                                        run_menu_entry(
                                            &mut state,
                                            menu_service.as_ref(),
                                            &menu_model,
                                            menu_setting.as_deref(),
                                            &entry,
                                        );
                                    },
                                }
                            }
                        }
                    }
                }
                div { class: "pd-meta-row",
                    span { class: "pd-path", title: "{folder_text}", "{folder_text}" }
                    if let Some(branch) = &model.branch {
                        span { class: "pd-branch", title: "Git branch",
                            crate::ui::Icon { name: "git-branch", size: crate::ui::IconSize::Md }
                            span { "{branch}" }
                        }
                    }
                    span { class: "pd-subline", "{model.subline}" }
                }
                if let Some(folder) = project.folder.clone() {
                    git_panel::GitPanel { folder }
                }
                if let Some(failure) = &row.failure {
                    div { class: "pd-failure",
                        InlineMessage { message: failure.message.clone() }
                        button {
                            class: "pd-failure-dismiss",
                            r#type: "button",
                            "aria-label": "Dismiss message",
                            onclick: move |_| {
                                if let Some(key) = &failure_key {
                                    crate::services::actions::dismiss(&mut state, key);
                                }
                            },
                            crate::ui::Icon { name: "x", size: crate::ui::IconSize::Sm }
                        }
                    }
                }
                if let Some(message) = banner {
                    div { class: "pd-banner",
                        Banner {
                            message,
                            tone: BannerTone::Danger,
                            action_label: (selected_tab != Tab::Files).then(|| "Show the file".to_string()),
                            onaction: move |()| tab.set(Tab::Files),
                        }
                    }
                }
                Tabs {
                    items: tabs,
                    selected: selected_tab.id().to_string(),
                    aria_label: "Project views".to_string(),
                    on_select: move |id: String| tab.set(Tab::from_id(&id)),
                }
            }
            div { class: "pd-body",
                match selected_tab {
                    Tab::Services => rsx! {
                        services_tab::ServicesTab {
                            id: id.clone(),
                            on_start: run_start,
                            start_reason,
                        }
                    },
                    Tab::Files => match project.folder.clone() {
                        Some(folder) => rsx! {
                            files::FilesTab {
                                folder,
                                folder_text: folder_text.clone(),
                                compose_files: project.compose_files.clone(),
                                problem,
                                editor: editor.clone(),
                                editor_setting,
                                terminal_setting,
                                on_refresh: move |()| check.call(()),
                            }
                        },
                        None => rsx! {
                            div { class: "pd-empty",
                                EmptyState {
                                    title: "No folder on this Mac".to_string(),
                                    body: "Berth found these containers on the engine, not a folder. Open the folder they came from to see its files here.".to_string(),
                                    icon: "folder",
                                }
                            }
                        },
                    },
                    Tab::Logs => rsx! {
                        div { class: "pd-logs",
                            if container_ids.is_empty() {
                                div { class: "pd-empty",
                                    EmptyState {
                                        title: "No logs yet".to_string(),
                                        body: "Logs appear once the project has containers.".to_string(),
                                        icon: "logs",
                                    }
                                }
                            } else {
                                LogPanel { containers: container_ids.clone() }
                            }
                        }
                    },
                    Tab::Activity => rsx! {
                        activity_tab::ActivityTab { id: id.clone(), seed: seed.activity.clone() }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "project_detail_tests.rs"]
mod tests;
