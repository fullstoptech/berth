//! One row of the Projects table (PLAN 7.3 `ProjectRow`, frame "Projects /
//! default / 1440"). `row_model` turns a project and the state around it
//! into plain data (pure, tested); `ProjectRow` draws that data and wires
//! its buttons to the `ActionsService` (D34). The row never calls an
//! engine itself.

use dioxus::prelude::*;

use super::button::{Button, ButtonVariant};
use super::icon::{Icon, IconSize};
use super::icon_button::IconButton;
use super::inline_message::InlineMessage;
use super::menu::{Menu, MenuEntry};
use super::sidebar_item::{Dot, DotShape, DotSpec};
use super::ControlSize;
use crate::services::actions::{dismiss, ActionsService, ProjectOp, StateView};
use crate::services::group_ops::{self, GroupIds};
use crate::state::{AppState, BottomTab, PendingKey, PendingState, Section, Selection};

/// Which labelled button the row shows besides Restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryOp {
    Start,
    Stop,
}

impl PrimaryOp {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start project",
            Self::Stop => "Stop project",
        }
    }
    pub fn op(self) -> ProjectOp {
        match self {
            Self::Start => ProjectOp::Start,
            Self::Stop => ProjectOp::Stop,
        }
    }
}

/// Colour of the note after the service count ("db failing").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteTone {
    Muted,
    Warning,
    Danger,
}

impl NoteTone {
    fn class(self) -> &'static str {
        match self {
            Self::Muted => "note-muted",
            Self::Warning => "note-warning",
            Self::Danger => "note-danger",
        }
    }
}

/// What the Ports, CPU and Memory cells say when there is no number.
#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    Normal,
    /// The project's engine cannot be reached: red dots, "Engine
    /// unreachable" where ports go, every button disabled.
    Unreachable,
    /// A known folder with no Compose file yet: "Set up project…".
    NotSetUp,
}

/// An action that is running on the row right now (D34).
#[derive(Debug, Clone, PartialEq)]
pub struct RowPending {
    pub action: &'static str,
    pub label: String,
}

/// The engine's message for the action that last failed on the row. The
/// pending key it came from is `subject` + `action`: the project for a
/// Compose project, one of its containers for the Unassigned row.
#[derive(Debug, Clone, PartialEq)]
pub struct RowFailure {
    pub subject: berth_core::Subject,
    pub action: &'static str,
    pub message: String,
}

/// What the Unassigned row's buttons do. Loose containers have no Compose
/// project, so Restart, Stop all and Start all run one container action per
/// qualifying member, through the same `group_ops` the Containers screen's
/// group header uses.
#[derive(Debug, Clone, PartialEq)]
pub struct LooseOps {
    pub ids: GroupIds,
    /// Why Start all cannot run (nothing is stopped, engine down).
    pub start_reason: Option<String>,
    /// Why Stop all and Restart cannot run (nothing is running, engine down).
    pub stop_reason: Option<String>,
}

impl LooseOps {
    /// The members and the container action a row button runs; `None` for a
    /// Compose-only operation.
    pub fn plan(
        &self,
        op: ProjectOp,
    ) -> Option<(&[berth_core::ContainerId], berth_core::ContainerAction)> {
        use berth_core::ContainerAction as A;
        match op {
            ProjectOp::Start => Some((&self.ids.start_ids, A::Start)),
            ProjectOp::Stop => Some((&self.ids.stop_ids, A::Stop { timeout_s: None })),
            ProjectOp::Restart => Some((&self.ids.stop_ids, A::Restart { timeout_s: None })),
            _ => None,
        }
    }
}

/// The reason a group action cannot run on `ids`: none to run on, or every
/// one refused (engine down, still busy). `None` when at least one qualifies.
fn group_reason(
    ids: &[berth_core::ContainerId],
    action: &str,
    none: &str,
    view: &StateView<'_>,
) -> Option<String> {
    if ids.is_empty() {
        return Some(none.to_string());
    }
    // `None` as soon as one member can run it; else the first reason.
    let reasons: Option<Vec<String>> = ids
        .iter()
        .map(|id| {
            let subject = berth_core::Subject::Container(id.clone());
            crate::services::actions::disabled_reason(&subject, action, view)
        })
        .collect();
    reasons.and_then(|reasons| reasons.into_iter().next())
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectRowModel {
    pub id: berth_core::ProjectId,
    pub name: String,
    pub kind: RowKind,
    pub path_short: String,
    /// The whole `~/…` path, or "—" when the project has no folder.
    pub path_full: String,
    pub engine_name: String,
    pub branch: Option<String>,
    pub dots: Vec<DotSpec>,
    pub services_label: String,
    pub note: Option<(String, NoteTone)>,
    pub ports: Vec<u16>,
    pub cpu: Option<String>,
    pub mem: Option<String>,
    /// Error-severity problems on this project's containers.
    pub problem_count: usize,
    pub problem_summary: Option<String>,
    pub primary: PrimaryOp,
    /// "Start project", "Stop project", or "Start all" / "Stop all" on the
    /// Unassigned row.
    pub primary_label: String,
    /// `Some` on the Unassigned row (a `ProjectSource::Loose` project).
    pub loose: Option<LooseOps>,
    pub primary_reason: Option<String>,
    pub restart_reason: Option<String>,
    pub rebuild_reason: Option<String>,
    pub pull_reason: Option<String>,
    /// Why "Open terminals for all services" is unavailable (P3-T6).
    pub terminals_reason: Option<String>,
    pub pending: Option<RowPending>,
    pub failure: Option<RowFailure>,
}

/// `~/Dev/clients/acme/shop-api` becomes `~/Dev/…/shop-api`: the first two
/// parts and the last one stay, the middle collapses. CSS ellipsis then
/// cuts what still does not fit; the full path is the hover title.
pub fn collapse_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 3 {
        return path.to_string();
    }
    let last = parts.last().copied().unwrap_or_default();
    format!("{}/{}/…/{last}", parts[0], parts[1])
}

/// Running services' host ports, ascending, each once.
pub fn running_ports(project: &berth_core::Project) -> Vec<u16> {
    let mut ports: Vec<u16> = project
        .services
        .iter()
        .filter(|s| s.state == berth_core::ContainerState::Running)
        .flat_map(|s| s.ports.iter().filter_map(|p| p.host_port))
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// The dot for one service's status (ring while changing, hollow when
/// paused, never colour alone).
pub fn dot_for_status(status: berth_core::StatusKind) -> DotSpec {
    use berth_core::StatusKind as S;
    let shape = match status {
        S::Starting | S::Restarting => DotShape::Ring,
        S::Paused => DotShape::Hollow,
        _ => DotShape::Solid,
    };
    let label = match status {
        S::Running | S::Healthy => "Running",
        S::Starting => "Starting",
        S::Restarting => "Restarting",
        S::Unhealthy => "Unhealthy",
        S::Paused => "Paused",
        S::Exited(0) => "Exited",
        S::Exited(_) => "Failed",
        S::Created => "Created",
        S::Removing => "Removing",
        S::Dead => "Dead",
    };
    DotSpec {
        tone: status.tone(),
        shape,
        label,
    }
}

/// The word after the service count. All running reads "all running"
/// (or "running" for one service), all stopped "stopped"; otherwise the
/// worst service is named ("db failing", "grafana exited").
pub fn service_note(services: &[(String, berth_core::StatusKind)]) -> Option<(String, NoteTone)> {
    use berth_core::StatusKind as S;
    if services.is_empty() {
        return None;
    }
    let all = |f: fn(&S) -> bool| services.iter().all(|(_, s)| f(s));
    if all(|s| matches!(s, S::Running | S::Healthy)) {
        let text = if services.len() == 1 {
            "running"
        } else {
            "all running"
        };
        return Some((text.to_string(), NoteTone::Muted));
    }
    if all(|s| matches!(s, S::Exited(0) | S::Created)) {
        return Some(("stopped".to_string(), NoteTone::Muted));
    }
    // A service that is down explains a project better than one that is
    // merely unwell: failed first, then exited, then restarting or
    // unhealthy.
    let rank = |s: &S| match s {
        S::Exited(code) if *code != 0 => 0,
        S::Dead => 0,
        S::Exited(_) | S::Created | S::Removing => 1,
        S::Restarting | S::Unhealthy | S::Paused | S::Starting => 2,
        S::Running | S::Healthy => 3,
    };
    let (name, status) = services.iter().min_by_key(|(_, s)| rank(s))?;
    let (word, tone) = match status {
        S::Exited(0) | S::Created | S::Removing => ("exited", NoteTone::Warning),
        S::Exited(_) | S::Dead => ("failing", NoteTone::Danger),
        S::Restarting => ("restarting", NoteTone::Warning),
        S::Unhealthy => ("unhealthy", NoteTone::Warning),
        S::Paused => ("paused", NoteTone::Warning),
        S::Starting => ("starting", NoteTone::Warning),
        S::Running | S::Healthy => return None,
    };
    Some((format!("{name} {word}"), tone))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Build one row from the state around it. `problems` are the problems
/// worth showing, newest first; only error severity counts on the row.
pub fn row_model(
    project: &berth_core::Project,
    view: &StateView<'_>,
    problems: &[berth_core::Problem],
    home: &std::path::Path,
) -> ProjectRowModel {
    use berth_core::{ProjectStatus as P, Subject};
    let engine = view.engines.iter().find(|e| e.id == project.engine);
    let reachable = engine.is_none_or(|e| e.state == berth_core::EngineState::Running);
    let unreachable = engine.is_some_and(|e| {
        matches!(
            e.state,
            berth_core::EngineState::Unreachable { .. }
                | berth_core::EngineState::Unsupported { .. }
        )
    });
    let container_ids: Vec<&berth_core::ContainerId> = project
        .services
        .iter()
        .flat_map(|s| s.containers.iter())
        .collect();
    let owned: Vec<&berth_core::Container> = view
        .containers
        .iter()
        .filter(|c| container_ids.contains(&&c.id))
        .collect();

    let service_status: Vec<(String, berth_core::StatusKind)> = project
        .services
        .iter()
        .map(|service| {
            let container = service
                .containers
                .first()
                .and_then(|id| owned.iter().find(|c| c.id == *id));
            let status = container.map_or_else(
                || berth_core::status_of(service.state, service.health, None),
                |c| berth_core::status_of(c.state, c.health, c.exit_code),
            );
            (service.name.clone(), status)
        })
        .collect();

    let not_set_up = project.compose_files.is_empty() && project.services.is_empty();
    let kind = if unreachable {
        RowKind::Unreachable
    } else if not_set_up {
        RowKind::NotSetUp
    } else {
        RowKind::Normal
    };

    let dots = if not_set_up {
        vec![DotSpec {
            tone: berth_core::Tone::Stopped,
            shape: DotShape::Hollow,
            label: "Not created",
        }]
    } else {
        service_status
            .iter()
            .map(|(_, status)| {
                if kind == RowKind::Unreachable {
                    DotSpec {
                        tone: berth_core::Tone::Danger,
                        shape: DotShape::Solid,
                        label: "Unreachable",
                    }
                } else {
                    dot_for_status(*status)
                }
            })
            .collect()
    };
    let services_label = if not_set_up {
        "No compose file yet".to_string()
    } else {
        plural(project.services.len(), "service", "services")
    };
    let note = if kind == RowKind::Normal {
        service_note(&service_status)
    } else {
        None
    };

    let stats: Vec<&berth_core::LiveStats> = owned
        .iter()
        .filter(|c| c.state == berth_core::ContainerState::Running)
        .filter_map(|c| c.stats.as_ref())
        .collect();
    let (cpu, mem) = if stats.is_empty() || kind != RowKind::Normal {
        (None, None)
    } else {
        let cpu: f32 = stats.iter().map(|s| s.cpu_pct).sum();
        let mut used = stats.iter().map(|s| s.mem_used);
        let mem = used
            .next()
            .map(|first| berth_core::fmt::bytes(used.fold(first, |total, next| total + next)));
        (Some(format!("{cpu:.1}%")), mem)
    };

    let mine: Vec<&berth_core::Problem> = problems
        .iter()
        .filter(|p| p.severity == berth_core::Severity::Error)
        .filter(|p| match &p.subject {
            Subject::Container(id) => container_ids.contains(&id),
            Subject::Project(id) => *id == project.id,
            _ => false,
        })
        .collect();
    let problem_summary = mine.first().map(|first| {
        format!(
            "{} · {}",
            plural(mine.len(), "problem", "problems"),
            first.title
        )
    });

    let is_loose = matches!(project.source, berth_core::ProjectSource::Loose);
    let loose = is_loose.then(|| {
        let ids = GroupIds::of(owned.iter().copied());
        let start_reason = group_reason(
            &ids.start_ids,
            "container.start",
            "Nothing is stopped",
            view,
        );
        let stop_reason = group_reason(&ids.stop_ids, "container.stop", "Nothing is running", view);
        LooseOps {
            ids,
            start_reason,
            stop_reason,
        }
    });

    let subject = Subject::Project(project.id.clone());
    let owns = |key: &PendingKey| match &key.subject {
        Subject::Container(id) => is_loose && container_ids.contains(&id),
        other => *other == subject,
    };
    // A member's start, stop or restart reads as the row's own button
    // running; other member actions leave the row alone.
    let row_action = |action: &str| match action {
        "container.start" => Some("project.start"),
        "container.stop" => Some("project.stop"),
        "container.restart" => Some("project.restart"),
        _ => None,
    };
    let pending = view.pending.iter().find_map(|(key, value)| match value {
        PendingState::Running { label } if owns(key) => {
            let action = if is_loose {
                row_action(key.action)?
            } else {
                key.action
            };
            Some(RowPending {
                action,
                label: label.clone(),
            })
        }
        _ => None,
    });
    let failure = view.pending.iter().find_map(|(key, value)| match value {
        PendingState::Failed { message } if owns(key) => Some(RowFailure {
            subject: key.subject.clone(),
            action: key.action,
            message: message.clone(),
        }),
        _ => None,
    });

    let primary = match &loose {
        Some(ops) if !ops.ids.stop_ids.is_empty() => PrimaryOp::Stop,
        Some(_) => PrimaryOp::Start,
        None if !reachable || matches!(project.status, P::Stopped | P::NotCreated) => {
            PrimaryOp::Start
        }
        None => PrimaryOp::Stop,
    };
    let reason = |action: &str| crate::services::actions::disabled_reason(&subject, action, view);
    let not_compose = || Some("Not a Compose project".to_string());
    let (primary_label, primary_reason, restart_reason, rebuild_reason, pull_reason) = match &loose
    {
        Some(ops) => (
            match primary {
                PrimaryOp::Start => "Start all",
                PrimaryOp::Stop => "Stop all",
            }
            .to_string(),
            match primary {
                PrimaryOp::Start => ops.start_reason.clone(),
                PrimaryOp::Stop => ops.stop_reason.clone(),
            },
            ops.stop_reason.clone(),
            not_compose(),
            not_compose(),
        ),
        None => (
            primary.label().to_string(),
            reason(primary.op().action_id()),
            reason("project.restart"),
            reason("project.rebuild"),
            reason("project.pull_images"),
        ),
    };

    ProjectRowModel {
        id: project.id.clone(),
        name: project.name.clone(),
        kind,
        path_short: project.folder.as_deref().map_or_else(
            || "—".to_string(),
            |f| collapse_path(&berth_core::fmt::tilde(f, home)),
        ),
        path_full: project
            .folder
            .as_deref()
            .map_or_else(|| "—".to_string(), |f| berth_core::fmt::tilde(f, home)),
        engine_name: engine.map_or_else(|| project.engine.0.clone(), |e| e.name.clone()),
        branch: project.git.as_ref().and_then(|g| g.branch.clone()),
        dots,
        services_label,
        note,
        ports: running_ports(project),
        cpu,
        mem,
        problem_count: mine.len(),
        problem_summary,
        primary,
        primary_label,
        loose,
        primary_reason,
        restart_reason,
        rebuild_reason,
        pull_reason,
        terminals_reason: crate::services::exec::project_terminals_reason(view.containers, project),
        pending,
        failure,
    }
}

/// Run a project action through the service: what a row button's click
/// calls. A refusal (disabled reason, engine not connected) lands in the
/// row as a failure message, never silently.
pub fn run_project_op(
    state: &mut AppState,
    service: Option<&ActionsService>,
    id: &berth_core::ProjectId,
    op: ProjectOp,
) {
    let project = state.projects.peek().iter().find(|p| p.id == *id).cloned();
    let (Some(project), Some(service)) = (project, service) else {
        return;
    };
    if let Err(message) = service.project(state, &project, op) {
        state.pending.write().insert(
            PendingKey {
                subject: berth_core::Subject::Project(id.clone()),
                action: op.action_id(),
            },
            PendingState::Failed { message },
        );
    }
}

/// What a row button's click calls: a Compose project action, or on the
/// Unassigned row one container action per qualifying member.
pub fn run_row_op(
    state: &mut AppState,
    service: Option<&ActionsService>,
    id: &berth_core::ProjectId,
    loose: Option<&LooseOps>,
    op: ProjectOp,
) {
    match loose {
        None => run_project_op(state, service, id, op),
        Some(ops) => {
            if let Some((ids, action)) = ops.plan(op) {
                group_ops::run(state, service, ids, &action);
            }
        }
    }
}

/// Open `http://localhost:<port>` with the system `open` (rule: binaries
/// come from `SearchPath`). Fire and forget; a failure is only logged.
fn open_localhost(port: u16) {
    let Some(open) = berth_sys::SearchPath::from_process().find("open") else {
        tracing::warn!(port, "no open binary on the search path");
        return;
    };
    let url = format!("http://localhost:{port}");
    let work = async move {
        let result = berth_sys::run_capture(
            &open,
            &[std::ffi::OsString::from(url)],
            berth_sys::SpawnOpts {
                timeout: Some(std::time::Duration::from_secs(10)),
                ..Default::default()
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
        if let Err(err) = result {
            tracing::warn!(%err, port, "open failed");
        }
    };
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(work);
    } else {
        spawn(work);
    }
}

/// `:3000 :6379 +1`: the first two ports as links, the rest as a count.
#[component]
fn PortLinks(ports: Vec<u16>) -> Element {
    let rest = ports.len().saturating_sub(2);
    rsx! {
        for port in ports.iter().copied().take(2) {
            a {
                key: "{port}",
                class: "port-link",
                href: "http://localhost:{port}",
                title: "Open http://localhost:{port}",
                onclick: move |e| {
                    e.prevent_default();
                    open_localhost(port);
                },
                ":{port}"
            }
        }
        if rest > 0 {
            span { class: "ports-more", "+{rest}" }
        }
    }
}

/// The ⋯ menu. A Compose project offers Rebuild, Pull and Delete; the
/// Unassigned row offers the same three group actions as its buttons and
/// leaves the Compose-only ones out.
pub fn menu_entries(model: &ProjectRowModel) -> Vec<MenuEntry> {
    let entry = |id: &str, label: &str, icon: &'static str, reason: &Option<String>| {
        let entry = MenuEntry::new(id, label).icon(icon);
        match reason {
            Some(reason) => entry.disabled(reason.clone()),
            None => entry,
        }
    };
    let open = MenuEntry::new("open", "Open project").icon("folder");
    if let Some(ops) = &model.loose {
        return vec![
            open,
            entry("start-all", "Start all", "play", &ops.start_reason),
            entry("stop-all", "Stop all", "square", &ops.stop_reason),
            entry("restart", "Restart", "rotate-cw", &ops.stop_reason),
        ];
    }
    vec![
        open,
        entry("rebuild", "Rebuild", "rotate-cw", &model.rebuild_reason),
        entry(
            "pull",
            "Pull images",
            "arrow-down-to-line",
            &model.pull_reason,
        ),
        entry(
            "terminals",
            "Open terminals for all services",
            "terminal",
            &model.terminals_reason,
        ),
        MenuEntry::new("delete", "Delete project\u{2026}")
            .icon("trash-2")
            .danger()
            .separator_before()
            .disabled("Delete confirmation is not built yet"),
    ]
}

#[component]
pub fn ProjectRow(model: ProjectRowModel) -> Element {
    let mut state = use_context::<AppState>();
    let service = try_use_context::<ActionsService>();
    let mut menu_open = use_signal(|| false);
    let name = model.name.clone();
    let id = model.id.clone();
    let busy = model.pending.is_some();
    let loading = |action: &str| model.pending.as_ref().filter(|p| p.action == action);

    let restart_pending = loading("project.restart");
    let primary_action = model.primary.op().action_id();
    let primary_pending = loading(primary_action);

    let mut row_class = String::from("project-row");
    match model.kind {
        RowKind::Unreachable => row_class.push_str(" project-row-unreachable"),
        RowKind::NotSetUp => row_class.push_str(" project-row-notsetup"),
        RowKind::Normal => {}
    }

    let failure_key = model.failure.as_ref().map(|f| PendingKey {
        subject: f.subject.clone(),
        action: f.action,
    });

    let menu_entries = menu_entries(&model);

    let open_project = {
        let id = id.clone();
        move || {
            state.selection.set(Selection::Project(id.clone()));
            state.section.set(Section::Project);
        }
    };

    rsx! {
        div { class: "project-row-wrap",
            div { class: "{row_class}", role: "row",
                // The full path is the name cell's tooltip at any size, so it
                // stays reachable when the Path column is hidden (D3). The
                // button inside carries it too: a child's title wins on hover.
                div {
                    class: "project-cell project-cell-name",
                    role: "cell",
                    title: "{model.path_full}",
                    button {
                        class: "project-name",
                        r#type: "button",
                        title: "Open {model.name}\n{model.path_full}",
                        onclick: {
                            let mut open_project = open_project.clone();
                            move |_| open_project()
                        },
                        Icon { name: "folder", size: IconSize::Md }
                        span { class: "project-name-text", "{model.name}" }
                    }
                    if model.problem_count > 0 {
                        span { class: "project-problem-tag",
                            {plural(model.problem_count, "problem", "problems")}
                        }
                    }
                }
                div {
                    class: "project-cell project-cell-path",
                    role: "cell",
                    title: "{model.path_full}",
                    "{model.path_short}"
                }
                div { class: "project-cell project-cell-engine", role: "cell", "{model.engine_name}" }
                div { class: "project-cell project-cell-branch", role: "cell",
                    if let Some(branch) = &model.branch {
                        Icon { name: "git-branch", size: IconSize::Md }
                        span { class: "project-branch-text", title: "{branch}", "{branch}" }
                    } else {
                        span { class: "cell-dash", "\u{2014}" }
                    }
                }
                div { class: "project-cell project-cell-services", role: "cell",
                    span { class: "project-dots",
                        for (index, spec) in model.dots.iter().enumerate() {
                            Dot { key: "{index}", spec: *spec }
                        }
                    }
                    span { class: "project-services-text",
                        "{model.services_label}"
                        if let Some((text, tone)) = &model.note {
                            span { class: "project-note {tone.class()}", " \u{00b7} {text}" }
                        }
                    }
                }
                div {
                    class: if model.kind == RowKind::NotSetUp {
                        "project-cell project-cell-ports project-cell-span"
                    } else {
                        "project-cell project-cell-ports"
                    },
                    role: "cell",
                    match model.kind {
                        RowKind::Unreachable => rsx! {
                            span { class: "ports-unreachable", "Engine unreachable" }
                        },
                        RowKind::NotSetUp => rsx! {
                            span { class: "ports-hint", "Shows a diff before writing" }
                        },
                        RowKind::Normal if model.ports.is_empty() => rsx! {
                            span { class: "cell-dash", "\u{2014}" }
                        },
                        RowKind::Normal => rsx! { PortLinks { ports: model.ports.clone() } },
                    }
                }
                if model.kind != RowKind::NotSetUp {
                    div { class: "project-cell project-cell-num", role: "cell",
                        match &model.cpu {
                            Some(cpu) => rsx! { "{cpu}" },
                            None => rsx! { span { class: "cell-dash", "\u{2014}" } },
                        }
                    }
                    div { class: "project-cell project-cell-num", role: "cell",
                        match &model.mem {
                            Some(mem) => rsx! { "{mem}" },
                            None => rsx! { span { class: "cell-dash", "\u{2014}" } },
                        }
                    }
                }
                div { class: "project-cell project-cell-actions", role: "cell",
                    if model.kind == RowKind::NotSetUp {
                        Button {
                            label: "Set up project\u{2026}",
                            size: ControlSize::Sm,
                            disabled: true,
                            disabled_reason: "Project setup is not built yet".to_string(),
                        }
                    } else {
                        Button {
                            label: restart_pending
                                .map_or_else(|| "Restart".to_string(), |p| p.label.clone()),
                            size: ControlSize::Sm,
                            loading: restart_pending.is_some(),
                            disabled: busy || model.restart_reason.is_some(),
                            disabled_reason: model.restart_reason.clone(),
                            onclick: {
                                let id = id.clone();
                                let service = service.clone();
                                let loose = model.loose.clone();
                                move |_| run_row_op(
                                    &mut state,
                                    service.as_ref(),
                                    &id,
                                    loose.as_ref(),
                                    ProjectOp::Restart,
                                )
                            },
                        }
                        Button {
                            label: primary_pending
                                .map_or_else(|| model.primary_label.clone(), |p| p.label.clone()),
                            size: ControlSize::Sm,
                            variant: ButtonVariant::Secondary,
                            loading: primary_pending.is_some(),
                            disabled: busy || model.primary_reason.is_some(),
                            disabled_reason: model.primary_reason.clone(),
                            onclick: {
                                let id = id.clone();
                                let service = service.clone();
                                let op = model.primary.op();
                                let loose = model.loose.clone();
                                move |_| run_row_op(&mut state, service.as_ref(), &id, loose.as_ref(), op)
                            },
                        }
                    }
                    IconButton {
                        icon: "logs",
                        label: "Logs for {name}",
                        size: ControlSize::Sm,
                        tooltip: "Logs".to_string(),
                        onclick: {
                            let id = id.clone();
                            move |_| {
                                state.selection.set(Selection::Project(id.clone()));
                                let mut bottom = state.bottom.write();
                                bottom.open = true;
                                bottom.tab = BottomTab::Logs;
                            }
                        },
                    }
                    span { class: "project-more",
                        IconButton {
                            icon: "more-horizontal",
                            label: "More actions for {name}",
                            size: ControlSize::Sm,
                            pressed: Some(*menu_open.read()),
                            onclick: move |_| {
                                let open = *menu_open.read();
                                menu_open.set(!open);
                            },
                        }
                        if *menu_open.read() {
                            div {
                                class: "project-menu-backdrop",
                                onclick: move |_| menu_open.set(false),
                            }
                            div { class: "project-menu",
                                Menu {
                                    aria_label: "Actions for {name}",
                                    items: menu_entries,
                                    on_select: {
                                        let id = id.clone();
                                        let service = service.clone();
                                        let mut open_project = open_project.clone();
                                        let loose = model.loose.clone();
                                        move |entry: String| {
                                            menu_open.set(false);
                                            let group_op = match entry.as_str() {
                                                "start-all" => Some(ProjectOp::Start),
                                                "stop-all" => Some(ProjectOp::Stop),
                                                "restart" => Some(ProjectOp::Restart),
                                                _ => None,
                                            };
                                            if let Some(op) = group_op {
                                                run_row_op(
                                                    &mut state,
                                                    service.as_ref(),
                                                    &id,
                                                    loose.as_ref(),
                                                    op,
                                                );
                                                return;
                                            }
                                            match entry.as_str() {
                                                "open" => open_project(),
                                                "rebuild" => run_project_op(
                                                    &mut state,
                                                    service.as_ref(),
                                                    &id,
                                                    ProjectOp::Rebuild,
                                                ),
                                                "pull" => run_project_op(
                                                    &mut state,
                                                    service.as_ref(),
                                                    &id,
                                                    ProjectOp::PullImages,
                                                ),
                                                "terminals" => {
                                                    if let (Some(core), Some(mut view)) = (
                                                        try_consume_context::<crate::services::exec::ExecCore>(),
                                                        try_consume_context::<crate::services::exec::TerminalView>(),
                                                    ) {
                                                        let _ = crate::services::exec::open_project_terminals(
                                                            &mut state, &mut view, &core, &id,
                                                        );
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
            }
            if let Some(summary) = &model.problem_summary {
                div { class: "project-problem-line", role: "alert",
                    Icon { name: "alert-circle", size: IconSize::Md }
                    span { "{summary}" }
                }
            }
            if let Some(failure) = &model.failure {
                div { class: "project-failure",
                    InlineMessage { message: failure.message.clone() }
                    button {
                        class: "project-failure-dismiss",
                        r#type: "button",
                        "aria-label": "Dismiss message for {name}",
                        onclick: move |_| {
                            if let Some(key) = &failure_key {
                                dismiss(&mut state, key);
                            }
                        },
                        Icon { name: "x", size: IconSize::Sm }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "project_row_tests.rs"]
mod tests;
