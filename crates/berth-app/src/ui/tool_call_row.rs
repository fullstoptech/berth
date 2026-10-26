//! One tool call in an assistant turn (frame "Project / assistant /
//! 1440"): icon, the plain-English summary, its risk badge and how long
//! it took. The frame's chevron promises detail Berth does not show yet,
//! so it is drawn as decoration, not a button.

use dioxus::prelude::*;

use berth_core::{Risk, ToolCall, ToolStatus};

use super::icon::{Icon, IconSize};
use crate::services::assistant::{canonical_tool, tool_summary};

/// The glyph a tool's row carries.
#[must_use]
pub fn tool_icon(tool: &str) -> &'static str {
    match canonical_tool(tool).as_str() {
        "containers.logs" => "logs",
        "containers.inspect" | "containers.list" => "box",
        "containers.stats" => "activity",
        "files.read" | "files.propose_edit" | "compose.config" => "file-text",
        "problems.list" => "alert-triangle",
        "images.list" | "images.pull" | "images.build" | "images.remove" => "layers",
        "volumes.list" | "volumes.remove" => "hard-drive",
        "networks.list" => "network",
        "engines.list" => "server",
        "projects.list" => "folder",
        "activity.list" => "clock",
        _ => "sparkles",
    }
}

/// The frame's risk token: "Read", "Change" or "Destructive".
#[must_use]
pub fn risk_label(risk: Risk) -> &'static str {
    match risk {
        Risk::Read => "Read",
        Risk::Change => "Change",
        Risk::Destructive => "Destructive",
    }
}

fn risk_class(risk: Risk) -> &'static str {
    match risk {
        Risk::Read => "assistant-risk risk-read",
        Risk::Change => "assistant-risk risk-change",
        Risk::Destructive => "assistant-risk risk-destructive",
    }
}

/// The one risk badge both the tool rows and the approval card draw.
#[component]
pub fn RiskBadge(risk: Risk) -> Element {
    rsx! {
        span { class: risk_class(risk), {risk_label(risk)} }
    }
}

#[component]
pub fn ToolCallRow(call: ToolCall, status: ToolStatus) -> Element {
    let summary = match &status {
        ToolStatus::Done { summary, .. } if !summary.is_empty() => summary.clone(),
        ToolStatus::Failed { message } => message.clone(),
        _ => tool_summary(&call.tool, &call.args),
    };
    let state_class = match &status {
        ToolStatus::Running => "tool-row tool-running",
        ToolStatus::WaitingForApproval => "tool-row tool-waiting",
        ToolStatus::Denied => "tool-row tool-denied",
        ToolStatus::Failed { .. } => "tool-row tool-failed",
        ToolStatus::Done { .. } => "tool-row",
    };
    rsx! {
        div { class: state_class,
            span { class: "tool-row-icon",
                if matches!(status, ToolStatus::Running) {
                    span { class: "assistant-spin",
                        Icon { name: "loader", size: IconSize::Sm }
                    }
                } else {
                    Icon { name: tool_icon(&call.tool), size: IconSize::Sm }
                }
            }
            span { class: "tool-row-summary", "{summary}" }
            RiskBadge { risk: call.risk }
            match &status {
                ToolStatus::Done { ms, .. } => rsx! {
                    span { class: "tool-row-ms", "{ms} ms" }
                },
                ToolStatus::Running => rsx! {
                    span { class: "tool-row-ms", "running\u{2026}" }
                },
                ToolStatus::WaitingForApproval => rsx! {
                    span { class: "tool-row-ms", "waiting for approval" }
                },
                ToolStatus::Denied => rsx! {
                    span { class: "tool-row-ms", "denied" }
                },
                ToolStatus::Failed { .. } => rsx! {
                    span { class: "tool-row-ms", "failed" }
                },
            }
            span { class: "tool-row-chevron", "aria-hidden": "true",
                Icon { name: "chevron-right", size: IconSize::Sm }
            }
        }
    }
}
