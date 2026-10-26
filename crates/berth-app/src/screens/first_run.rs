//! First run (P6-T2, brief 6.1). Two faces of one list of checks:
//!
//! - [`FirstRunCard`] fills the slot on Projects empty (frame "Projects / empty / 1440"): the
//!   checks as a small card beside the empty state. A stopped Colima shows its Start button
//!   there; a missing Compose plugin shows its hint.
//! - [`FirstRunScreen`] is `Section::FirstRun`, full screen, and only opens when Colima or the
//!   Docker command is missing: the checks, the Homebrew command with a Copy button (Berth never
//!   installs anything), Check again and Continue.
//!
//! The rows come from `services::first_run::evaluate`; this file only draws them.

use dioxus::prelude::*;

use crate::services::actions::ActionsService;
use crate::services::first_run::{
    refresh, run_start, FirstRunStore, Report, Row, Scan, Status, HARNESS_NOTE, INSTALL_COMMAND,
};
use crate::state::{AppState, Section};
use crate::ui::{copy_to_clipboard, Button, ButtonVariant, ControlSize, Icon, IconSize};

/// The screens' CSS.
pub const FIRST_RUN_CSS: &str = include_str!("../../assets/css/first_run.css");

/// The checks for what the last scan found; `None` until it answers or when there is no store
/// (a bare component render).
fn current_report(state: &AppState) -> Option<Report> {
    let store = try_consume_context::<FirstRunStore>()?;
    store.report(&state.engines.read(), &state.harnesses.read())
}

fn status_icon(status: Status) -> (&'static str, &'static str) {
    match status {
        Status::Pass => ("circle-check", "tone-success"),
        Status::Warn => ("alert-triangle", "tone-warning"),
        Status::Fail => ("alert-circle", "tone-danger"),
        Status::Absent => ("info", "tone-stopped"),
    }
}

fn status_class(status: Status) -> &'static str {
    match status {
        Status::Pass => "fr-row fr-pass",
        Status::Warn => "fr-row fr-warn",
        Status::Fail => "fr-row fr-fail",
        Status::Absent => "fr-row fr-absent",
    }
}

fn status_word(status: Status) -> &'static str {
    match status {
        Status::Pass => "passed",
        Status::Warn => "needs attention",
        Status::Fail => "failed",
        Status::Absent => "not installed",
    }
}

#[component]
fn CheckRow(row: Row) -> Element {
    let (icon, tone) = status_icon(row.status);
    rsx! {
        div {
            class: status_class(row.status),
            role: "listitem",
            "aria-label": "{row.label}, {status_word(row.status)}, {row.value}",
            span { class: "fr-icon {tone}", Icon { name: icon, size: IconSize::Md } }
            span { class: "fr-label", "{row.label}" }
            span { class: "fr-value", "{row.value}" }
            if let Some(hint) = &row.hint {
                span { class: "fr-hint", "{hint}" }
            }
        }
    }
}

#[component]
fn CheckList(report: Report) -> Element {
    rsx! {
        div { class: "fr-rows", role: "list", "aria-label": "First-run checks",
            for row in report.rows {
                CheckRow { key: "{row.id}", row }
            }
        }
    }
}

/// Start the stopped profile (or create `default`). Shown only when the plan exists.
#[component]
fn StartButton(report: Report) -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_use_context::<ActionsService>();
    let Some(plan) = report.start else {
        return rsx! {};
    };
    let label = plan.label.clone();
    let busy = state
        .engines
        .read()
        .iter()
        .any(|e| e.id == plan.engine.id && e.state == berth_core::EngineState::Starting);
    rsx! {
        div { class: "fr-actions",
            Button {
                label,
                variant: ButtonVariant::Primary,
                size: ControlSize::Sm,
                loading: busy,
                onclick: move |_| {
                    let Some(actions) = &actions else { return };
                    if let Err(reason) = run_start(&mut state, actions, &plan) {
                        tracing::warn!(%reason, "start from first run refused");
                    }
                },
            }
        }
    }
}

/// The checks as a card on Projects empty. Until the first scan answers it says so.
#[component]
pub fn FirstRunCard() -> Element {
    let state = use_context::<AppState>();
    let report = current_report(&state);
    rsx! {
        style { "{FIRST_RUN_CSS}" }
        section { class: "fr-card", "aria-label": "First-run checks",
            h2 { class: "fr-card-title", "First-run checks" }
            match report {
                Some(report) => rsx! {
                    CheckList { report: report.clone() }
                    StartButton { report }
                    p { class: "fr-note", "{HARNESS_NOTE}" }
                },
                None => rsx! { p { class: "fr-note", "Checking this Mac\u{2026}" } },
            }
        }
    }
}

#[component]
fn InstallBlock() -> Element {
    let mut copied = use_signal(|| false);
    let copy_label = if *copied.read() { "Copied" } else { "Copy" }.to_string();
    rsx! {
        div { class: "fr-install",
            p { class: "fr-install-text",
                "Berth never installs anything itself. Paste this in Terminal, then press Check again."
            }
            div { class: "fr-command",
                code { class: "fr-command-text", "{INSTALL_COMMAND}" }
                Button {
                    label: copy_label.clone(),
                    size: ControlSize::Sm,
                    icon: Some("copy"),
                    onclick: move |_| {
                        copy_to_clipboard(INSTALL_COMMAND);
                        copied.set(true);
                    },
                }
            }
        }
    }
}

/// The full-screen setup (`Section::FirstRun`).
#[component]
pub fn FirstRunScreen() -> Element {
    let mut state = use_context::<AppState>();
    let store = try_consume_context::<FirstRunStore>();
    let mock = *state.backing.peek() == crate::services::backing::Backing::Mock;
    let report = current_report(&state);
    let scanning = store.is_some_and(|s| *s.scanning.read());
    let missing = report.as_ref().is_some_and(Report::needs_setup);
    rsx! {
        style { "{FIRST_RUN_CSS}" }
        section { class: "first-run-screen", "aria-label": "Set up Berth",
            div { class: "fr-panel",
                h1 { class: "fr-title", "Set up Berth" }
                p { class: "fr-lead",
                    "Berth manages containers on Colima through the Docker command line. Here is what it found on this Mac."
                }
                div { class: "fr-card fr-card-flat",
                    match report {
                        Some(report) => rsx! {
                            CheckList { report: report.clone() }
                            StartButton { report }
                            p { class: "fr-note", "{HARNESS_NOTE}" }
                        },
                        None => rsx! { p { class: "fr-note", "Checking this Mac\u{2026}" } },
                    }
                }
                if missing {
                    InstallBlock {}
                }
                div { class: "fr-footer",
                    Button {
                        label: "Check again".to_string(),
                        variant: if missing { ButtonVariant::Primary } else { ButtonVariant::Secondary },
                        icon: Some("rotate-cw"),
                        loading: scanning,
                        onclick: move |_| {
                            if let Some(store) = store {
                                refresh(state, store, mock, Scan::Again);
                            }
                        },
                    }
                    Button {
                        label: "Continue".to_string(),
                        variant: ButtonVariant::Secondary,
                        onclick: move |_| state.section.set(Section::Projects),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "first_run_tests.rs"]
mod tests;
