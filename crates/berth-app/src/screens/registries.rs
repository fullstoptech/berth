//! The Registries screen (P8-T2; no frame, built from the Templates
//! header and table). Rows are the stored `Registry`s; passwords are
//! never here — only the login state. Log in lives in
//! `sheets/registry_login.rs`.

use dioxus::prelude::*;

use berth_core::{Registry, RegistryStatus};

use crate::services::registries::RegistriesService;
use crate::state::{AppState, Overlay, Toast, ToastTone};
use crate::ui::{Badge, BadgeKind, Button, ButtonVariant, ControlSize, EmptyState};

/// The screen's CSS (with the log-in sheet).
pub const REGISTRIES_CSS: &str = include_str!("../../assets/css/registries.css");

/// One row of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryRow {
    pub id: String,
    pub name: String,
    pub url: String,
    pub username: String,
    pub status: String,
    pub logged_in: bool,
}

/// What a row shows for its login state.
#[must_use]
pub fn status_text(status: &RegistryStatus) -> String {
    match status {
        RegistryStatus::LoggedIn => "Logged in".to_string(),
        RegistryStatus::LoggedOut => "Logged out".to_string(),
        RegistryStatus::Error { reason } => reason.clone(),
    }
}

/// The rows to draw, in stored order.
#[must_use]
pub fn rows(registries: &[Registry]) -> Vec<RegistryRow> {
    registries
        .iter()
        .map(|r| RegistryRow {
            id: r.id.clone(),
            name: r.name.clone(),
            url: r.url.to_string().trim_end_matches('/').to_string(),
            username: r.username.clone().unwrap_or_else(|| "\u{2014}".to_string()),
            status: status_text(&r.status),
            logged_in: r.status == RegistryStatus::LoggedIn,
        })
        .collect()
}

fn toast_error(state: &mut AppState, reason: String) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text: reason,
        tone: ToastTone::Danger,
        action: None,
    });
}

#[component]
pub fn RegistriesScreen() -> Element {
    let mut state = use_context::<AppState>();
    let service = try_consume_context::<Option<RegistriesService>>().flatten();
    let shown = rows(&state.registries.read());
    let total = shown.len();

    rsx! {
        style { "{REGISTRIES_CSS}" }
        section { class: "registries-screen", "aria-label": "Registries",
            div { class: "rg-header",
                h1 { class: "rg-title", "Registries" }
                Badge { kind: BadgeKind::Count, text: "{total}" }
                span { class: "rg-spacer" }
                Button {
                    label: "Add registry\u{2026}".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Sm,
                    icon: "plus",
                    disabled: service.is_none(),
                    disabled_reason: Some("Registries are not connected".to_string()),
                    onclick: move |_| {
                        state.overlays.write().push(Overlay::AddRegistry(None));
                    },
                }
            }
            p { class: "rg-lede",
                "Logins for pulling and pushing private images. Passwords live in your Keychain, never in Berth's database."
            }
            if shown.is_empty() {
                div { class: "rg-empty",
                    EmptyState {
                        title: "No registries yet".to_string(),
                        body: "Add Docker Hub or a private registry to pull private images. Public images need no login.".to_string(),
                        icon: "cloud",
                        action_label: Some("Add registry\u{2026}".to_string()),
                        onaction: move |()| {
                            state.overlays.write().push(Overlay::AddRegistry(None));
                        },
                    }
                }
            } else {
                div { class: "rg-list", role: "table", "aria-label": "Registries",
                    div { class: "rg-head", role: "row",
                        span { role: "columnheader", "Name" }
                        span { role: "columnheader", "URL" }
                        span { role: "columnheader", "Username" }
                        span { role: "columnheader", "Status" }
                        span {}
                    }
                    for row in shown {
                        div { key: "{row.id}", class: "rg-row", role: "row",
                            span { "{row.name}" }
                            span { class: "rg-url", "{row.url}" }
                            span { "{row.username}" }
                            span { "{row.status}" }
                            span { class: "rg-actions",
                                if row.logged_in {
                                    Button {
                                        label: "Log out".to_string(),
                                        variant: ButtonVariant::Ghost,
                                        size: ControlSize::Sm,
                                        onclick: {
                                            let service = service.clone();
                                            let id = row.id.clone();
                                            move |_| {
                                                if let Some(service) = &service {
                                                    if let Err(reason) = service.log_out(&mut state, &id) {
                                                        toast_error(&mut state, reason);
                                                    }
                                                }
                                            }
                                        },
                                    }
                                } else {
                                    Button {
                                        label: "Log in\u{2026}".to_string(),
                                        variant: ButtonVariant::Ghost,
                                        size: ControlSize::Sm,
                                        onclick: {
                                            let id = row.id.clone();
                                            move |_| {
                                                state
                                                    .overlays
                                                    .write()
                                                    .push(Overlay::AddRegistry(Some(id.clone())));
                                            }
                                        },
                                    }
                                }
                                Button {
                                    label: "Remove".to_string(),
                                    variant: ButtonVariant::Ghost,
                                    size: ControlSize::Sm,
                                    onclick: {
                                        let service = service.clone();
                                        let id = row.id.clone();
                                        move |_| {
                                            if let Some(service) = &service {
                                                if let Err(reason) = service.remove(&mut state, &id) {
                                                    toast_error(&mut state, reason);
                                                }
                                            }
                                        }
                                    },
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
#[path = "registries_tests.rs"]
mod tests;
