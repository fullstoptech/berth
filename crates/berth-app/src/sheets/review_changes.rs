//! The Review changes sheet (P5-T3; ADR 0012; frames "Review changes / default / 1440" and
//! "Review changes / new project / 1440"). Every file Berth proposes to write to a project goes
//! through here and nowhere else: the plan is parked in the [`PlanStore`] under a uuid, this sheet
//! shows it, and only its buttons call [`write_plan`].
//!
//! The sheet is keyed by the plan id, so a later task (Add service, Problems fixes, the assistant)
//! opens it with `Overlay::ReviewChanges(id)` after `PlanStore::insert`.
//!
//! Values in `.env` and `.env.*` are masked in the file list preview and in the diff, with one
//! Reveal per file (CLAUDE.md: secrets). The sheet never logs a plan.
//!
//! States: checking the compose text, ready, writing, starting, conflict (a file changed since the
//! plan was made: nothing was written, Regenerate), write error, start error (the files are
//! written), and an empty plan.

use std::collections::BTreeSet;
use std::path::PathBuf;

use dioxus::prelude::*;
use uuid::Uuid;

use crate::services::actions::ActionsService;
use crate::services::backing::Capability;
use crate::services::plans::{
    commit, compose_file_of, env_for_validation, off_ui, regenerate, Committed, PlanStore,
};
use crate::services::Services;
use crate::state::{AppState, Overlay, PendingKey, PendingState, Toast, ToastTone};
use crate::ui::diff_view::{added_key_lines, is_secret_file, DIFF_VIEW_CSS};
use crate::ui::{
    Button, ButtonVariant, ControlSize, DiffCard, DiffFileList, InlineMessage, InlineTone,
};
use model::{compose_failure_text, file_views, footer_labels, source_line, summary};

mod model;

/// The sheet's CSS, rendered with it (tokens only).
pub const REVIEW_CHANGES_CSS: &str = include_str!("../../assets/css/review_changes.css");

/// Whether the compose text the plan writes has been checked with `docker compose config`.
#[derive(Debug, Clone, PartialEq)]
enum Check {
    Checking,
    Valid,
    Invalid(String),
}

/// What the sheet is doing.
#[derive(Debug, Clone, PartialEq)]
enum Phase {
    Idle,
    Regenerating,
    Writing,
    /// The files are written and the command runs; the pending key says how it is going.
    Starting(PendingKey),
    /// A file changed since the plan was made: nothing was written.
    Conflict(PathBuf),
    /// The write failed (nothing was written).
    Failed(String),
    /// The files are written, the command failed.
    StartFailed(String),
}

/// Scenes and tests put the sheet into a state the frames do not draw before it is drawn. With
/// one set, the sheet does not check the compose text itself.
#[derive(Debug, Clone, PartialEq)]
pub enum DevState {
    Conflict(PathBuf),
    WriteFailed(String),
    StartFailed(String),
    Invalid(String),
    Checking,
}

/// The most of Compose's complaint the sheet quotes.
const CHECK_TEXT_MAX: usize = 400;

fn clip(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= CHECK_TEXT_MAX {
        return text.to_string();
    }
    let cut: String = text.chars().take(CHECK_TEXT_MAX).collect();
    format!("{cut}\u{2026}")
}

/// Close the sheet, and the Generate or Detected sheet that opened it when the files are written.
fn close(state: &mut AppState, id: Uuid, root: Option<&std::path::Path>) {
    state.overlays.write().retain(|o| match o {
        Overlay::ReviewChanges(other) => *other != id,
        Overlay::Generate(folder) | Overlay::Detected(folder) => root != Some(folder.as_path()),
        // Add service opened this sheet: once the files are written it has nothing left to ask.
        Overlay::AddService(_) => root.is_none(),
        _ => true,
    });
}

/// "Adds 2 keys. Existing keys unchanged." under the key preview.
fn keys_caption(count: usize, existing_file: bool) -> String {
    let noun = if count == 1 { "key" } else { "keys" };
    if existing_file {
        format!("Adds {count} {noun}. Existing keys unchanged.")
    } else {
        format!("Adds {count} {noun}.")
    }
}

fn toast(state: &mut AppState, text: String) {
    state.toasts.write().push(Toast {
        id: Uuid::now_v7(),
        text,
        tone: ToastTone::Success,
        action: None,
    });
}

#[component]
pub fn ReviewChangesSheet(id: Uuid) -> Element {
    let mut state = use_context::<AppState>();
    let store = try_consume_context::<PlanStore>();
    let services = try_consume_context::<Services>();
    let link = try_consume_context::<Option<crate::services::engines::EngineLink>>().flatten();
    let actions = try_consume_context::<ActionsService>();

    let mut plan_entry = use_signal({
        let store = store.clone();
        move || store.as_ref().and_then(|s| s.get(&id))
    });
    let mut selected = use_signal(|| 0_usize);
    let mut revealed = use_signal(BTreeSet::<String>::new);
    let mut expanded = use_signal(BTreeSet::<String>::new);
    let dev = try_consume_context::<DevState>();
    let dev_set = dev.is_some();
    let mut phase = use_signal({
        let dev = dev.clone();
        move || match dev {
            Some(DevState::Conflict(path)) => Phase::Conflict(path),
            Some(DevState::WriteFailed(message)) => Phase::Failed(message),
            Some(DevState::StartFailed(message)) => Phase::StartFailed(message),
            _ => Phase::Idle,
        }
    });
    let needs_check = dev.is_none()
        && actions.is_some()
        && plan_entry
            .peek()
            .as_ref()
            .is_some_and(|p| compose_file_of(&p.plan).is_some());
    let mut check = use_signal(move || match dev {
        Some(DevState::Invalid(message)) => Check::Invalid(message),
        Some(DevState::Checking) => Check::Checking,
        _ if needs_check => Check::Checking,
        _ => Check::Valid,
    });

    // The plan is dropped from the store when the sheet goes.
    use_drop({
        let store = store.clone();
        move || {
            if let Some(store) = &store {
                store.remove(&id);
            }
        }
    });

    // Check the compose text before Write is offered (AD33), again after a Regenerate.
    {
        let actions = actions.clone();
        use_effect(move || {
            let Some(p) = plan_entry.read().clone() else {
                return;
            };
            if dev_set {
                return;
            }
            let Some(actions) = actions.clone() else {
                return;
            };
            let Some((_, text)) = compose_file_of(&p.plan) else {
                check.set(Check::Valid);
                return;
            };
            let text = text.to_string();
            let env = env_for_validation(&p.plan);
            let folder = p.plan.root.clone();
            check.set(Check::Checking);
            spawn(async move {
                let result =
                    off_ui(
                        async move { actions.validate_compose_text(&folder, &text, &env).await },
                    )
                    .await;
                check.set(match result {
                    Some(Ok(())) | None => Check::Valid,
                    Some(Err(message)) => Check::Invalid(clip(&message)),
                });
            });
        });
    }

    // A started command ends the sheet when it succeeds and says why when it does not.
    let mut was_running = use_signal(|| false);
    {
        let store = store.clone();
        use_effect(move || {
            let Phase::Starting(key) = phase.read().clone() else {
                return;
            };
            let current = state.pending.read().get(&key).cloned();
            match current {
                Some(PendingState::Running { .. }) => {
                    if !*was_running.peek() {
                        was_running.set(true);
                    }
                }
                Some(PendingState::Failed { message }) => {
                    phase.set(Phase::StartFailed(compose_failure_text(&message)));
                }
                None if *was_running.peek() => {
                    let pending = plan_entry.peek().clone();
                    let name = pending
                        .as_ref()
                        .and_then(|p| p.run.as_ref().map(|r| r.name.clone()))
                        .unwrap_or_default();
                    let root = pending.as_ref().map(|p| p.plan.root.clone());
                    toast(&mut state, format!("{name} is up"));
                    if let Some(store) = &store {
                        store.remove(&id);
                    }
                    close(&mut state, id, root.as_deref());
                }
                None => {}
            }
        });
    }

    let Some(pending) = plan_entry.read().clone() else {
        return rsx! {
            style { "{REVIEW_CHANGES_CSS}" }
            div {
                class: "rc-sheet",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "Review changes",
                onclick: move |e| e.stop_propagation(),
                h2 { class: "rc-title", "Review changes" }
                p { class: "rc-empty", "This plan is no longer available. Ask for the changes again." }
                div { class: "rc-footer",
                    span { class: "rc-footer-spacer" }
                    Button {
                        label: "Close".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Md,
                        onclick: move |_| close(&mut state, id, None),
                    }
                }
            }
        };
    };

    let harness_name = {
        let harnesses = state.harnesses.read().clone();
        move |hid: &berth_core::HarnessId| {
            harnesses
                .iter()
                .find(|h| h.id == *hid)
                .map_or_else(|| hid.0.clone(), |h| h.name.clone())
        }
    };
    let plan = &pending.plan;
    let backing = *state.backing.read();
    let busy = matches!(
        &*phase.read(),
        Phase::Writing | Phase::Regenerating | Phase::Starting(_)
    );
    let conflict = match &*phase.read() {
        Phase::Conflict(path) => Some(path.clone()),
        _ => None,
    };
    let folder = berth_core::fmt::tilde(&plan.root, &berth_sys::paths::home());
    let title_name = match &plan.source {
        berth_core::PlanSource::Harness(hid) => harness_name(hid),
        _ if !pending.title.is_empty() => pending.title.clone(),
        _ => plan
            .root
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
    };
    let views = file_views(plan, &revealed.read());
    let tall = views
        .iter()
        .any(|v| v.lines.len() > crate::ui::diff_view::COLLAPSE_AT);
    let current = (*selected.read()).min(views.len().saturating_sub(1));
    let empty = plan.files.is_empty();

    // The command only runs where the plan knows its project.
    let mut for_labels = plan.clone();
    if pending.run.is_none() {
        for_labels.after_write = None;
    }
    let labels = footer_labels(&for_labels);

    let write_blocker: Option<String> =
        if let Some(reason) = backing.unsupported(Capability::WriteFiles) {
            Some(reason)
        } else if conflict.is_some() {
            Some("A file changed. Regenerate the plan first.".to_string())
        } else {
            match &*check.read() {
                Check::Checking => Some("Checking the compose file\u{2026}".to_string()),
                Check::Invalid(_) => Some("The compose file is not valid.".to_string()),
                Check::Valid => None,
            }
        };

    // The keys a secret file gains, under the file list (values masked until revealed).
    let keys = plan.files.iter().find_map(|f| {
        if !is_secret_file(&f.path) || f.removed > 0 {
            return None;
        }
        let shown = revealed.read().contains(&f.path.display().to_string());
        let lines = added_key_lines(f, shown);
        (!lines.is_empty()).then_some((lines, f))
    });

    let mut write = {
        let store = store.clone();
        let services = services.clone();
        let link = link.clone();
        let actions = actions.clone();
        let pending = pending.clone();
        move |start: bool| {
            if *phase.peek() == Phase::Writing {
                return;
            }
            phase.set(Phase::Writing);
            let pending = pending.clone();
            let store = store.clone();
            let services = services.clone();
            let link = link.clone();
            let actions = actions.clone();
            spawn(async move {
                let db = services.as_ref().map(|s| std::sync::Arc::clone(&s.store));
                let outcome = off_ui(commit(pending.clone(), db)).await;
                match outcome {
                    Some(Committed::Written { count }) => {
                        // A folder pinned by this write shows up as a project now.
                        if let (Some(link), Some(_)) = (&link, &pending.pin) {
                            link.pins_changed();
                        }
                        let command = pending.plan.after_write.clone();
                        let started = match (start, command, &pending.run, &actions) {
                            (true, Some(command), Some(run), Some(actions)) => {
                                Some(actions.run_for_folder(&mut state, run, command))
                            }
                            _ => None,
                        };
                        match started {
                            Some(Ok(key)) => {
                                was_running.set(false);
                                phase.set(Phase::Starting(key));
                            }
                            Some(Err(message)) => phase.set(Phase::StartFailed(message)),
                            None => {
                                toast(
                                    &mut state,
                                    format!(
                                        "Wrote {count} {}",
                                        if count == 1 { "file" } else { "files" }
                                    ),
                                );
                                if let Some(store) = &store {
                                    store.remove(&id);
                                }
                                close(&mut state, id, Some(pending.plan.root.as_path()));
                            }
                        }
                    }
                    Some(Committed::Conflict(path)) => phase.set(Phase::Conflict(path)),
                    Some(Committed::Failed(message)) => phase.set(Phase::Failed(message)),
                    None => phase.set(Phase::Failed("The write did not finish.".to_string())),
                }
            });
        }
    };

    let can_regenerate = pending.regenerate.is_some();
    let on_regenerate = {
        let store = store.clone();
        let pending = pending.clone();
        move |_| {
            let Some(make) = pending.regenerate.clone() else {
                return;
            };
            phase.set(Phase::Regenerating);
            let store = store.clone();
            spawn(async move {
                match off_ui(regenerate(make)).await {
                    Some(Ok(plan)) => {
                        if let Some(store) = &store {
                            store.replace(&id, plan.clone());
                        }
                        plan_entry.with_mut(|entry| {
                            if let Some(entry) = entry {
                                entry.plan = plan;
                            }
                        });
                        selected.set(0);
                        revealed.set(BTreeSet::new());
                        expanded.set(BTreeSet::new());
                        phase.set(Phase::Idle);
                    }
                    Some(Err(message)) => phase.set(Phase::Failed(message)),
                    None => phase.set(Phase::Failed("The plan did not finish.".to_string())),
                }
            });
        }
    };

    let source_text = source_line(&plan.source, harness_name);
    let summary_text = summary(plan, &folder);
    let footer_note = pending.footer_note.clone();
    let check_note = match &*check.read() {
        Check::Checking => Some("Checking the compose file\u{2026}".to_string()),
        _ => None,
    };
    let invalid = match &*check.read() {
        Check::Invalid(message) => Some(message.clone()),
        _ => None,
    };
    let notes = plan.notes.clone();
    let phase_now = phase.read().clone();
    let label_primary = match &phase_now {
        Phase::Writing => "Writing\u{2026}".to_string(),
        Phase::Starting(_) => "Starting\u{2026}".to_string(),
        _ => labels.primary.clone(),
    };

    rsx! {
        style { "{REVIEW_CHANGES_CSS}" }
        style { "{DIFF_VIEW_CSS}" }
        div {
            class: if tall { "rc-sheet rc-sheet-tall" } else { "rc-sheet" },
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Review changes",
            onclick: move |e| e.stop_propagation(),
            h2 { class: "rc-title", "Review changes \u{b7} {title_name}" }
            p { class: "rc-summary", "{summary_text}" }

            if empty {
                div { class: "rc-empty",
                    p { "There is nothing to write." }
                    for (index, note) in notes.iter().enumerate() {
                        p { key: "{index}", "{note}" }
                    }
                }
            } else {
                div { class: "rc-body",
                    div { class: if keys.is_some() { "rc-left rc-left-wide" } else { "rc-left" },
                        DiffFileList {
                            files: views.clone(),
                            selected: current,
                            on_select: move |index: usize| {
                                selected.set(index);
                                spawn(async move {
                                    let script = format!(
                                        "document.getElementById('dv-file-{index}')?.scrollIntoView({{block:'nearest'}});"
                                    );
                                    let _ = document::eval(&script).await;
                                });
                            },
                        }
                        if let Some((lines, file)) = keys.as_ref().filter(|(lines, _)| !lines.is_empty()) {
                            ul { class: "rc-keys", "aria-label": "Keys added to {file.path.display()}",
                                for (n, line) in lines.iter().enumerate() {
                                    li { key: "{n}", "+ {line}" }
                                }
                            }
                            p { class: "rc-caption",
                                {keys_caption(lines.len(), file.old_sha256.is_some())}
                            }
                        }
                        p { class: "rc-caption", "{source_text}" }
                        if !notes.is_empty() {
                            ul { class: "rc-notes",
                                for (index, note) in notes.iter().enumerate() {
                                    li { key: "{index}", class: "rc-caption", "{note}" }
                                }
                            }
                        }
                    }
                    div { class: "rc-right",
                        for (index, view) in views.iter().cloned().enumerate() {
                            {
                                let path = view.path.clone();
                                let reveal_path = path.clone();
                                let is_expanded = expanded.read().contains(&path);
                                rsx! {
                                    DiffCard {
                                        key: "{path}",
                                        file: view,
                                        index,
                                        expanded: is_expanded,
                                        on_expand: move |()| {
                                            expanded.write().insert(path.clone());
                                        },
                                        on_toggle_reveal: move |()| {
                                            let mut set = revealed.write();
                                            if !set.remove(&reveal_path) {
                                                set.insert(reveal_path.clone());
                                            }
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if let Some(message) = invalid {
                div { class: "rc-state",
                    InlineMessage {
                        message: format!("The compose file is not valid. {message}"),
                        tone: InlineTone::Danger,
                    }
                }
            }
            if let Some(path) = conflict.clone() {
                div { class: "rc-state",
                    InlineMessage {
                        message: format!(
                            "{} changed since this plan was made. Nothing was written.",
                            path.display()
                        ),
                        tone: InlineTone::Warning,
                    }
                    if can_regenerate {
                        div { class: "rc-state-actions",
                            Button {
                                label: "Regenerate".to_string(),
                                variant: ButtonVariant::Secondary,
                                size: ControlSize::Sm,
                                onclick: on_regenerate,
                            }
                            span { class: "rc-caption",
                                "Builds the plan again from the files as they are now."
                            }
                        }
                    }
                }
            }
            if let Phase::Failed(message) = &phase_now {
                div { class: "rc-state",
                    InlineMessage {
                        message: format!("Could not write the files. {message} Nothing was changed."),
                        tone: InlineTone::Danger,
                    }
                }
            }
            if let Phase::StartFailed(message) = &phase_now {
                div { class: "rc-state",
                    InlineMessage {
                        message: format!("The files are written, but it did not start. {message}"),
                        tone: InlineTone::Danger,
                    }
                }
            }

            div { class: "rc-footer",
                if let Some(note) = footer_note.or(check_note) {
                    p { class: "rc-footer-note", "{note}" }
                } else {
                    span { class: "rc-footer-note" }
                }
                Button {
                    label: if empty || matches!(phase_now, Phase::StartFailed(_)) { "Close".to_string() } else { "Cancel".to_string() },
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    disabled: matches!(phase_now, Phase::Writing | Phase::Regenerating),
                    onclick: move |_| close(&mut state, id, None),
                }
                if !empty && !matches!(phase_now, Phase::StartFailed(_)) {
                    if labels.runs {
                        Button {
                            label: labels.write_only.clone(),
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Md,
                            disabled: busy || write_blocker.is_some(),
                            disabled_reason: write_blocker.clone(),
                            onclick: {
                                let mut write = write.clone();
                                move |_| write(false)
                            },
                        }
                    }
                    Button {
                        label: label_primary,
                        variant: ButtonVariant::Primary,
                        size: ControlSize::Md,
                        loading: matches!(phase_now, Phase::Writing | Phase::Starting(_)),
                        disabled: busy || write_blocker.is_some(),
                        disabled_reason: write_blocker.clone(),
                        onclick: move |_| write(labels.runs),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
