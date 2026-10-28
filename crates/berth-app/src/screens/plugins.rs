//! The Plugins screen (P9-T2, AD48; no frame, built from the Registries
//! header and a card per plugin). Shows what is installed, what each plugin
//! may do, whether it runs, and the last output of one that failed. Enable
//! opens the permission request sheet and Configure the settings form
//! (`sheets/plugin_enable.rs`, `sheets/plugin_configure.rs`); the screen
//! reads [`PluginsView`] and asks [`PluginsService`] for every change.

use dioxus::prelude::*;

use berth_plugin_host::PluginState;

use crate::services::plugins::{permission_sentence, PluginEntry, PluginsService, PluginsView};
use crate::state::{AppState, Overlay, Toast, ToastTone};
use crate::ui::{Badge, BadgeKind, Button, ButtonVariant, ControlSize, EmptyState};

/// The screen's CSS (with the two sheets).
pub const PLUGINS_CSS: &str = include_str!("../../assets/css/plugins.css");

/// How a plugin's state reads: label, tone class and whether the dot is hollow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateLook {
    pub label: &'static str,
    pub tone: &'static str,
    pub hollow: bool,
}

#[must_use]
pub fn state_look(state: &PluginState) -> StateLook {
    match state {
        PluginState::Enabled => StateLook {
            label: "Enabled",
            tone: "tone-success",
            hollow: false,
        },
        PluginState::Disabled => StateLook {
            label: "Disabled",
            tone: "tone-stopped",
            hollow: true,
        },
        PluginState::Error { .. } => StateLook {
            label: "Error",
            tone: "tone-danger",
            hollow: false,
        },
    }
}

/// The permissions a card lists: what the plugin may do now when enabled,
/// else what it asks for.
#[must_use]
pub fn permission_lines(entry: &PluginEntry) -> Vec<String> {
    entry
        .plugin
        .manifest
        .permissions
        .iter()
        .map(permission_sentence)
        .collect()
}

/// The line under a card that says what is still to do: settings missing, or
/// a restart count while the host keeps bringing the plugin back.
#[must_use]
pub fn attention_line(entry: &PluginEntry) -> Option<String> {
    if let PluginState::Error { .. } = entry.plugin.state {
        return None;
    }
    let missing = entry.missing_required();
    if !missing.is_empty() {
        return Some(format!("Needs setup: {}", missing.join(", ")));
    }
    (entry.restarts > 0).then(|| {
        let times = if entry.restarts == 1 {
            "once".to_string()
        } else {
            format!("{} times", entry.restarts)
        };
        format!("Restarted {times} after it stopped")
    })
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
pub fn PluginsScreen() -> Element {
    let view = use_context::<PluginsView>();
    let entries = view.entries.read().clone();
    let total = entries.len();

    rsx! {
        style { "{PLUGINS_CSS}" }
        section { class: "plugins-screen", "aria-label": "Plugins",
            div { class: "pl-header",
                h1 { class: "pl-title", "Plugins" }
                Badge { kind: BadgeKind::Count, text: "{total}" }
            }
            p { class: "pl-lede",
                "Plugins run as separate programs and add screens to Berth. Each one lists what it may do before you enable it, and its secrets stay in your Keychain."
            }
            if entries.is_empty() {
                div { class: "pl-empty",
                    EmptyState {
                        title: "No plugins installed".to_string(),
                        body: "Berth looks for plugins next to the app and in its Plugins folder. Coolify ships with Berth.".to_string(),
                        icon: "zap",
                    }
                }
            } else {
                div { class: "pl-list",
                    for entry in entries {
                        PluginCard { key: "{entry.id()}", entry }
                    }
                }
            }
        }
    }
}

#[component]
fn PluginCard(entry: PluginEntry) -> Element {
    let mut state = use_context::<AppState>();
    let service = try_consume_context::<Option<PluginsService>>().flatten();
    let look = state_look(&entry.plugin.state);
    let enabled = entry.plugin.state != PluginState::Disabled;
    let id = entry.id().to_string();
    let name = entry.plugin.manifest.name.clone();
    let version = entry.plugin.manifest.version.clone();
    let permissions = permission_lines(&entry);
    let attention = attention_line(&entry);
    let configurable =
        !entry.plugin.manifest.config.is_empty() || !entry.plugin.manifest.secrets.is_empty();
    let failure = match &entry.plugin.state {
        PluginState::Error { reason } => Some(reason.clone()),
        _ => None,
    };
    let crash_log = entry.crash_log.join("\n");
    let dot_class = if look.hollow {
        "status-dot status-dot-hollow"
    } else {
        "status-dot"
    };
    let aria = format!("{name} plugin");

    let on_toggle = {
        let service = service.clone();
        let id = id.clone();
        move |_| {
            if enabled {
                let Some(service) = service.clone() else {
                    toast_error(&mut state, "Plugins are not connected".to_string());
                    return;
                };
                let id = id.clone();
                spawn(async move {
                    if let Err(reason) = service.disable(&id).await {
                        toast_error(&mut state, reason);
                    }
                });
            } else {
                state
                    .overlays
                    .write()
                    .push(Overlay::PluginEnable(id.clone()));
            }
        }
    };
    let on_retry = {
        let service = service.clone();
        let id = id.clone();
        move |_| {
            let Some(service) = service.clone() else {
                toast_error(&mut state, "Plugins are not connected".to_string());
                return;
            };
            let id = id.clone();
            spawn(async move {
                if let Err(reason) = service.restart(&id).await {
                    toast_error(&mut state, reason);
                }
            });
        }
    };

    rsx! {
        article { class: "pl-card", role: "group", "aria-label": aria,
            div { class: "pl-card-head",
                h2 { class: "pl-name", "{name}" }
                span { class: "pl-version", "v{version}" }
                span { class: "status-badge {look.tone}",
                    span { class: dot_class }
                    span { "{look.label}" }
                }
                span { class: "pl-spacer" }
                if configurable {
                    Button {
                        label: "Configure\u{2026}".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Sm,
                        onclick: {
                            let id = id.clone();
                            move |_| {
                                state.overlays.write().push(Overlay::PluginConfigure(id.clone()));
                            }
                        },
                    }
                }
                Button {
                    label: if enabled { "Disable".to_string() } else { "Enable\u{2026}".to_string() },
                    variant: if enabled { ButtonVariant::Secondary } else { ButtonVariant::Primary },
                    size: ControlSize::Sm,
                    onclick: on_toggle,
                }
            }
            div { class: "pl-section",
                span { class: "pl-label",
                    if enabled { "Allowed to" } else { "Asks to" }
                }
                ul { class: "pl-perms",
                    for line in permissions {
                        li { key: "{line}", "{line}" }
                    }
                    if entry.plugin.manifest.permissions.is_empty() {
                        li { "Nothing beyond showing its own screens" }
                    }
                }
            }
            if let Some(line) = attention {
                p { class: "pl-attention", "{line}" }
            }
            if let Some(reason) = failure {
                div { class: "pl-failure", role: "alert",
                    div { class: "pl-failure-head",
                        svg { class: "pl-failure-icon", "aria-hidden": "true",
                            r#use { href: "/icons.svg#alert-circle" }
                        }
                        span { class: "pl-failure-text", "{name} stopped: {reason}" }
                        span { class: "pl-spacer" }
                        Button {
                            label: "Try again".to_string(),
                            variant: ButtonVariant::Secondary,
                            size: ControlSize::Sm,
                            onclick: on_retry,
                        }
                    }
                    if !crash_log.is_empty() {
                        span { class: "pl-label", "Last output" }
                        pre { class: "pl-crash-log", role: "region", tabindex: "0", "aria-label": "Crash log", "{crash_log}" }
                    }
                }
            } else if entry.restarts > 0 && !crash_log.is_empty() {
                div { class: "pl-failure pl-failure-soft",
                    span { class: "pl-label", "Last output before the restart" }
                    pre { class: "pl-crash-log", role: "region", tabindex: "0", "aria-label": "Crash log", "{crash_log}" }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "plugins_tests.rs"]
mod tests;
