use dioxus::prelude::*;

use super::icon::{Icon, IconSize};
use super::tone_class;

/// Shape of a status dot: never colour alone (components-spec), so a
/// not-yet-created thing is a hollow ring and a transitioning one a ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DotShape {
    Solid,
    Ring,
    Hollow,
}

/// What a sidebar or title-bar dot looks like and says. The accessible
/// name is the word, so the state survives greyscale and screen readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DotSpec {
    pub tone: berth_core::Tone,
    pub shape: DotShape,
    pub label: &'static str,
}

impl DotSpec {
    pub fn class(&self) -> String {
        let shape = match self.shape {
            DotShape::Solid => "",
            DotShape::Ring => " status-dot-ring",
            DotShape::Hollow => " status-dot-hollow",
        };
        format!("status-dot {}{shape}", tone_class(self.tone))
    }
}

/// The dot for a project row (frame: shop-api red, analytics amber,
/// berth-site green, ml-notebooks grey, docker-manger a hollow ring).
pub fn project_dot(status: &berth_core::ProjectStatus) -> DotSpec {
    use berth_core::{ProjectStatus as P, Tone};
    let (tone, shape, label) = match status {
        P::AllRunning => (Tone::Success, DotShape::Solid, "Running"),
        P::Partial { .. } => (Tone::Warning, DotShape::Solid, "Partly running"),
        P::Failing => (Tone::Danger, DotShape::Solid, "Failing"),
        P::Stopped => (Tone::Stopped, DotShape::Solid, "Stopped"),
        P::NotCreated => (Tone::Stopped, DotShape::Hollow, "Not created"),
    };
    DotSpec { tone, shape, label }
}

#[component]
pub fn Dot(spec: DotSpec) -> Element {
    rsx! {
        span { class: spec.class(), role: "img", "aria-label": spec.label }
    }
}

/// A navigation destination (sections and projects), not an action:
/// icon, label, optional count, optional status dot, indent level.
/// `selected` marks the current destination (`aria-current`).
#[component]
pub fn SidebarItem(
    label: String,
    #[props(default)] icon: Option<&'static str>,
    #[props(default)] count: Option<usize>,
    #[props(default)] dot: Option<DotSpec>,
    #[props(default)] indent: bool,
    #[props(default)] selected: bool,
    #[props(default)] onclick: Option<EventHandler<MouseEvent>>,
) -> Element {
    let mut class = "sidebar-item".to_string();
    if indent {
        class.push_str(" sidebar-item-indent");
    }
    if selected {
        class.push_str(" sidebar-item-selected");
    }
    rsx! {
        button {
            class,
            r#type: "button",
            "aria-current": if selected { "page" },
            onclick: move |e| {
                if let Some(handler) = &onclick {
                    handler.call(e);
                }
            },
            if let Some(name) = icon {
                Icon { name, size: IconSize::Md }
            }
            if let Some(spec) = dot {
                Dot { spec }
            }
            span { class: "sidebar-label", "{label}" }
            if let Some(n) = count {
                span { class: "sidebar-count", "{n}" }
            }
        }
    }
}
