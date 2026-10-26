//! One gated tool call waiting for a click (frame "Project /
//! assistant / 1440"): risk badge and what runs, the command in mono,
//! the explanation lines, Approve and Deny. A Destructive call adds
//! the typed-name field (uncontrolled, D59); Approve stays disabled
//! while an earlier step is unapplied or the typed name differs —
//! and the service refuses both cases again, so the card can never
//! be the only gate.

use dioxus::prelude::*;

use super::button::{Button, ButtonVariant};
use super::text_field::TextField;
use super::tool_call_row::RiskBadge;
use super::ControlSize;
use crate::services::assistant::command_line;
use crate::state::Approval;

#[component]
pub fn ApprovalCard(
    approval: Approval,
    /// Why Approve waits (an unapplied earlier step), if it does.
    #[props(default)]
    blocked_reason: Option<String>,
    /// Called with what was typed (Destructive) or `None` (Change).
    #[props(default)]
    on_approve: Option<EventHandler<Option<String>>>,
    #[props(default)] on_deny: Option<EventHandler<()>>,
) -> Element {
    let mut typed = use_signal(String::new);
    let summary = approval
        .effect
        .lines
        .first()
        .map(|line| line.text.clone())
        .unwrap_or_default();
    let heading = if approval.after_plan.is_some() {
        format!("Then: {summary}")
    } else {
        summary
    };
    let typed_ok = approval
        .typed_name
        .as_ref()
        .is_none_or(|expected| typed.read().as_str() == expected.as_str());
    let blocked = blocked_reason.clone().or_else(|| {
        (!typed_ok).then(|| {
            format!(
                "Type {} to confirm",
                approval.typed_name.clone().unwrap_or_default()
            )
        })
    });
    let approve_disabled = blocked.is_some();
    rsx! {
        section { class: "approval-card", "aria-label": "Approval needed: {heading}",
            div { class: "approval-head",
                RiskBadge { risk: approval.call.risk }
                span { class: "approval-title", "{heading}" }
            }
            pre { class: "approval-command", {command_line(&approval.call)} }
            for line in approval.effect.lines.iter().skip(1) {
                p { key: "{line.text}", class: "approval-line", "{line.text}" }
            }
            if let Some(expected) = &approval.typed_name {
                TextField {
                    label: format!("Type {expected} to confirm"),
                    id: format!("approval-typed-{}", approval.id),
                    placeholder: expected.clone(),
                    aria_label: format!("Type {expected} to confirm"),
                    oninput: move |e: FormEvent| typed.set(e.value()),
                }
            }
            div { class: "approval-actions",
                Button {
                    label: "Approve",
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Sm,
                    disabled: approve_disabled,
                    disabled_reason: blocked,
                    onclick: move |_| {
                        if let Some(handler) = &on_approve {
                            let value = typed.peek().clone();
                            handler.call((!value.is_empty()).then_some(value));
                        }
                    },
                }
                Button {
                    label: "Deny",
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Sm,
                    onclick: move |_| {
                        if let Some(handler) = &on_deny {
                            handler.call(());
                        }
                    },
                }
            }
        }
    }
}
