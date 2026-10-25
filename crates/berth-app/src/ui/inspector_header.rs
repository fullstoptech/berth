//! InspectorHeader (components-spec): name, status, image, id with copy
//! and the labelled buttons of one container. Controlled: the screen says
//! which buttons exist, which are disabled and why, and what a click does.

use dioxus::prelude::*;

use super::button::{Button, ButtonVariant};
use super::icon::{Icon, IconSize};
use super::icon_button::IconButton;
use super::key_value_list::copy_to_clipboard;
use super::menu::{Menu, MenuEntry};
use super::status_badge::StatusBadge;
use super::ControlSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeaderTone {
    #[default]
    Neutral,
    /// Kill: the icon alone is in the danger tone.
    DangerIcon,
    /// Delete: label, icon and border in the danger tone.
    Danger,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeaderButton {
    /// Stable id the screen matches in `onbutton` ("stop", "kill"...).
    pub id: &'static str,
    pub label: String,
    pub icon: &'static str,
    pub tone: HeaderTone,
    /// Why the button cannot be used right now; shown as its tooltip.
    pub disabled_reason: Option<String>,
    /// The action is running; the label is then the pending label.
    pub loading: bool,
}

impl HeaderButton {
    pub fn new(id: &'static str, label: impl Into<String>, icon: &'static str) -> Self {
        Self {
            id,
            label: label.into(),
            icon,
            tone: HeaderTone::Neutral,
            disabled_reason: None,
            loading: false,
        }
    }

    #[must_use]
    pub fn tone(mut self, tone: HeaderTone) -> Self {
        self.tone = tone;
        self
    }

    #[must_use]
    pub fn reason(mut self, reason: Option<String>) -> Self {
        self.disabled_reason = reason;
        self
    }

    #[must_use]
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
}

#[component]
pub fn InspectorHeader(
    name: String,
    status: berth_core::StatusKind,
    #[props(default)] note: Option<String>,
    image: String,
    id_short: String,
    id_full: String,
    buttons: Vec<HeaderButton>,
    menu: Vec<MenuEntry>,
    #[props(default)] menu_open: bool,
    /// Where the inspector sits now; with it the ⋯ menu offers "Dock side".
    #[props(default)]
    dock: Option<berth_store::InspectorDock>,
    #[props(default)] ondock: Option<EventHandler<berth_store::InspectorDock>>,
    onbutton: EventHandler<&'static str>,
    onmenutoggle: EventHandler<()>,
    onmenu: EventHandler<String>,
) -> Element {
    let copy_id = id_full.clone();
    rsx! {
        header { class: "ih",
            div { class: "ih-title",
                h2 { class: "ih-name", title: "{name}", "{name}" }
                div { class: "ih-more",
                    IconButton {
                        icon: "more-horizontal",
                        label: format!("More actions for {name}"),
                        tooltip: "More".to_string(),
                        size: ControlSize::Sm,
                        pressed: menu_open,
                        onclick: move |_| onmenutoggle.call(()),
                    }
                    if menu_open {
                        div { class: "ih-menu",
                            Menu {
                                items: menu.clone(),
                                aria_label: "More actions".to_string(),
                                on_select: move |id: String| onmenu.call(id),
                            }
                            if let Some(current) = dock {
                                DockSide { current, ondock }
                            }
                        }
                    }
                }
            }
            div { class: "ih-status",
                StatusBadge { status }
                if let Some(note) = note {
                    span { class: "ih-note", "{note}" }
                }
            }
            div { class: "ih-image", title: "{image}", "{image}" }
            div { class: "ih-id",
                span { class: "ih-id-label", "ID" }
                span { class: "ih-id-value", "{id_short}" }
                button {
                    class: "ih-id-copy",
                    r#type: "button",
                    "aria-label": "Copy container ID",
                    onclick: move |_| copy_to_clipboard(&copy_id),
                    Icon { name: "copy", size: IconSize::Sm }
                }
            }
            div { class: "ih-buttons",
                for button in buttons {
                    {
                        let id = button.id;
                        let class = match button.tone {
                            HeaderTone::Neutral => "ih-btn",
                            HeaderTone::DangerIcon => "ih-btn ih-btn-kill",
                            HeaderTone::Danger => "ih-btn ih-btn-danger",
                        };
                        let disabled = button.disabled_reason.is_some();
                        rsx! {
                            span { key: "{id}", class,
                                Button {
                                    label: button.label.clone(),
                                    variant: ButtonVariant::Secondary,
                                    size: ControlSize::Sm,
                                    icon: button.icon,
                                    disabled,
                                    disabled_reason: button.disabled_reason.clone(),
                                    loading: button.loading,
                                    onclick: move |_| onbutton.call(id),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The ⋯ menu's "Dock side" row: Right and Bottom, the current one marked
/// (D22). The glyphs are drawn here: the icon sprite is generated from the
/// design file and has no panel icons yet.
#[component]
fn DockSide(
    current: berth_store::InspectorDock,
    ondock: Option<EventHandler<berth_store::InspectorDock>>,
) -> Element {
    use berth_store::InspectorDock::{Bottom, Right};
    rsx! {
        div { class: "ih-dock", role: "group", "aria-label": "Dock side",
            span { class: "ih-dock-label", "Dock side" }
            for (side, label, divider) in [
                (Right, "Dock inspector at right", "M10 3v10"),
                (Bottom, "Dock inspector at bottom", "M2 9.5h12"),
            ] {
                button {
                    key: "{label}",
                    class: if current == side { "ih-dock-btn ih-dock-btn-on" } else { "ih-dock-btn" },
                    r#type: "button",
                    "aria-label": "{label}",
                    "aria-pressed": (current == side).to_string(),
                    "data-tooltip": label,
                    onclick: move |_| {
                        if let Some(handler) = &ondock {
                            handler.call(side);
                        }
                    },
                    svg {
                        class: "icon icon-sm",
                        view_box: "0 0 16 16",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "1.5",
                        "aria-hidden": "true",
                        rect { x: "2", y: "3", width: "12", height: "10", rx: "1.5" }
                        path { d: divider }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[component]
    fn Host(buttons: Vec<HeaderButton>) -> Element {
        rsx! {
            InspectorHeader {
                name: "shop-api-worker-1".to_string(),
                status: berth_core::StatusKind::Restarting,
                note: Some("Restarting (1) 3 seconds ago".to_string()),
                image: "shop-api-worker:latest".to_string(),
                id_short: "7b2e91c04f3a".to_string(),
                id_full: "7b2e91c04f3a1d2c".to_string(),
                buttons,
                menu: vec![MenuEntry::new("copy-id", "Copy container ID")],
                onbutton: |_| {},
                onmenutoggle: |_| {},
                onmenu: |_| {},
            }
        }
    }

    fn render(buttons: Vec<HeaderButton>) -> String {
        let mut dom = VirtualDom::new_with_props(Host, HostProps { buttons });
        let _ = dom.rebuild_to_vec();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn inspector_header_shows_name_status_image_id_and_labelled_buttons() {
        let html = render(vec![
            HeaderButton::new("stop", "Stop", "square"),
            HeaderButton::new("kill", "Kill", "zap").tone(HeaderTone::DangerIcon),
            HeaderButton::new("delete", "Delete…", "trash-2").tone(HeaderTone::Danger),
        ]);
        for needle in [
            "shop-api-worker-1",
            "Restarting",
            "3 seconds ago",
            "shop-api-worker:latest",
            "7b2e91c04f3a",
            "aria-label=\"Copy container ID\"",
            ">Stop<",
            ">Kill<",
            ">Delete…<",
            "ih-btn-kill",
            "ih-btn-danger",
        ] {
            assert!(html.contains(needle), "missing {needle}: {html}");
        }
        // The copy button copies the whole id, the header shows twelve.
        assert!(!html.contains("7b2e91c04f3a1d2c"), "{html}");
    }

    #[test]
    fn inspector_header_disabled_button_keeps_its_reason_as_tooltip() {
        let html =
            render(vec![HeaderButton::new("pause", "Pause", "pause")
                .reason(Some("Not running".to_string()))]);
        assert!(html.contains("disabled"), "{html}");
        assert!(html.contains("data-tooltip=\"Not running\""), "{html}");
    }
}
