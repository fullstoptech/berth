use dioxus::prelude::*;

use super::status_dot::StatusDot;
use super::tone_class;

/// State as text in detail headers and filters. Not interactive:
/// subtle background + dot + the `StatusKind::label` text.
#[component]
pub fn StatusBadge(status: berth_core::StatusKind) -> Element {
    let class = format!("status-badge {}", tone_class(status.tone()));
    rsx! {
        span { class,
            StatusDot { status }
            span { {status.label()} }
        }
    }
}
