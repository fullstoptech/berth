//! The view model of a problem and what a click on its fix does (P5-T8). The Problems tab, the
//! inspector's card and the log Explain list all build their items here, so every kind of problem
//! reads and behaves the same wherever it shows.
//!
//! Fix buttons carry ids of the form `<problem id>#<index>` (the index is the fix's place in
//! `Problem::fixes`) or `goto:<container id>`; [`click`] finds the problem and does what the fix
//! says:
//!
//! - `EditFiles(plan)`: the plan is parked in the `PlanStore` and Review changes opens. Nothing is
//!   written until its Write button is clicked (ADR 0012).
//! - `RunAction`: a confirm dialog opens, drawn from the fix's own `EffectSummary` (plus a time
//!   line for what takes a while), and its button runs the action through `confirm::run`. Prune
//!   opens Clean up instead, which lets the person choose what goes.
//! - `Explain` is advice and shows as a note, not a button.
//! - A harness fix is disabled until a harness exists (P7-T6).

use berth_core::{
    ColimaOp, Container, ContainerId, EffectKind, EffectLine, EffectSummary, EngineAction,
    FixEffect, FixSource, GenerationPlan, Problem, ProblemKind, Project, Subject,
};
use dioxus::prelude::WritableExt;
use uuid::Uuid;

use crate::screens::containers::select_only;
use crate::services::actions::{container_action_id, disabled_reason, with_view, StateView};
use crate::services::backing::Capability;
use crate::services::confirm;
use crate::services::plans::{PendingPlan, PlanRun, PlanStore};
use crate::state::{AppState, ConfirmModel, Overlay, PendingKey, PendingState};
use crate::ui::{FixButton, ProblemView};

/// The pending key of "Recreate" (`EditOp::Recreate`).
const RECREATE: &str = "container.recreate";
/// How long a Colima start or restart takes, as the engines screen says it too.
const COLIMA_TIME: &str = "This takes about 20 s.";

/// The id of fix `index` of `problem` on its button.
#[must_use]
pub fn fix_id(problem: &Problem, index: usize) -> String {
    format!("{}#{index}", problem.id.0)
}

/// What a person calls the thing a problem is about.
#[must_use]
pub fn subject_name(problem: &Problem, view: &StateView<'_>) -> String {
    match &problem.subject {
        Subject::Container(id) => view
            .containers
            .iter()
            .find(|c| c.id == *id)
            .map_or_else(|| id.short().to_string(), |c| c.name.clone()),
        Subject::Project(id) => view
            .projects
            .iter()
            .find(|p| p.id == *id)
            .map_or_else(|| "project".to_string(), |p| p.name.clone()),
        Subject::Engine(id) => view
            .engines
            .iter()
            .find(|e| e.id == *id)
            .map_or_else(|| id.0.clone(), |e| e.name.clone()),
        Subject::Image(name)
        | Subject::Volume(name)
        | Subject::Network(name)
        | Subject::Plugin(name) => name.clone(),
    }
}

/// The project a problem belongs to: its subject, or the one its container is part of (the
/// container's own link, else the project whose services list it, else the Compose project its
/// labels name).
fn project_of<'a>(problem: &Problem, view: &'a StateView<'_>) -> Option<&'a Project> {
    match &problem.subject {
        Subject::Project(id) => view.projects.iter().find(|p| p.id == *id),
        Subject::Container(id) => {
            let container = view.containers.iter().find(|c| c.id == *id)?;
            container
                .project
                .as_ref()
                .and_then(|id| view.projects.iter().find(|p| p.id == *id))
                .or_else(|| {
                    view.projects.iter().find(|p| {
                        p.engine == container.engine
                            && p.services.iter().any(|s| s.containers.contains(id))
                    })
                })
                .or_else(|| {
                    let name = container.compose_project.as_ref()?;
                    view.projects
                        .iter()
                        .find(|p| p.engine == container.engine && p.name == *name)
                })
        }
        _ => None,
    }
}

/// The group a problem is listed under in the Problems tab: its project, else its engine.
#[must_use]
pub fn group_label(problem: &Problem, view: &StateView<'_>) -> String {
    if let Some(project) = project_of(problem, view) {
        return project.name.clone();
    }
    match &problem.subject {
        Subject::Container(_) => "Unassigned".to_string(),
        Subject::Engine(_) => subject_name(problem, view),
        _ => "Other".to_string(),
    }
}

fn engine_of_profile<'a>(view: &'a StateView<'_>, profile: &str) -> Option<&'a berth_core::Engine> {
    view.engines
        .iter()
        .find(|e| crate::services::colima::profile_of(e) == Some(profile))
}

/// The pending key an action runs under, which says whether the fix is running or has failed.
/// `None` for an action a fix cannot run from here (Prune opens Clean up, the rest are not
/// offered by any rule).
#[must_use]
pub fn pending_key(action: &EngineAction, view: &StateView<'_>) -> Option<PendingKey> {
    match action {
        EngineAction::Container { id, action, .. } => Some(PendingKey {
            subject: Subject::Container(id.clone()),
            action: container_action_id(action),
        }),
        EngineAction::Recreate { id, .. } => Some(PendingKey {
            subject: Subject::Container(id.clone()),
            action: RECREATE,
        }),
        EngineAction::Compose { target, command } => {
            let project = view
                .projects
                .iter()
                .find(|p| p.engine == target.engine && p.name == target.project_name)?;
            Some(PendingKey {
                subject: Subject::Project(project.id.clone()),
                action: crate::services::compose::command_action_id(command),
            })
        }
        EngineAction::Colima { profile, op } => {
            let engine = engine_of_profile(view, profile)?;
            Some(PendingKey {
                subject: Subject::Engine(engine.id.clone()),
                action: crate::services::colima::action_id(op),
            })
        }
        EngineAction::PullImage { .. }
        | EngineAction::Prune { .. }
        | EngineAction::Remove { .. } => None,
    }
}

/// Why a rule fix cannot be clicked now, or `None`.
fn disabled_for(effect: &FixEffect, view: &StateView<'_>) -> Option<String> {
    match effect {
        FixEffect::EditFiles(_) => view.backing.unsupported(Capability::WriteFiles),
        FixEffect::RunAction { action, .. } => match action {
            EngineAction::Prune { .. } => None,
            EngineAction::Recreate { .. } => view
                .backing
                .unsupported(Capability::CreateContainers)
                .or_else(|| {
                    let key = pending_key(action, view)?;
                    disabled_reason(&key.subject, key.action, view)
                }),
            other => match pending_key(other, view) {
                Some(key) => disabled_reason(&key.subject, key.action, view),
                None => Some("This fix cannot run from here yet".to_string()),
            },
        },
        FixEffect::Explain => None,
    }
}

/// Is a fix's action running now, and did it fail?
fn run_state(effect: &FixEffect, view: &StateView<'_>) -> (bool, Option<String>) {
    let FixEffect::RunAction { action, .. } = effect else {
        return (false, None);
    };
    let Some(key) = pending_key(action, view) else {
        return (false, None);
    };
    match view.pending.get(&key) {
        Some(PendingState::Running { .. }) => (true, None),
        Some(PendingState::Failed { message }) => (false, Some(message.clone())),
        None => (false, None),
    }
}

/// The container a `DependencyFailed` problem is waiting for, in the same project.
fn dependency_of<'a>(
    problem: &Problem,
    service: &str,
    containers: &'a [Container],
) -> Option<&'a Container> {
    let Subject::Container(id) = &problem.subject else {
        return None;
    };
    let project = containers
        .iter()
        .find(|c| c.id == *id)?
        .compose_project
        .as_ref()?;
    containers.iter().find(|c| {
        c.compose_project.as_ref() == Some(project) && c.service.as_deref() == Some(service)
    })
}

/// One problem as an item. `with_subject` adds what it is about (the Problems tab), which the
/// inspector's card leaves out because the inspector is already about one container.
#[must_use]
pub fn problem_view(
    problem: &Problem,
    view: &StateView<'_>,
    now: chrono::DateTime<chrono::Utc>,
    with_subject: bool,
) -> ProblemView {
    let mut buttons: Vec<FixButton> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (index, fix) in problem.fixes.iter().enumerate() {
        let id = fix_id(problem, index);
        match (&fix.source, &fix.effect) {
            (FixSource::Harness(_), _) => buttons.push(FixButton {
                id,
                label: fix.label.clone(),
                primary: false,
                icon: Some("sparkles"),
                split: true,
                disabled_reason: Some("Needs a harness (P7-T6)".to_string()),
                loading: false,
                failed: None,
            }),
            (FixSource::Rule, FixEffect::Explain) => notes.push(fix.label.clone()),
            (FixSource::Rule, effect) => {
                let (loading, failed) = run_state(effect, view);
                let first = !buttons.iter().any(|b| b.primary || b.split);
                buttons.push(FixButton {
                    id,
                    label: fix.label.clone(),
                    primary: first,
                    icon: None,
                    split: false,
                    disabled_reason: disabled_for(effect, view),
                    loading,
                    failed,
                });
            }
        }
    }
    if let ProblemKind::DependencyFailed { service } = &problem.kind {
        if let Some(dependency) = dependency_of(problem, service, view.containers) {
            let go = FixButton {
                id: format!("goto:{}", dependency.id.0),
                label: format!("Go to {}", dependency.name),
                primary: false,
                icon: None,
                split: false,
                disabled_reason: None,
                loading: false,
                failed: None,
            };
            let at = buttons
                .iter()
                .position(|b| b.split)
                .unwrap_or(buttons.len());
            buttons.insert(at, go);
        }
    }
    ProblemView {
        id: problem.id.0.clone(),
        severity: problem.severity,
        title: problem.title.clone(),
        subject: with_subject.then(|| subject_name(problem, view)),
        age: berth_core::fmt::uptime(problem.first_seen, now),
        explanation: problem.explanation.clone(),
        evidence: problem.evidence.clone(),
        notes,
        fixes: buttons,
    }
}

/// The problems of one container, as the inspector's cards.
#[must_use]
pub fn card_views(
    container: &Container,
    problems: &[Problem],
    view: &StateView<'_>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<ProblemView> {
    problems
        .iter()
        .filter(|p| p.subject == Subject::Container(container.id.clone()))
        .map(|p| problem_view(p, view, now, false))
        .collect()
}

/// Problems of one project, one engine, or neither, listed together in the Problems tab.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub label: String,
    pub items: Vec<ProblemView>,
}

/// The Problems tab's list, grouped by project (or engine). `problems` is in the service's
/// stable order: a group sits where its first problem sits and its items keep their order, so
/// nothing moves under the pointer when another problem appears (AD31).
#[must_use]
pub fn groups(
    problems: &[Problem],
    view: &StateView<'_>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for problem in problems {
        let label = group_label(problem, view);
        let item = problem_view(problem, view, now, true);
        match out.iter_mut().find(|g| g.label == label) {
            Some(group) => group.items.push(item),
            None => out.push(Group {
                label,
                items: vec![item],
            }),
        }
    }
    out
}

// ---------------------------------------------------------------------
// Clicks
// ---------------------------------------------------------------------

/// What a click on a fix did, for the caller and the tests.
#[derive(Debug, Clone, PartialEq)]
pub enum Clicked {
    /// Review changes opened for this plan.
    Review(Uuid),
    /// A confirm dialog opened.
    Confirm,
    /// Clean up opened.
    CleanUp,
    /// The container the problem points at is selected.
    Selected,
    /// AD37: a harness fix opened this assistant session (P7-T6 builds the
    /// button that returns it).
    Assistant(berth_core::SessionId),
    /// Nothing happened, and why.
    Nothing(String),
}

/// The effect the dialog shows: the fix's own lines plus how long it takes, where that is known.
#[must_use]
pub fn effect_with_time(effect: &EffectSummary, action: &EngineAction) -> EffectSummary {
    let mut shown = effect.clone();
    if let EngineAction::Colima {
        op: ColimaOp::Start | ColimaOp::Restart | ColimaOp::Apply(_),
        ..
    } = action
    {
        shown.lines.push(EffectLine {
            kind: EffectKind::Changes,
            text: COLIMA_TIME.to_string(),
        });
    }
    shown
}

/// The dialog of a fix that runs `action`.
fn confirm_model(
    label: &str,
    action: &EngineAction,
    effect: &EffectSummary,
    view: &StateView<'_>,
) -> Option<ConfirmModel> {
    let key = pending_key(action, view)?;
    let shown = effect_with_time(effect, action);
    let title = format!("{label}?");
    if let EngineAction::Compose { target, command } = action {
        let project = view
            .projects
            .iter()
            .find(|p| Subject::Project(p.id.clone()) == key.subject)?;
        return Some(confirm::compose_fix_model(
            project,
            title,
            shown,
            target.clone(),
            command.clone(),
        ));
    }
    Some(ConfirmModel {
        title,
        effect: shown,
        action: action.clone(),
        checkbox: None,
        targets: vec![key.subject],
    })
}

/// The pending plan a file fix proposes. The project it belongs to supplies what runs after the
/// write (the plan's `after_write`).
fn pending_plan(plan: &GenerationPlan, problem: &Problem, view: &StateView<'_>) -> PendingPlan {
    let project = project_of(problem, view).or_else(|| {
        view.projects
            .iter()
            .find(|p| p.folder.as_deref() == Some(plan.root.as_path()))
    });
    let title = project.map_or_else(
        || {
            plan.root
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        },
        |p| p.name.clone(),
    );
    let mut pending = PendingPlan::new(plan.clone(), title);
    pending.run = project.and_then(PlanRun::for_project);
    pending
}

/// Click fix `index` of `problem`.
pub fn fix_click(
    state: &mut AppState,
    plans: Option<&PlanStore>,
    problem: &Problem,
    index: usize,
) -> Clicked {
    let Some(fix) = problem.fixes.get(index) else {
        return Clicked::Nothing("This fix is gone".to_string());
    };
    if !matches!(fix.source, FixSource::Rule) {
        return Clicked::Nothing("Needs a harness".to_string());
    }
    match &fix.effect {
        FixEffect::Explain => Clicked::Nothing("This fix is advice only".to_string()),
        FixEffect::EditFiles(plan) => {
            let Some(plans) = plans else {
                return Clicked::Nothing("Review changes is not available here".to_string());
            };
            let pending = with_view(state, |view| {
                if let Some(reason) = view.backing.unsupported(Capability::WriteFiles) {
                    return Err(reason);
                }
                Ok(pending_plan(plan, problem, view))
            });
            match pending {
                Ok(pending) => {
                    let id = plans.insert(pending);
                    state.overlays.write().push(Overlay::ReviewChanges(id));
                    Clicked::Review(id)
                }
                Err(reason) => Clicked::Nothing(reason),
            }
        }
        FixEffect::RunAction {
            action: EngineAction::Prune { plan },
            ..
        } => {
            state
                .overlays
                .write()
                .push(Overlay::CleanUp(plan.engine.clone()));
            Clicked::CleanUp
        }
        FixEffect::RunAction { action, effect } => {
            let model = with_view(state, |view| {
                if let Some(reason) = disabled_for(&fix.effect, view) {
                    return Err(reason);
                }
                confirm_model(&fix.label, action, effect, view)
                    .ok_or_else(|| "This fix cannot run from here yet".to_string())
            });
            match model {
                Ok(model) => {
                    state.overlays.write().push(Overlay::Confirm(model));
                    Clicked::Confirm
                }
                Err(reason) => Clicked::Nothing(reason),
            }
        }
    }
}

/// Click the button with id `fix_id` among the buttons of `problems`.
pub fn click(
    state: &mut AppState,
    plans: Option<&PlanStore>,
    problems: &[Problem],
    fix_id: &str,
) -> Clicked {
    if let Some(id) = fix_id.strip_prefix("goto:") {
        select_only(state, &ContainerId(id.to_string()));
        return Clicked::Selected;
    }
    let Some((problem, index)) = fix_id.rsplit_once('#') else {
        return Clicked::Nothing("Unknown fix".to_string());
    };
    let Ok(index) = index.parse::<usize>() else {
        return Clicked::Nothing("Unknown fix".to_string());
    };
    match problems.iter().find(|p| p.id.0 == problem) {
        Some(found) => fix_click(state, plans, found, index),
        None => Clicked::Nothing("This problem is gone".to_string()),
    }
}
