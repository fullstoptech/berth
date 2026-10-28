//! The Git panel of a project Berth cloned (P8-T5, D72): where it came from, the commit that is
//! deployed, the commits waiting upstream, and Check, Pull and redeploy, the polling interval and
//! the Auto-redeploy switch. Nothing runs here; the buttons call [`GitDeployHandle`], which
//! fetches, pulls (fast-forward only) and then runs `Up` with a build through the actions service.

use std::path::PathBuf;

use dioxus::prelude::*;

use crate::services::git_deploy::{
    new_commits_label, Busy, GitDeployHandle, GitDeployView, GitStatus, POLL_CHOICES,
};
use crate::sheets::from_git::FROM_GIT_CSS;
use crate::state::AppState;
use crate::ui::{
    Button, ButtonVariant, ControlSize, Icon, IconSize, InlineMessage, Select, SelectOption, Switch,
};

/// Commits listed before "and N more".
const SHOWN: usize = 8;

/// The Select value of a polling interval.
fn poll_value(minutes: Option<u64>) -> String {
    minutes.map_or_else(|| "off".to_string(), |m| m.to_string())
}

/// The polling choices, plus the stored interval when it is none of them.
#[must_use]
pub fn poll_options(current: Option<u64>) -> Vec<SelectOption> {
    let mut options: Vec<SelectOption> = POLL_CHOICES
        .iter()
        .map(|(minutes, label)| SelectOption::new(poll_value(*minutes), *label))
        .collect();
    if let Some(minutes) = current {
        if !POLL_CHOICES.iter().any(|(m, _)| *m == Some(minutes)) {
            options.push(SelectOption::new(
                poll_value(Some(minutes)),
                format!("Every {minutes} minutes"),
            ));
        }
    }
    options
}

/// Why Pull and redeploy is off, or `None`.
#[must_use]
pub fn pull_blocker(status: &GitStatus) -> Option<String> {
    match status.busy {
        Busy::Checking => Some("Looking for new commits".to_string()),
        Busy::Pulling => Some("Pulling".to_string()),
        Busy::Idle if status.incoming.is_empty() => {
            Some("No new commits. Check for new commits first.".to_string())
        }
        Busy::Idle => None,
    }
}

/// The panel for the project in `folder`; nothing when Berth did not clone it.
#[component]
pub fn GitPanel(folder: PathBuf) -> Element {
    let state = use_context::<AppState>();
    let Some(view) = try_consume_context::<GitDeployView>() else {
        return rsx! {};
    };
    let handle = try_consume_context::<Option<GitDeployHandle>>().flatten();
    let Some(status) = view.status(&folder) else {
        return rsx! {};
    };

    let blocker = pull_blocker(&status);
    let working = status.busy != Busy::Idle;
    let minutes = status
        .project
        .poll
        .map(|every| every.as_secs() / 60)
        .filter(|m| *m > 0);
    let count = status.incoming.len();
    let hidden = count.saturating_sub(SHOWN);
    let repo = status.project.repo.to_string();
    let no_handle = handle.is_none().then(|| "Git is not connected".to_string());

    let check = {
        let (handle, folder) = (handle.clone(), folder.clone());
        move |_| {
            if let Some(handle) = &handle {
                handle.check_project(view, folder.clone());
            }
        }
    };
    let pull = {
        let (handle, folder) = (handle.clone(), folder.clone());
        move |_| {
            if let Some(handle) = &handle {
                handle.pull_project(view, state, folder.clone());
            }
        }
    };

    rsx! {
        style { "{FROM_GIT_CSS}" }
        section { class: "gp-panel", "aria-label": "Git",
            div { class: "gp-head",
                span { class: "gp-title",
                    Icon { name: "git-branch", size: IconSize::Md }
                    "{status.project.branch}"
                }
                if let Some(commit) = &status.project.commit {
                    span { class: "gp-commit", title: "Deployed commit", "{commit}" }
                }
                span { class: "gp-repo", title: "{repo}", "{repo}" }
                if count > 0 {
                    span { class: "gp-new", role: "status", {new_commits_label(count)} }
                }
            }
            if count > 0 {
                ul { class: "gp-incoming", "aria-label": "New commits",
                    for commit in status.incoming.iter().take(SHOWN) {
                        li { key: "{commit.hash}", class: "gp-commit-row",
                            span { class: "gp-hash", "{commit.hash}" }
                            span { class: "gp-subject", title: "{commit.subject}", "{commit.subject}" }
                        }
                    }
                    if hidden > 0 {
                        li { class: "gp-muted", "and {hidden} more" }
                    }
                }
            }
            if let Some(message) = status.error.clone() {
                InlineMessage { message }
            }
            if let Some(notice) = &status.notice {
                p { class: "gp-notice", role: "status", "{notice}" }
            }
            div { class: "gp-controls",
                Button {
                    label: if status.busy == Busy::Checking { "Checking\u{2026}".to_string() } else { "Check for new commits".to_string() },
                    icon: "arrow-down",
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    loading: status.busy == Busy::Checking,
                    disabled: working || no_handle.is_some(),
                    disabled_reason: no_handle.clone(),
                    onclick: check,
                }
                Button {
                    label: if status.busy == Busy::Pulling { "Pulling\u{2026}".to_string() } else { "Pull and redeploy".to_string() },
                    icon: "rotate-cw",
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    loading: status.busy == Busy::Pulling,
                    disabled: blocker.is_some() || no_handle.is_some(),
                    disabled_reason: blocker.or(no_handle),
                    onclick: pull,
                }
                span { class: "gp-spacer" }
                div { class: "gp-poll",
                    Select {
                        label: "Look for new commits".to_string(),
                        aria_label: "Look for new commits".to_string(),
                        options: poll_options(minutes),
                        value: poll_value(minutes),
                        onchange: {
                            let (handle, folder) = (handle.clone(), folder.clone());
                            move |e: FormEvent| {
                                if let Some(handle) = &handle {
                                    handle.set_poll_minutes(view, &folder, e.value().parse::<u64>().ok());
                                }
                            }
                        },
                    }
                }
                Switch {
                    on: status.project.auto_redeploy,
                    label: "Auto-redeploy".to_string(),
                    aria_label: "Auto-redeploy when new commits arrive".to_string(),
                    disabled: status.project.poll.is_none(),
                    on_toggle: {
                        let (handle, folder) = (handle.clone(), folder.clone());
                        move |on: bool| {
                            if let Some(handle) = &handle {
                                handle.set_auto_redeploy(view, &folder, on);
                            }
                        }
                    },
                }
            }
            if status.project.poll.is_none() {
                p { class: "gp-muted",
                    "Polling is off. Choose an interval to look for new commits on its own; Auto-redeploy then pulls and redeploys them."
                }
            }
        }
    }
}
