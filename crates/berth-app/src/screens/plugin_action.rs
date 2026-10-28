//! Plugin actions (P9-T5, AD52): the click that runs or asks first, the
//! confirm dialog (`Overlay::PluginAction`, drawn with `ConfirmDialog`) and
//! "Fix with <harness>" on a failed row's log, which goes through the one
//! assistant entry funnel (`services::assistant_entry`).

use std::collections::BTreeMap;

use dioxus::prelude::*;

use berth_plugin_protocol::ActionSpec;

use super::plugin_view_model::confirm_view;
use crate::services::assistant::AssistantService;
use crate::services::assistant_entry;
use crate::services::plugin_views::{
    linked_project, open_only, plugin_effect, PendingAction, PluginNav, PluginViewStore, RowContext,
};
use crate::services::plugins::{PluginsService, PluginsView};
use crate::state::{AppState, Overlay};
use crate::ui::{
    Button, ButtonVariant, CheckState, Checkbox, ConfirmDialog, ControlSize, Icon, IconSize, Menu,
    MenuEntry,
};

/// A click on an action. One that only opens a view (`{ "open": view }`) opens
/// it here, without the plugin; one with a `Confirm` asks first in the
/// overlay; any other runs at once.
pub fn request_action(
    store: PluginViewStore,
    service: Option<&PluginsService>,
    mut state: AppState,
    plugin: &str,
    action: &ActionSpec,
    context: Option<RowContext>,
) {
    if let Some(view) = open_only(action) {
        let mut nav = store.nav;
        let mut next = nav.peek().clone();
        next.row = context.map(|c| c.row);
        next.open = Some(view.to_string());
        nav.set(next);
        return;
    }
    let pending = PendingAction {
        plugin: plugin.to_string(),
        action: action.clone(),
        context,
    };
    if action.confirm.is_some() {
        state.overlays.write().push(Overlay::PluginAction(pending));
    } else {
        store.run_action(service, state, pending, BTreeMap::new());
    }
}

// ---------------------------------------------------------------------
// Fix with <harness>
// ---------------------------------------------------------------------

/// "Fix with Claude Code ▾": a primary button that opens the project linked to
/// the row's repository in the assistant with the failed log attached, and a
/// chevron that picks another installed harness. Disabled with its reason when
/// there is no assistant, harness, linked folder or log yet.
#[component]
pub fn FixWithHarness(
    plugin: String,
    context: RowContext,
    #[props(default)] on_started: Option<EventHandler<()>>,
) -> Element {
    let state = use_context::<AppState>();
    let store = use_context::<PluginViewStore>();
    let service = try_consume_context::<Option<AssistantService>>().flatten();
    let plugins = try_consume_context::<PluginsView>();
    let mut picked = use_signal(|| None::<String>);
    let mut menu = use_signal(|| false);

    let harnesses = state.harnesses.read().clone();
    let available: Vec<berth_core::Harness> =
        harnesses.iter().filter(|h| h.available).cloned().collect();
    let harness = picked
        .read()
        .as_ref()
        .and_then(|id| available.iter().find(|h| h.id.0 == *id).cloned())
        .or_else(|| assistant_entry::first_available(&harnesses));
    let projects = state.projects.read().clone();
    let project = context
        .repository
        .as_deref()
        .and_then(|repo| linked_project(&projects, repo))
        .cloned();
    let lines: Vec<String> = context
        .stream
        .as_ref()
        .and_then(|stream| {
            store
                .logs
                .read()
                .get(&(plugin.clone(), stream.clone()))
                .map(|log| log.lines.iter().map(|(_, l)| l.clone()).collect())
        })
        .unwrap_or_default();
    let plugin_name = plugins
        .and_then(|p| p.entry(&plugin))
        .map_or_else(|| plugin.clone(), |e| e.plugin.manifest.name.clone());
    let reason =
        assistant_entry::disabled_reason(service.is_some(), harness.as_ref(), project.as_ref())
            .or_else(|| {
                lines
                    .is_empty()
                    .then(|| "The log has not loaded yet".to_string())
            });
    let label = harness.as_ref().map_or_else(
        || "Fix with a harness".to_string(),
        |h| format!("Fix with {}", h.name),
    );
    let entries: Vec<MenuEntry> = available
        .iter()
        .map(|h| MenuEntry::new(h.id.0.clone(), h.name.clone()))
        .collect();

    let run = {
        let (title, harness, project) = (context.title.clone(), harness.clone(), project.clone());
        let model_state = state;
        move |_| {
            let (Some(service), Some(harness), Some(project)) =
                (service.clone(), harness.clone(), project.clone())
            else {
                return;
            };
            let Some(mut entry) =
                assistant_entry::plugin_log_entry(&project, &harness, &plugin_name, &title, &lines)
            else {
                return;
            };
            entry.request.model =
                assistant_entry::saved_model(&model_state.settings.peek(), &harness);
            let project_id = entry.request.project.clone();
            let mut shown = model_state;
            if let Some(done) = &on_started {
                done.call(());
            }
            spawn(async move {
                if assistant_entry::run(&service, entry).await.is_ok() {
                    assistant_entry::show(&mut shown, &project_id);
                }
            });
        }
    };

    rsx! {
        div { class: "pv-fix",
            Button {
                label,
                variant: ButtonVariant::Primary,
                size: ControlSize::Md,
                icon: "sparkles",
                disabled: reason.is_some(),
                disabled_reason: reason.clone(),
                onclick: run,
            }
            button {
                class: "pv-fix-chevron",
                r#type: "button",
                "aria-label": "Choose a tool",
                "aria-expanded": "{menu}",
                disabled: reason.is_some() || available.len() < 2,
                "data-tooltip": if reason.is_some() { reason.clone() } else if available.len() < 2 { Some("Only one assistant is installed".to_string()) } else { None },
                onclick: move |e| {
                    e.stop_propagation();
                    let open = *menu.peek();
                    menu.set(!open);
                },
                Icon { name: "chevron-down", size: IconSize::Sm }
            }
            if menu() {
                div { class: "pv-fix-menu",
                    Menu {
                        items: entries,
                        aria_label: Some("Assistants".to_string()),
                        on_select: move |id: String| {
                            picked.set(Some(id));
                            menu.set(false);
                        },
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// The confirm
// ---------------------------------------------------------------------

/// The body of `Overlay::PluginAction`: the plugin's `Confirm` in a
/// `ConfirmDialog`. Its button reads from the one `EffectSummary`
/// (`plugin_effect`), its rows, warning and notes from the same `Confirm`, and
/// the action's options are checkboxes. A failed row adds "Fix with <harness>".
#[component]
pub fn PluginActionDialog(pending: PendingAction) -> Element {
    let store = use_context::<PluginViewStore>();
    let mut state = use_context::<AppState>();
    let Some(confirm) = pending.action.confirm.clone() else {
        return rsx! {};
    };
    let view = confirm_view(&confirm);
    let effect = plugin_effect(&confirm);
    let mut chosen = use_signal(|| {
        pending
            .action
            .options
            .iter()
            .map(|o| (o.key.clone(), o.default))
            .collect::<BTreeMap<String, bool>>()
    });
    let warned = !view.warnings.is_empty();
    let variant = if pending.action.danger {
        ButtonVariant::Danger
    } else if warned {
        ButtonVariant::Secondary
    } else {
        ButtonVariant::Primary
    };
    // The frame's last note also says where "Fix with" works.
    let mut notes = view.notes.clone();
    let failed = pending.context.as_ref().is_some_and(|c| c.failed);
    if failed {
        let folder = pending
            .context
            .as_ref()
            .and_then(|c| c.repository.as_deref())
            .and_then(|repo| {
                linked_project(&state.projects.read(), repo).and_then(|p| p.folder.clone())
            });
        let harness = assistant_entry::first_available(&state.harnesses.read());
        if let (Some(folder), Some(harness)) = (folder, harness) {
            let shown = berth_core::fmt::tilde(&folder, &berth_sys::paths::home());
            let sentence = format!(
                "Fix with {} opens {shown} (git remote matches).",
                harness.name
            );
            match notes.last_mut() {
                Some(last) => *last = format!("{last} {sentence}"),
                None => notes.push(sentence),
            }
        }
    }
    let service = try_consume_context::<Option<PluginsService>>().flatten();
    let run_pending = pending.clone();
    rsx! {
        ConfirmDialog {
            title: view.title.clone(),
            effect,
            rows: view.rows.clone(),
            warning: view.warnings.first().cloned(),
            notes,
            button_label: Some(view.button.clone()),
            confirm_variant: variant,
            wide: true,
            body: rsx! {
                for option in pending.action.options.iter() {
                    {
                        let key = option.key.clone();
                        let on = chosen.read().get(&key).copied().unwrap_or(option.default);
                        rsx! {
                            div { key: "{option.key}", class: "confirm-option",
                                Checkbox {
                                    state: CheckState::from_bool(on),
                                    label: option.label.clone(),
                                    on_toggle: move |next: bool| {
                                        chosen.write().insert(key.clone(), next);
                                    },
                                }
                            }
                        }
                    }
                }
            },
            buttons: rsx! {
                if failed {
                    if let Some(context) = pending.context.clone() {
                        FixWithHarness {
                            plugin: pending.plugin.clone(),
                            context,
                            on_started: move |()| {
                                state.overlays.write().pop();
                            },
                        }
                    }
                }
            },
            oncheck: move |_| {},
            oncancel: move |()| {
                state.overlays.write().pop();
            },
            onconfirm: move |()| {
                state.overlays.write().pop();
                store.run_action(service.as_ref(), state, run_pending.clone(), chosen.peek().clone());
            },
        }
    }
}

/// Selecting a row: the nav moves to the row and what it opens.
pub fn select_row(
    store: PluginViewStore,
    plugin: &str,
    context: &RowContext,
    open: Option<String>,
) {
    let mut nav = store.nav;
    let view = nav.peek().view.clone();
    nav.set(PluginNav {
        plugin: plugin.to_string(),
        view,
        row: Some(context.row.clone()),
        open,
    });
}
