use dioxus::prelude::*;

use super::tone_class;

/// State inside dense rows; never colour alone — Starting/Restarting show a
/// 2-segment ring and Paused a hollow dot (components-spec), so the state
/// survives greyscale. Tone and label come from `StatusKind::tone`/`label`.
#[component]
pub fn StatusDot(status: berth_core::StatusKind) -> Element {
    let shape = match status {
        berth_core::StatusKind::Starting | berth_core::StatusKind::Restarting => " status-dot-ring",
        berth_core::StatusKind::Paused => " status-dot-hollow",
        _ => "",
    };
    let class = format!("status-dot {}{shape}", tone_class(status.tone()));
    rsx! {
        span { class, role: "img", "aria-label": status.label() }
    }
}
