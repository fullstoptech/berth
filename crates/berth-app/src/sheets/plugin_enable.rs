//! The permission request sheet (P9-T2, AD48; brief §7.3 "permission
//! request"). Lists what the plugin asks to do and the network hosts it says
//! it contacts (shown, not enforced, D45). "Enable" is the grant: it records
//! exactly the manifest's permissions and starts the plugin.

use dioxus::prelude::*;

use crate::screens::plugins::{permission_lines, PLUGINS_CSS};
use crate::services::plugins::{PluginsService, PluginsView};
use crate::state::AppState;
use crate::ui::{Button, ButtonVariant, ControlSize, InlineMessage, InlineTone};

#[component]
pub fn PluginEnableSheet(id: String) -> Element {
    let mut state = use_context::<AppState>();
    let view = use_context::<PluginsView>();
    let service = try_consume_context::<Option<PluginsService>>().flatten();
    let entry = view.entry(&id);
    let mut working = use_signal(|| false);
    let mut refused = use_signal(|| None::<String>);

    let Some(entry) = entry else {
        return rsx! {
            style { "{PLUGINS_CSS}" }
            div {
                class: "pl-sheet",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "Enable plugin",
                onclick: move |e| e.stop_propagation(),
                h2 { class: "pl-sheet-title", "Plugin not found" }
                p { class: "pl-sheet-lede", "That plugin is no longer installed." }
                div { class: "pl-sheet-actions",
                    Button {
                        label: "Close".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Md,
                        onclick: move |_| {
                            state.overlays.write().pop();
                        },
                    }
                }
            }
        };
    };

    let name = entry.plugin.manifest.name.clone();
    let version = entry.plugin.manifest.version.clone();
    let lines = permission_lines(&entry);
    let hosts = entry.network_hosts();
    let secrets = entry.secret_fields();
    let needs_setup = !entry.missing_required().is_empty();
    let aria = format!("Enable {name}");

    let enable = {
        let id = id.clone();
        move |_| {
            let Some(service) = service.clone() else {
                refused.set(Some("Plugins are not connected".to_string()));
                return;
            };
            refused.set(None);
            working.set(true);
            let id = id.clone();
            spawn(async move {
                match service.enable(&id).await {
                    Ok(()) => {
                        state.overlays.write().pop();
                    }
                    Err(reason) => refused.set(Some(reason)),
                }
                working.set(false);
            });
        }
    };

    rsx! {
        style { "{PLUGINS_CSS}" }
        div {
            class: "pl-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": aria,
            onclick: move |e| e.stop_propagation(),
            h2 { class: "pl-sheet-title", "Enable {name}" }
            p { class: "pl-sheet-lede",
                "{name} {version} is a separate program. If you enable it, it can:"
            }
            ul { class: "pl-grants",
                for line in lines {
                    li { key: "{line}", "{line}" }
                }
                if entry.plugin.manifest.permissions.is_empty() {
                    li { "Show its own screens, nothing more" }
                }
            }
            if !hosts.is_empty() {
                div { class: "pl-hosts",
                    span { class: "pl-label", "Addresses it says it contacts" }
                    span { class: "pl-hosts-list", "{hosts.join(\", \")}" }
                    span { class: "pl-label",
                        "Berth shows these but does not block other addresses in this version."
                    }
                }
            }
            if !secrets.is_empty() {
                p { class: "pl-sheet-lede",
                    "Secrets it declares are kept in your Keychain and handed over only when it asks for one it is allowed to read."
                }
            }
            if needs_setup {
                p { class: "pl-sheet-lede", "After enabling, open Configure to finish setting it up." }
            }
            if let Some(reason) = refused.read().clone() {
                InlineMessage { message: reason, tone: InlineTone::Danger }
            }
            div { class: "pl-sheet-actions",
                Button {
                    label: "Cancel".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: move |_| {
                        state.overlays.write().pop();
                    },
                }
                Button {
                    label: format!("Enable {name}"),
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    loading: working(),
                    disabled: working(),
                    onclick: enable,
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "plugin_enable_tests.rs"]
mod tests;
