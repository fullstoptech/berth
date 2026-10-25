//! ProblemItem (components-spec): severity, title, subject (in the Problems tab), age, explanation,
//! evidence lines (mono), advice and fix buttons. The first fix is the primary action. States:
//! default, expanded, fix running (a loading button) and fix failed (the engine's message under
//! the button). Controlled: the screen builds the view from a `berth_core::Problem` and runs what
//! a fix click means.

use dioxus::prelude::*;

use super::button::{Button, ButtonVariant};
use super::icon::{Icon, IconSize};
use super::inline_message::{InlineMessage, InlineTone};
use super::ControlSize;

/// The item's CSS (tokens only). The screens that show items render it with them.
pub const PROBLEM_ITEM_CSS: &str = include_str!("../../assets/css/problem_item.css");

#[derive(Debug, Clone, PartialEq)]
pub struct FixButton {
    pub id: String,
    pub label: String,
    pub primary: bool,
    pub icon: Option<&'static str>,
    /// A split button: the label plus an attached ▾ for choosing the tool.
    pub split: bool,
    pub disabled_reason: Option<String>,
    pub loading: bool,
    /// The fix ran and the engine said no: its message shows under the button.
    pub failed: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProblemView {
    /// The problem's id: the item's key, so an open item stays open while the list changes.
    pub id: String,
    pub severity: berth_core::Severity,
    pub title: String,
    /// What the problem is about ("shop-api-db-1"); the Problems tab shows it, the inspector's
    /// card (which is already about one container) does not.
    pub subject: Option<String>,
    /// "2 min", already formatted.
    pub age: String,
    pub explanation: String,
    pub evidence: Vec<String>,
    /// Advice with no button ("Keep both: choose another name in the Compose file").
    pub notes: Vec<String>,
    pub fixes: Vec<FixButton>,
}

fn severity_class(severity: berth_core::Severity) -> &'static str {
    match severity {
        berth_core::Severity::Error => "tone-danger",
        berth_core::Severity::Warning => "tone-warning",
        berth_core::Severity::Info => "tone-info",
    }
}

fn severity_icon(severity: berth_core::Severity) -> &'static str {
    match severity {
        berth_core::Severity::Error => "alert-circle",
        berth_core::Severity::Warning => "alert-triangle",
        berth_core::Severity::Info => "info",
    }
}

#[component]
pub fn ProblemItem(
    problem: ProblemView,
    onfix: EventHandler<String>,
    /// Whether the item starts open; the inspector's card does, a list of many may not.
    #[props(default = true)]
    open: bool,
) -> Element {
    let mut expanded = use_signal(move || open);
    let open = *expanded.read();
    let tone = severity_class(problem.severity);
    let chevron = if open {
        "chevron-down"
    } else {
        "chevron-right"
    };
    rsx! {
        section { class: "pi {tone}", "aria-label": "Problem: {problem.title}",
            button {
                class: "pi-head",
                r#type: "button",
                "aria-expanded": open.to_string(),
                onclick: move |_| {
                    let now = *expanded.peek();
                    expanded.set(!now);
                },
                span { class: "pi-icon",
                    Icon { name: severity_icon(problem.severity), size: IconSize::Sm }
                }
                span { class: "pi-title", "{problem.title}" }
                if let Some(subject) = &problem.subject {
                    span { class: "pi-subject", title: "{subject}", "{subject}" }
                }
                span { class: "pi-age", "{problem.age}" }
                Icon { name: chevron, size: IconSize::Sm }
            }
            if open {
                if !problem.explanation.is_empty() {
                    p { class: "pi-explanation", "{problem.explanation}" }
                }
                if !problem.evidence.is_empty() {
                    pre { class: "pi-evidence",
                        for line in &problem.evidence {
                            "{line}\n"
                        }
                    }
                }
                if !problem.notes.is_empty() {
                    ul { class: "pi-notes",
                        for note in &problem.notes {
                            li { key: "{note}", "{note}" }
                        }
                    }
                }
                div { class: "pi-fixes",
                    for fix in problem.fixes.clone() {
                        {
                            let id = fix.id.clone();
                            let variant = if fix.primary { ButtonVariant::Primary } else { ButtonVariant::Secondary };
                            let disabled = fix.disabled_reason.is_some();
                            rsx! {
                                div { key: "{fix.id}", class: "pi-fail",
                                  div { class: if fix.split { "pi-fix pi-fix-split" } else { "pi-fix" },
                                    Button {
                                        label: fix.label.clone(),
                                        variant,
                                        size: ControlSize::Md,
                                        icon: fix.icon,
                                        disabled,
                                        disabled_reason: fix.disabled_reason.clone(),
                                        loading: fix.loading,
                                        onclick: move |_| onfix.call(id.clone()),
                                    }
                                    if fix.split {
                                        button {
                                            class: "pi-split",
                                            r#type: "button",
                                            "aria-label": "Choose a tool",
                                            disabled,
                                            "data-tooltip": fix.disabled_reason.clone(),
                                            Icon { name: "chevron-down", size: IconSize::Sm }
                                        }
                                    }
                                  }
                                  if let Some(message) = &fix.failed {
                                      InlineMessage { message: message.clone(), tone: InlineTone::Danger }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ProblemView {
        ProblemView {
            id: "p1".to_string(),
            severity: berth_core::Severity::Error,
            title: "worker keeps restarting".to_string(),
            subject: None,
            age: "2 min".to_string(),
            explanation: "The worker exits with code 1.".to_string(),
            evidence: vec!["Error: connect ECONNREFUSED".to_string()],
            notes: Vec::new(),
            fixes: vec![
                FixButton {
                    id: "stop".to_string(),
                    label: "Stop the restart loop".to_string(),
                    primary: true,
                    icon: None,
                    split: false,
                    disabled_reason: None,
                    loading: false,
                    failed: None,
                },
                FixButton {
                    id: "ai".to_string(),
                    label: "Fix with Claude Code".to_string(),
                    primary: false,
                    icon: Some("sparkles"),
                    split: true,
                    disabled_reason: Some("Needs a harness".to_string()),
                    loading: false,
                    failed: None,
                },
            ],
        }
    }

    #[test]
    fn problem_item_shows_title_age_explanation_evidence_and_fixes() {
        #[component]
        fn Host(problem: ProblemView) -> Element {
            rsx! { ProblemItem { problem, onfix: |_| {} } }
        }
        let mut dom = VirtualDom::new_with_props(Host, HostProps { problem: view() });
        let _ = dom.rebuild_to_vec();
        let html = dioxus_ssr::render(&dom);
        for needle in [
            "worker keeps restarting",
            "2 min",
            "exits with code 1",
            "ECONNREFUSED",
            "Stop the restart loop",
            "btn-primary",
            "Fix with Claude Code",
            "pi-split",
            "data-tooltip=\"Needs a harness\"",
        ] {
            assert!(html.contains(needle), "missing {needle}: {html}");
        }
    }
}
