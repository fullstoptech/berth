//! The Assistant bottom tab (P7-T5, frame "Project / assistant /
//! 1440"): the conversation for the selected project on the left, its
//! proposed changes and pending approvals on the right. The "full
//! section" is this same body in a maximized bottom panel (AD37).
//!
//! The screen reads signals and calls the assistant service; it never
//! talks to a harness or an engine itself (D23). With no service (a
//! scene), the same state renders and a click updates the signals
//! locally, so a shot can be poked at by hand.

use std::path::PathBuf;

use dioxus::prelude::*;

use berth_core::{
    AssistantSession, Container, ContextItem, Harness, Problem, Project, ProposalState,
    SessionState, TurnPart,
};

use crate::services::assistant::{
    approve_blocked, session_for_project, Answer, AssistantService, OpenRequest,
};
use crate::services::plans::PlanStore;
use crate::state::{AppState, Approval, Overlay, Selection};
use crate::ui::key_value_list::copy_to_clipboard;
use crate::ui::{
    ApprovalCard, Banner, BannerTone, Button, ButtonVariant, ChatTurn, ContextChip, Icon, IconSize,
    InlineMessage, ProposedChangeCard, Switch, TextField,
};

pub const ASSISTANT_CSS: &str = include_str!("../../assets/css/assistant.css");

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// The harness a new session would use: the open session's, else the
/// last one picked, else the first that is installed.
#[must_use]
pub fn picked_harness(
    harnesses: &[Harness],
    session: Option<&AssistantSession>,
    last: Option<&berth_core::HarnessId>,
) -> Option<Harness> {
    if let Some(session) = session {
        if let Some(found) = harnesses.iter().find(|h| h.id == session.harness) {
            return Some(found.clone());
        }
    }
    if let Some(last) = last {
        if let Some(found) = harnesses.iter().find(|h| h.id == *last && h.available) {
            return Some(found.clone());
        }
    }
    harnesses.iter().find(|h| h.available).cloned()
}

/// The session request for a project and harness; a project with no
/// folder on this Mac gives the assistant nowhere to work.
#[must_use]
pub fn open_request(project: &Project, harness: &Harness) -> Option<OpenRequest> {
    Some(OpenRequest {
        harness: harness.clone(),
        project: project.id.clone(),
        project_name: project.name.clone(),
        folder: project.folder.clone()?,
    })
}

fn base_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The icon and label of one context chip.
#[must_use]
pub fn context_chip_model(
    item: &ContextItem,
    problems: &[Problem],
    containers: &[Container],
) -> (&'static str, String) {
    match item {
        ContextItem::Logs { name, lines, .. } => {
            ("logs", format!("{name} \u{b7} {} lines", lines.len()))
        }
        ContextItem::Problem(id) => (
            "alert-triangle",
            problems
                .iter()
                .find(|p| &p.id == id)
                .map_or_else(|| "Problem".to_string(), |p| p.title.clone()),
        ),
        ContextItem::ComposeFile(path)
        | ContextItem::Dockerfile(path)
        | ContextItem::File(path) => ("file-text", base_name(path)),
        ContextItem::Inspect(id) => (
            "box",
            containers
                .iter()
                .find(|c| &c.id == id)
                .map_or_else(|| "Container".to_string(), |c| c.name.clone()),
        ),
        ContextItem::FolderTree(path) => ("folder", base_name(path)),
    }
}

/// One proposed change as its card draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub change_id: uuid::Uuid,
    pub title: String,
    pub files: Vec<(PathBuf, u32, u32)>,
    pub state: ProposalState,
}

/// Every proposed change in a session, in the order it was proposed.
#[must_use]
pub fn proposals_of(session: &AssistantSession) -> Vec<Proposal> {
    session
        .turns
        .iter()
        .flat_map(|turn| &turn.parts)
        .filter_map(|part| match part {
            TurnPart::ProposedChange {
                change_id,
                title,
                files,
                state,
            } => Some(Proposal {
                change_id: *change_id,
                title: title.clone(),
                files: files.clone(),
                state: *state,
            }),
            _ => None,
        })
        .collect()
}

/// This session's pending approvals, oldest first.
#[must_use]
pub fn approvals_of(approvals: &[Approval], session: &AssistantSession) -> Vec<Approval> {
    approvals
        .iter()
        .filter(|a| a.session == session.id)
        .cloned()
        .collect()
}

/// The install hint of one harness id: what to paste in Terminal.
/// `muse` has no package command Berth can name, so its hint is the
/// check that the command answers once it is installed.
#[must_use]
pub fn install_hint(id: &str) -> (&'static str, &'static str) {
    match id {
        "claude-code" | "claude" => (
            "Install Claude Code:",
            "npm install -g @anthropic-ai/claude-code",
        ),
        "muse" => (
            "Install muse so the muse command is on your PATH, then check:",
            "muse --version",
        ),
        _ => (
            "Install it so its command is on your PATH, then check:",
            "--version",
        ),
    }
}

/// The footnote under the input (frame copy, D79).
#[must_use]
pub fn footnote(harness: Option<&Harness>) -> String {
    let who = harness.map_or("the assistant", |h| h.name.as_str());
    format!(".env values stay on this Mac. {who} sees key names and ports only.")
}

pub(crate) fn current_project(state: &AppState) -> Option<Project> {
    let Selection::Project(id) = &*state.selection.peek() else {
        return None;
    };
    state.projects.peek().iter().find(|p| &p.id == id).cloned()
}

/// The harness the picker in the panel head shows for the state now.
#[must_use]
pub(crate) fn picked_harness_for_state(state: &AppState) -> Option<Harness> {
    let project = current_project(state);
    let sessions = state.assistant.peek();
    let session = project
        .as_ref()
        .and_then(|p| session_for_project(&sessions, &p.id));
    let last = state.settings.peek().last_harness.clone();
    picked_harness(&state.harnesses.peek(), session, last.as_ref())
}

/// What the screen shows, worked out from the signals in one place so
/// the component body stays a rendering of it.
struct Shown {
    project: Option<Project>,
    harness: Option<Harness>,
    session: Option<AssistantSession>,
}

fn shown(state: &AppState) -> Shown {
    let _ = state.selection.read();
    let _ = state.projects.read();
    let _ = state.settings.read();
    let project = current_project(state);
    let sessions = state.assistant.read();
    let session = project
        .as_ref()
        .and_then(|p| session_for_project(&sessions, &p.id).cloned());
    drop(sessions);
    let last = state.settings.peek().last_harness.clone();
    let harness = picked_harness(&state.harnesses.peek(), session.as_ref(), last.as_ref());
    Shown {
        project,
        harness,
        session,
    }
}

/// The Assistant tab body.
#[component]
pub fn AssistantPanel() -> Element {
    let mut state = use_context::<AppState>();
    let service = try_consume_context::<Option<AssistantService>>().flatten();
    let plans = try_consume_context::<PlanStore>();
    let view = shown(&state);
    let problems = state.problems.read().clone();
    let containers = state.containers.read().clone();
    let approvals = state.approvals.read().clone();
    let harnesses = state.harnesses.read().clone();

    let mut opened = use_signal(|| None::<String>);
    let mut draft = use_signal(String::new);
    let mut sends = use_signal(|| 0_u32);
    let mut refusal = use_signal(|| None::<String>);

    // Open (or resume) the session when the project or the picked
    // harness changes.
    {
        let service = service.clone();
        let key = match (&view.project, &view.harness) {
            (Some(project), Some(harness)) => project
                .folder
                .as_ref()
                .map(|folder| format!("{}|{}", folder.display(), harness.id.0)),
            _ => None,
        };
        let request = match (&view.project, &view.harness) {
            (Some(project), Some(harness)) => open_request(project, harness),
            _ => None,
        };
        use_effect(move || {
            let (Some(service), Some(request), Some(key)) =
                (service.clone(), request.clone(), key.clone())
            else {
                return;
            };
            if opened.peek().as_deref() == Some(key.as_str()) {
                return;
            }
            opened.set(Some(key));
            spawn(async move {
                let _ = service.open(request).await;
            });
        });
    }

    // A plan written or dropped in Review changes moves its card.
    {
        let service = service.clone();
        let plans = plans.clone();
        use_effect(move || {
            let _ = state.overlays.read();
            if let (Some(service), Some(plans)) = (&service, &plans) {
                service.reconcile_plans(plans);
            }
        });
    }

    let project = view.project.clone();
    let harness = view.harness.clone();
    let session = view.session.clone();
    let no_harness = !harnesses.iter().any(|h| h.available);
    let request = match (&project, &harness) {
        (Some(project), Some(harness)) => open_request(project, harness),
        _ => None,
    };
    let proposals = session.as_ref().map(proposals_of).unwrap_or_default();
    let session_approvals = session
        .as_ref()
        .map(|s| approvals_of(&approvals, s))
        .unwrap_or_default();
    let total_steps = proposals.len() + session_approvals.len();
    let has_cards = total_steps > 0;
    // Stop interrupts a running turn; a turn waiting for approval is
    // answered on its card, not stopped (the frame shows no Stop there).
    let busy = matches!(
        session.as_ref().map(|s| &s.state),
        Some(SessionState::Thinking)
    );
    let project_name = project
        .as_ref()
        .map_or_else(String::new, |p| p.name.clone());
    let input_disabled = project.is_none() || no_harness;

    let on_send = {
        let service = service.clone();
        let session = session.clone();
        let request = request.clone();
        move |_| {
            let text = draft.peek().trim().to_string();
            if text.is_empty() {
                return;
            }
            draft.set(String::new());
            let next_send = sends.peek().wrapping_add(1);
            sends.set(next_send);
            refusal.set(None);
            match (&service, &session) {
                (Some(service), Some(session)) => {
                    if let Err(reason) = service.send(session.id, text, None) {
                        refusal.set(Some(reason));
                    }
                }
                (Some(service), None) => {
                    let service = service.clone();
                    let request = request.clone();
                    spawn(async move {
                        if let Some(request) = request {
                            if let Ok(id) = service.open(request).await {
                                let _ = service.send(id, text, None);
                            }
                        }
                    });
                }
                (None, _) => {}
            }
        }
    };

    rsx! {
        style { "{ASSISTANT_CSS}" }
        div { class: "assistant", role: "region", "aria-label": "Assistant",
            div { class: if has_cards { "assistant-cols has-cards" } else { "assistant-cols" },
                div { class: "assistant-main",
                    if session.as_ref().is_some_and(|s| s.watch) {
                        Banner {
                            message: format!(
                                "Watching {project_name}: Berth follows its logs and proposes fixes here. Nothing is applied without your click."
                            ),
                            tone: BannerTone::Info,
                        }
                    }
                    div { class: "assistant-turns",
                        if project.is_none() {
                            div { class: "assistant-empty",
                                p { class: "assistant-empty-title", "No project selected" }
                                p { class: "assistant-empty-text",
                                    "Open a project, then ask about its containers, logs and Compose file."
                                }
                            }
                        } else if project.as_ref().is_some_and(|p| p.folder.is_none()) {
                            div { class: "assistant-empty",
                                p { class: "assistant-empty-title", "No folder on this Mac" }
                                p { class: "assistant-empty-text",
                                    "This project has no folder here, so the assistant has nowhere to work."
                                }
                            }
                        } else if no_harness {
                            div { class: "assistant-no-harness",
                                p { class: "assistant-empty-title", "No assistant installed" }
                                p { class: "assistant-empty-text",
                                    "Berth drives an assistant you already have. Install one, then ask here."
                                }
                                for hint_harness in hint_rows(&harnesses) {
                                    div { key: "{hint_harness.0}", class: "assistant-install-row",
                                        span { class: "assistant-install-name", "{hint_harness.1}" }
                                        span { class: "assistant-install-hint", "{hint_harness.2}" }
                                        code { class: "assistant-install-cmd", "{hint_harness.3}" }
                                        Button {
                                            label: "Copy",
                                            variant: ButtonVariant::Ghost,
                                            size: crate::ui::ControlSize::Sm,
                                            icon: "copy",
                                            onclick: {
                                                let command = hint_harness.3.to_string();
                                                move |_| copy_to_clipboard(&command)
                                            },
                                        }
                                    }
                                }
                                p { class: "assistant-empty-text",
                                    "Problems, rules and every other part of Berth keep working without an assistant."
                                }
                            }
                        } else if session.as_ref().is_none_or(|s| s.turns.is_empty()) {
                            div { class: "assistant-empty",
                                p { class: "assistant-empty-text",
                                    "Ask about {project_name} \u{2014} why a container keeps restarting, what is using the most memory, or what a log line means."
                                }
                            }
                        }
                        if let Some(session) = &session {
                            for (index, turn) in session.turns.iter().enumerate() {
                                ChatTurn { key: "{index}", turn: turn.clone() }
                            }
                            if session.state == SessionState::Thinking {
                                p { class: "assistant-thinking",
                                    span { class: "assistant-spin",
                                        Icon { name: "loader", size: IconSize::Sm }
                                    }
                                    "Thinking\u{2026}"
                                }
                            }
                            if let SessionState::Error { stderr } = &session.state {
                                div { class: "assistant-error",
                                    Banner {
                                        message: "The assistant stopped.".to_string(),
                                        tone: BannerTone::Danger,
                                        action_label: "Retry".to_string(),
                                        onaction: {
                                            let service = service.clone();
                                            let request = request.clone();
                                            let id = session.id;
                                            move |_| {
                                                if let (Some(service), Some(request)) =
                                                    (service.clone(), request.clone())
                                                {
                                                    spawn(async move {
                                                        let _ = service.retry(id, request).await;
                                                    });
                                                }
                                            }
                                        },
                                    }
                                    pre { class: "assistant-stderr", "{stderr}" }
                                }
                            }
                        }
                    }
                    if let Some(session) = &session {
                        if !session.context.is_empty() {
                            div { class: "assistant-chips", "aria-label": "Attached context",
                                for (index, item) in session.context.iter().enumerate() {
                                    {
                                        let (icon, label) =
                                            context_chip_model(item, &problems, &containers);
                                        let service = service.clone();
                                        let id = session.id;
                                        rsx! {
                                            ContextChip {
                                                key: "{index}",
                                                icon,
                                                label,
                                                on_remove: move |_| {
                                                    if let Some(service) = &service {
                                                        service.remove_context(id, index);
                                                    } else if let Some(found) = state
                                                        .assistant
                                                        .write()
                                                        .get_mut(&id)
                                                    {
                                                        if index < found.context.len() {
                                                            found.context.remove(index);
                                                        }
                                                    }
                                                },
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "assistant-input-row",
                        div { class: "assistant-input",
                            for send in [sends()] {
                                TextField {
                                    key: "{send}",
                                    initial_value: String::new(),
                                    placeholder: format!("Ask about {project_name}\u{2026}"),
                                    aria_label: format!("Ask about {project_name}"),
                                    disabled: input_disabled,
                                    oninput: move |e: FormEvent| draft.set(e.value()),
                                    on_enter: on_send.clone(),
                                }
                            }
                        }
                        if busy {
                            Button {
                                label: "Stop",
                                variant: ButtonVariant::Ghost,
                                size: crate::ui::ControlSize::Sm,
                                icon: "square",
                                onclick: {
                                    let service = service.clone();
                                    let id = session.as_ref().map(|s| s.id);
                                    move |_| {
                                        if let (Some(service), Some(id)) = (&service, id) {
                                            service.interrupt(id);
                                        }
                                    }
                                },
                            }
                        }
                        Button {
                            label: "Open in Terminal",
                            variant: ButtonVariant::Ghost,
                            size: crate::ui::ControlSize::Sm,
                            icon: "terminal",
                            disabled: service.is_none() || request.is_none(),
                            disabled_reason: if service.is_none() {
                                "The assistant is not connected".to_string()
                            } else {
                                "Open a project first".to_string()
                            },
                            onclick: {
                                let service = service.clone();
                                let request = request.clone();
                                move |_| {
                                    let (Some(service), Some(request)) =
                                        (service.clone(), request.clone())
                                    else {
                                        return;
                                    };
                                    refusal.set(None);
                                    spawn(async move {
                                        let opened = async {
                                            let launch =
                                                service.terminal_launch(request).await?;
                                            crate::platform::open_command_in_terminal_app(
                                                crate::platform::CommandTarget {
                                                    program: launch.program,
                                                    args: launch.args,
                                                    cwd: launch.cwd,
                                                    cleanup: launch.cleanup,
                                                },
                                            )
                                            .await
                                            .map(|_| ())
                                        };
                                        if let Err(reason) = opened.await {
                                            refusal.set(Some(reason));
                                        }
                                    });
                                }
                            },
                        }
                        Switch {
                            on: session.as_ref().is_some_and(|s| s.watch),
                            label: "Watch and fix",
                            aria_label: Some("Watch and fix".to_string()),
                            disabled: session.is_none(),
                            on_toggle: {
                                let service = service.clone();
                                let id = session.as_ref().map(|s| s.id);
                                move |next: bool| {
                                    let Some(id) = id else { return };
                                    if let Some(service) = &service {
                                        service.set_watch(id, next);
                                    } else if let Some(found) =
                                        state.assistant.write().get_mut(&id)
                                    {
                                        found.watch = next;
                                    }
                                }
                            },
                        }
                    }
                    p { class: "assistant-footnote", {footnote(harness.as_ref())} }
                    if let Some(reason) = refusal() {
                        InlineMessage { message: reason }
                    }
                }
                if has_cards {
                    div { class: "assistant-cards",
                        for (index, proposal) in proposals.iter().enumerate() {
                            ProposedChangeCard {
                                key: "{proposal.change_id}",
                                title: proposal.title.clone(),
                                files: proposal.files.clone(),
                                state: proposal.state,
                                step: (index + 1, total_steps),
                                on_review: {
                                    let change_id = proposal.change_id;
                                    move |_| {
                                        state
                                            .overlays
                                            .write()
                                            .push(Overlay::ReviewChanges(change_id));
                                    }
                                },
                            }
                        }
                        for approval in &session_approvals {
                            {
                                let blocked = approve_blocked(
                                    approval,
                                    session.as_ref(),
                                );
                                let service = service.clone();
                                let approval_id = approval.id;
                                let deny_id = approval.id;
                                rsx! {
                                    ApprovalCard {
                                        key: "{approval.id}",
                                        approval: approval.clone(),
                                        blocked_reason: blocked,
                                        on_approve: {
                                            let service = service.clone();
                                            move |typed: Option<String>| {
                                                answer_card(
                                                    &mut state,
                                                    &service,
                                                    approval_id,
                                                    Answer::Approve { typed },
                                                    &mut refusal,
                                                );
                                            }
                                        },
                                        on_deny: move |_| {
                                            answer_card(
                                                &mut state,
                                                &service,
                                                deny_id,
                                                Answer::Deny,
                                                &mut refusal,
                                            );
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
}

/// (id, name, hint, copyable command) for every harness that is not
/// installed; detection's own list, or the two Berth knows when it is
/// empty (a scene).
fn hint_rows(harnesses: &[Harness]) -> Vec<(String, String, &'static str, &'static str)> {
    let rows: Vec<(String, String, &'static str, &'static str)> = harnesses
        .iter()
        .filter(|h| !h.available)
        .map(|h| {
            let (hint, command) = install_hint(&h.id.0);
            (h.id.0.clone(), h.name.clone(), hint, command)
        })
        .collect();
    if rows.is_empty() {
        ["claude-code", "muse"]
            .into_iter()
            .map(|id| {
                let (hint, command) = install_hint(id);
                let name = if id == "claude-code" {
                    "Claude Code"
                } else {
                    "muse"
                };
                (id.to_string(), name.to_string(), hint, command)
            })
            .collect()
    } else {
        rows
    }
}

/// Answer a card: through the service when there is one; in a scene the
/// same refusals apply and the card simply leaves the signal.
fn answer_card(
    state: &mut AppState,
    service: &Option<AssistantService>,
    id: uuid::Uuid,
    answer: Answer,
    refusal: &mut Signal<Option<String>>,
) {
    if let Some(service) = service {
        match service.answer(id, answer) {
            Ok(()) => refusal.set(None),
            Err(reason) => refusal.set(Some(reason)),
        }
        return;
    }
    let approval = state.approvals.peek().iter().find(|a| a.id == id).cloned();
    let Some(approval) = approval else { return };
    if let Answer::Approve { typed } = &answer {
        if let Some(expected) = &approval.typed_name {
            if typed.as_deref() != Some(expected.as_str()) {
                refusal.set(Some(format!("Type {expected} to confirm")));
                return;
            }
        }
        let session = state.assistant.peek().get(&approval.session).cloned();
        if let Some(reason) = approve_blocked(&approval, session.as_ref()) {
            refusal.set(Some(reason));
            return;
        }
    }
    refusal.set(None);
    state.approvals.write().retain(|a| a.id != id);
}

#[cfg(test)]
#[path = "assistant_tests.rs"]
mod tests;
