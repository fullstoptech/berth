//! The Configure form of a plugin (P9-T2, AD48). One field per manifest
//! `config` entry, plus a masked field for every secret the manifest names
//! (a `Secret` setting or a `secrets` declaration). Secrets go to the
//! Keychain through the service and are never shown again: a stored one
//! reads "Saved" and an empty field keeps it.

use std::collections::BTreeMap;

use dioxus::prelude::*;
use serde_json::{json, Value};

use berth_plugin_protocol::FieldKind;

use crate::screens::plugins::PLUGINS_CSS;
use crate::services::plugins::{
    field_problem, ConfigUpdate, PluginEntry, PluginsService, PluginsView,
};
use crate::state::AppState;
use crate::ui::{Button, ButtonVariant, ControlSize, InlineMessage, InlineTone, Switch, TextField};

/// The first reason the form cannot be saved yet, in field order, or none.
#[must_use]
pub fn form_blocker(
    entry: &PluginEntry,
    values: &BTreeMap<String, Value>,
    typed_secrets: &BTreeMap<String, String>,
) -> Option<String> {
    for field in &entry.plugin.manifest.config {
        match field.kind {
            FieldKind::Secret => {
                let typed = typed_secrets.get(&field.key).is_some_and(|v| !v.is_empty());
                if field.required && !typed && !entry.secrets_set.contains(&field.key) {
                    return Some(format!("{} is required", field.label));
                }
            }
            FieldKind::Bool => {}
            FieldKind::Text | FieldKind::Url => {
                let text = values.get(&field.key).and_then(Value::as_str).unwrap_or("");
                if let Some(problem) = field_problem(field, text) {
                    return Some(problem);
                }
            }
        }
    }
    None
}

/// The form's starting values: the saved settings of the entry.
#[must_use]
pub fn initial_values(entry: &PluginEntry) -> BTreeMap<String, Value> {
    entry
        .plugin
        .manifest
        .config
        .iter()
        .filter(|f| f.kind != FieldKind::Secret)
        .filter_map(|f| {
            let saved = entry.config.get(&f.key).cloned();
            match f.kind {
                FieldKind::Bool => Some((f.key.clone(), saved.unwrap_or(json!(false)))),
                _ => saved.map(|v| (f.key.clone(), v)),
            }
        })
        .collect()
}

#[component]
pub fn PluginConfigureSheet(id: String) -> Element {
    let mut state = use_context::<AppState>();
    let view = use_context::<PluginsView>();
    let service = try_consume_context::<Option<PluginsService>>().flatten();
    let seed = view.entry(&id);
    let mut values = use_signal(|| seed.as_ref().map(initial_values).unwrap_or_default());
    let mut typed = use_signal(BTreeMap::<String, String>::new);
    let mut working = use_signal(|| false);
    let mut refused = use_signal(|| None::<String>);

    let Some(entry) = view.entry(&id) else {
        return rsx! {
            style { "{PLUGINS_CSS}" }
            div {
                class: "pl-sheet",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "Configure plugin",
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
    let fields: Vec<_> = entry
        .plugin
        .manifest
        .config
        .iter()
        .filter(|f| f.kind != FieldKind::Secret)
        .cloned()
        .collect();
    let secret_fields = entry.secret_fields();
    let blocker = form_blocker(&entry, &values.read(), &typed.read()).or_else(|| {
        service
            .is_none()
            .then(|| "Plugins are not connected".to_string())
    });
    let aria = format!("Configure {name}");
    let restarts_note = entry.plugin.state != berth_plugin_host::PluginState::Disabled;

    let save = {
        let id = id.clone();
        let service = service.clone();
        move |_| {
            let Some(service) = service.clone() else {
                return;
            };
            refused.set(None);
            working.set(true);
            let update = ConfigUpdate {
                values: values.peek().clone(),
                secrets: typed
                    .peek()
                    .iter()
                    .map(|(k, v)| (k.clone(), secrecy::SecretString::from(v.clone())))
                    .collect(),
            };
            let id = id.clone();
            spawn(async move {
                match service.configure(&id, update).await {
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
            h2 { class: "pl-sheet-title", "Configure {name}" }
            p { class: "pl-sheet-lede",
                "Settings stay on this Mac. Secrets go to your Keychain, never to Berth's database or logs."
                if restarts_note {
                    " Saving restarts the plugin."
                }
            }
            div { class: "pl-form",
                for field in fields {
                    if field.kind == FieldKind::Bool {
                        Switch {
                            key: "{field.key}",
                            on: values.read().get(&field.key).and_then(Value::as_bool).unwrap_or(false),
                            label: field.label.clone(),
                            aria_label: field.label.clone(),
                            on_toggle: {
                                let key = field.key.clone();
                                move |next: bool| {
                                    values.write().insert(key.clone(), json!(next));
                                }
                            },
                        }
                    } else {
                        TextField {
                            key: "{field.key}",
                            label: if field.required { field.label.clone() } else { format!("{} (optional)", field.label) },
                            initial_value: values
                                .peek()
                                .get(&field.key)
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            placeholder: (field.kind == FieldKind::Url).then(|| "https://".to_string()),
                            aria_label: field.label.clone(),
                            oninput: {
                                let key = field.key.clone();
                                move |e: FormEvent| {
                                    values.write().insert(key.clone(), json!(e.value()));
                                }
                            },
                        }
                    }
                }
                for (key , label) in secret_fields {
                    div { key: "{key}", class: "pl-secret-row",
                        TextField {
                            label: label.clone(),
                            input_type: "password".to_string(),
                            aria_label: label.clone(),
                            placeholder: entry.secrets_set.contains(&key).then(|| "Saved".to_string()),
                            help: if entry.secrets_set.contains(&key) {
                                "Saved in your Keychain. Leave empty to keep it.".to_string()
                            } else {
                                "Stored in your Keychain once you save.".to_string()
                            },
                            oninput: {
                                let key = key.clone();
                                move |e: FormEvent| {
                                    typed.write().insert(key.clone(), e.value());
                                }
                            },
                        }
                        if entry.secrets_set.contains(&key) {
                            Button {
                                label: "Remove saved value".to_string(),
                                variant: ButtonVariant::Ghost,
                                size: ControlSize::Sm,
                                onclick: {
                                    let key = key.clone();
                                    let id = id.clone();
                                    let service = service.clone();
                                    move |_| {
                                        let Some(service) = service.clone() else { return };
                                        let key = key.clone();
                                        let id = id.clone();
                                        spawn(async move {
                                            if let Err(reason) = service.clear_secret(&id, &key).await {
                                                refused.set(Some(reason));
                                            }
                                        });
                                    }
                                },
                            }
                        }
                    }
                }
                if let Some(reason) = refused.read().clone() {
                    InlineMessage { message: reason, tone: InlineTone::Danger }
                }
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
                    label: "Save settings".to_string(),
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    loading: working(),
                    disabled: blocker.is_some() || working(),
                    disabled_reason: blocker,
                    onclick: save,
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "plugin_configure_tests.rs"]
mod tests;
