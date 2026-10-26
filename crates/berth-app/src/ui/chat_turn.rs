//! One conversation turn (frame "Project / assistant / 1440"): the
//! origin line and the user's right-aligned bubble, or the assistant's
//! tool rows and prose. A proposed change draws as its card in the
//! screen's right column, not inline.

use dioxus::prelude::*;

use berth_core::{Turn, TurnPart, TurnRole};

use super::tool_call_row::ToolCallRow;

#[component]
pub fn ChatTurn(turn: Turn) -> Element {
    rsx! {
        div { class: "chat-turn",
            if let Some(origin) = &turn.origin {
                p { class: "chat-origin", "{origin}" }
            }
            match turn.role {
                TurnRole::User => rsx! {
                    div { class: "chat-user-row",
                        div { class: "chat-bubble",
                            for part in &turn.parts {
                                if let TurnPart::Text(text) = part {
                                    span { "{text}" }
                                }
                            }
                        }
                    }
                },
                TurnRole::Assistant => rsx! {
                    div { class: "chat-assistant",
                        for (index, part) in turn.parts.iter().enumerate() {
                            match part {
                                TurnPart::Text(text) => rsx! {
                                    p { key: "{index}", class: "chat-prose", "{text}" }
                                },
                                TurnPart::Tool { call, status } => rsx! {
                                    ToolCallRow {
                                        key: "{call.call_id}",
                                        call: call.clone(),
                                        status: status.clone(),
                                    }
                                },
                                TurnPart::ProposedChange { .. } => rsx! {},
                            }
                        }
                    }
                },
            }
        }
    }
}
