use dioxus::prelude::*;

use super::tone_class;

/// The two arcs of the Components set's starting/restarting dot: a ring of
/// 8 units with two 150-degree strokes 180 degrees apart ("arc a" starts at
/// 0, "arc b" at 180; radius 3, stroke 2). Drawn in `--dot-color`.
#[component]
pub fn RingArcs() -> Element {
    rsx! {
        svg {
            class: "status-dot-arcs",
            view_box: "0 0 8 8",
            "aria-hidden": "true",
            path { d: "M7 4 A3 3 0 0 1 1.4019 5.5" }
            path { d: "M1 4 A3 3 0 0 1 6.5981 2.5" }
        }
    }
}

/// State inside dense rows; never colour alone — Starting/Restarting show a
/// two-arc ring and Paused a hollow dot (components-spec), so the state
/// survives greyscale. Tone and label come from `StatusKind::tone`/`label`.
#[component]
pub fn StatusDot(status: berth_core::StatusKind) -> Element {
    let ring = matches!(
        status,
        berth_core::StatusKind::Starting | berth_core::StatusKind::Restarting
    );
    let shape = if ring {
        " status-dot-ring"
    } else if status == berth_core::StatusKind::Paused {
        " status-dot-hollow"
    } else {
        ""
    };
    let class = format!("status-dot {}{shape}", tone_class(status.tone()));
    rsx! {
        span { class, role: "img", "aria-label": status.label(),
            if ring {
                RingArcs {}
            }
        }
    }
}
