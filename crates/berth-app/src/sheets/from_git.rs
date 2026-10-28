//! New project, From Git (P8-T5): the repository address, a branch, an ssh key from `~/.ssh`
//! (used through the user's agent, never read by Berth), the engine it deploys to and the folder
//! it is cloned into. Clone runs `git clone --progress`; a finished clone goes on to the Detected
//! sheet, where Start project deploys it (or the generate flow takes a folder with no Compose
//! file). The states are the form, cloning, clone failed (with the fix) and invalid compose.
//!
//! The text inputs are uncontrolled (D59). Nothing here runs git: the sheet reads signals and
//! calls [`GitDeployHandle`], which owns the I/O (D23).

use std::path::PathBuf;

use dioxus::prelude::*;

use berth_core::{EngineId, EngineKind, EngineState};

use crate::services::folders::display_path;
use crate::services::git_deploy::{
    clone_blocker, list_ssh_keys, parse_repo_url, CloneRequest, CloneState, FailureKind,
    GitDeployHandle, GitDeployView,
};
use crate::services::plans::{blocking, target_engine};
use crate::state::{AppState, Overlay};
use crate::ui::progress_bar::PROGRESS_BAR_CSS;
use crate::ui::{
    Button, ButtonVariant, ControlSize, Icon, IconSize, InlineMessage, InlineTone, ProgressBar,
    Select, SelectOption, TextField,
};

/// The sheet's CSS, rendered with it (tokens only).
pub const FROM_GIT_CSS: &str = include_str!("../../assets/css/from_git.css");

/// The Select value for "no particular key".
const AGENT: &str = "";

/// A scene or test fills the form and the key list instead of reading `~/.ssh`.
#[derive(Debug, Clone, Default)]
pub struct SeedGit {
    pub url: String,
    pub branch: String,
    pub key: Option<PathBuf>,
    pub keys: Vec<PathBuf>,
    pub parent: Option<PathBuf>,
}

/// The engines a clone can deploy to: running and on this Mac (a remote engine has no folder of
/// this Mac's to build from).
#[must_use]
pub fn clone_engines(engines: &[berth_core::Engine]) -> Vec<(EngineId, String)> {
    engines
        .iter()
        .filter(|e| e.state == EngineState::Running && !matches!(e.kind, EngineKind::Remote { .. }))
        .map(|e| (e.id.clone(), e.name.clone()))
        .collect()
}

/// "Receiving objects 40%", or the phase alone while git has no number.
#[must_use]
pub fn progress_text(phase: &str, percent: Option<u8>) -> String {
    match percent {
        Some(p) => format!("{phase} {p}%"),
        None => phase.to_string(),
    }
}

fn key_label(key: &std::path::Path) -> String {
    key.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// The page: form, progress, failure or invalid compose, by the clone's state.
#[component]
pub fn GitStage(on_back: EventHandler<()>) -> Element {
    let mut state = use_context::<AppState>();
    let view = use_context::<GitDeployView>();
    let handle = try_consume_context::<Option<GitDeployHandle>>().flatten();
    let seed = try_consume_context::<SeedGit>();

    let mut url = use_signal({
        let seed = seed.clone();
        move || seed.map_or_else(String::new, |s| s.url)
    });
    let mut branch = use_signal({
        let seed = seed.clone();
        move || seed.map_or_else(String::new, |s| s.branch)
    });
    let mut key = use_signal({
        let seed = seed.clone();
        move || seed.and_then(|s| s.key)
    });
    let mut keys = use_signal({
        let seed = seed.clone();
        move || seed.map(|s| s.keys).unwrap_or_default()
    });
    let mut parent = use_signal({
        let seed = seed.clone();
        move || seed.and_then(|s| s.parent)
    });
    let running = clone_engines(&state.engines.read());
    let mut engine = use_signal({
        let running = running.clone();
        move || {
            target_engine(&state.engine_scope.peek(), &state.engines.peek())
                .filter(|id| running.iter().any(|(r, _)| r == id))
                .or_else(|| running.first().map(|(id, _)| id.clone()))
        }
    });

    // The keys in ~/.ssh, read once off the UI path; a scene brings its own.
    let seeded = seed.is_some();
    use_effect(move || {
        if seeded {
            return;
        }
        spawn(async move {
            let found = blocking(|| list_ssh_keys(&berth_sys::paths::home().join(".ssh"))).await;
            keys.set(found.unwrap_or_default());
        });
    });

    let phase = view.cloning.read().clone();
    let initial_url = url.read().clone();
    let initial_branch = branch.read().clone();
    let home = berth_sys::paths::home();
    let chosen = parent.read().clone();
    let shown = chosen.as_ref().map(|p| display_path(p, &home));
    let url_problem = {
        let text = url.read();
        if text.trim().is_empty() {
            None
        } else {
            parse_repo_url(&text).err()
        }
    };
    let blocker = clone_blocker(&url.read(), chosen.as_deref(), engine.read().as_ref());
    let backing = *state.backing.read();
    let blocker = backing
        .unsupported(crate::services::backing::Capability::WriteFiles)
        .or(blocker);
    let key_options: Vec<SelectOption> =
        std::iter::once(SelectOption::new(AGENT, "Use whatever my ssh-agent offers"))
            .chain(
                keys.read()
                    .iter()
                    .map(|k| SelectOption::new(k.to_string_lossy(), key_label(k))),
            )
            .collect();
    let engine_options: Vec<SelectOption> = running
        .iter()
        .map(|(id, name)| SelectOption::new(id.0.clone(), name.clone()))
        .collect();

    let pick = move |_| {
        spawn(async move {
            let mut dialog = rfd::AsyncFileDialog::new().set_title("Clone into");
            if let Some(start) = parent.peek().clone() {
                dialog = dialog.set_directory(start);
            }
            if let Some(picked) = dialog.pick_folder().await {
                parent.set(Some(picked.path().to_path_buf()));
            }
        });
    };
    let clone_handle = handle.clone();
    let clone = move |_| {
        let (Some(handle), Ok(address), Some(parent), Some(engine)) = (
            clone_handle.clone(),
            parse_repo_url(&url.peek()),
            parent.peek().clone(),
            engine.peek().clone(),
        ) else {
            return;
        };
        let branch = branch.peek().trim().to_string();
        handle.clone_project(
            view,
            state,
            CloneRequest {
                url: address,
                branch: (!branch.is_empty()).then_some(branch),
                ssh_key: key.peek().clone(),
                parent,
                engine,
            },
        );
    };
    let cancel_handle = handle.clone();
    let reset = move |()| {
        let mut cloning = view.cloning;
        cloning.set(CloneState::Idle);
    };

    rsx! {
        style { "{FROM_GIT_CSS}" }
        style { "{PROGRESS_BAR_CSS}" }
        match phase {
            CloneState::Idle => rsx! {
                p { class: "np-lede", "Clone a repository. Berth reads it like any folder, then you deploy it." }
                div { class: "fg-form",
                    TextField {
                        label: "Repository".to_string(),
                        initial_value: initial_url,
                        placeholder: "git@github.com:owner/repo.git or https://github.com/owner/repo.git".to_string(),
                        aria_label: "Repository address".to_string(),
                        error: url_problem,
                        oninput: move |e: FormEvent| url.set(e.value()),
                    }
                    div { class: "fg-pair",
                        TextField {
                            label: "Branch".to_string(),
                            initial_value: initial_branch,
                            placeholder: "The repository's default".to_string(),
                            aria_label: "Branch".to_string(),
                            oninput: move |e: FormEvent| branch.set(e.value()),
                        }
                        Select {
                            label: "Target engine".to_string(),
                            aria_label: "Target engine".to_string(),
                            options: engine_options,
                            value: engine.read().as_ref().map(|id| id.0.clone()).unwrap_or_default(),
                            disabled: running.is_empty(),
                            onchange: move |e: FormEvent| engine.set(Some(EngineId(e.value()))),
                        }
                    }
                    Select {
                        label: "SSH key".to_string(),
                        aria_label: "SSH key".to_string(),
                        options: key_options,
                        value: key.read().as_ref().map(|k| k.to_string_lossy().into_owned()).unwrap_or_default(),
                        onchange: move |e: FormEvent| {
                            let value = e.value();
                            key.set((!value.is_empty()).then(|| PathBuf::from(value)));
                        },
                    }
                    p { class: "fg-help",
                        "For ssh:// and git@ addresses. Berth never reads the key: your ssh-agent signs for it (ssh-add loads a key)."
                    }
                    div { class: "np-folder",
                        span { class: "np-folder-label", "Clone into" }
                        span { class: "np-folder-path",
                            if let Some(shown) = shown {
                                code { "{shown}" }
                            } else {
                                span { class: "np-muted", "No folder chosen" }
                            }
                        }
                        Button {
                            label: "Choose folder\u{2026}".to_string(),
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Md,
                            onclick: pick,
                        }
                    }
                }
                div { class: "np-footer",
                    p { class: "np-footer-note", "Nothing is written to your project until you review it." }
                    Button {
                        label: "Back".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Md,
                        onclick: move |_| on_back.call(()),
                    }
                    Button {
                        label: "Clone".to_string(),
                        icon: "git-branch",
                        variant: ButtonVariant::Primary,
                        size: ControlSize::Md,
                        disabled: blocker.is_some() || (handle.is_none() && !seeded),
                        disabled_reason: blocker.or_else(|| (handle.is_none() && !seeded).then(|| "Git is not connected".to_string())),
                        onclick: clone,
                    }
                }
            },
            CloneState::Cloning { url: address, phase, percent } => rsx! {
                p { class: "np-lede", role: "status", "Cloning {address}\u{2026}" }
                div { class: "fg-progress",
                    ProgressBar {
                        fraction: percent.map(|p| f32::from(p) / 100.0),
                        label: phase.clone(),
                        value_text: progress_text(&phase, percent),
                        aria_label: "Clone progress".to_string(),
                    }
                }
                div { class: "np-footer",
                    Button {
                        label: "Cancel".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Md,
                        onclick: move |_| {
                            if let Some(handle) = &cancel_handle {
                                handle.cancel_clone();
                            }
                        },
                    }
                }
            },
            CloneState::Failed(failure) => rsx! {
                div { class: "fg-failure",
                    InlineMessage {
                        message: match failure.kind {
                            FailureKind::Auth => "The clone was refused: git could not sign in.".to_string(),
                            FailureKind::Url => "The clone found nothing at that address.".to_string(),
                            FailureKind::HostKey => "The clone stopped: ssh does not trust that host yet.".to_string(),
                            FailureKind::Other => "The clone did not finish.".to_string(),
                        },
                        tone: InlineTone::Danger,
                    }
                    p { class: "fg-git-said", title: "What git said", "{failure.message}" }
                    p { class: "fg-fix",
                        Icon { name: "info", size: IconSize::Sm }
                        span { "{failure.fix}" }
                    }
                }
                div { class: "np-footer",
                    Button {
                        label: "Edit the address".to_string(),
                        variant: ButtonVariant::Primary,
                        size: ControlSize::Md,
                        onclick: move |_| reset(()),
                    }
                }
            },
            CloneState::InvalidCompose { folder, message } => {
                let shown = display_path(&folder, &home);
                rsx! {
                    div { class: "fg-failure",
                        InlineMessage {
                            message: "The repository was cloned, but its Compose file does not load.".to_string(),
                            tone: InlineTone::Warning,
                        }
                        p { class: "fg-git-said", "{message}" }
                        p { class: "fg-fix",
                            Icon { name: "info", size: IconSize::Sm }
                            span { "The folder is at {shown}. Fix the file in your editor and open the folder again, or open it now to see what Berth found." }
                        }
                    }
                    div { class: "np-footer",
                        Button {
                            label: "Close".to_string(),
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Md,
                            onclick: move |_| {
                                let mut cloning = view.cloning;
                                cloning.set(CloneState::Idle);
                                state.overlays.write().retain(|o| !matches!(o, Overlay::NewProject));
                            },
                        }
                        Button {
                            label: "Open folder".to_string(),
                            variant: ButtonVariant::Primary,
                            size: ControlSize::Md,
                            onclick: {
                                let folder = folder.clone();
                                move |_| {
                                    let mut cloning = view.cloning;
                                    cloning.set(CloneState::Idle);
                                    let mut overlays = state.overlays.write();
                                    overlays.retain(|o| !matches!(o, Overlay::NewProject));
                                    overlays.push(Overlay::Detected(folder.clone()));
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "from_git_tests.rs"]
mod tests;
