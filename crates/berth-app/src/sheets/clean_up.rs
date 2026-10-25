//! The Clean up sheet (P4-T7; frames "Clean up / sheet / 1440" and "Clean
//! up / done / 1440"). One checkbox per group of things the engine could
//! shed, the stopped containers named under theirs, a summary of what is
//! ticked and one red button whose text is the plan's own effect. Volumes
//! start unticked: they hold data.
//!
//! Removing goes item by item through [`prune::run`] with a progress bar.
//! A clean run closes the sheet and toasts "Removed 13 items · 5.7 GB
//! freed"; a run with failures keeps the failed items listed with the
//! engine's reason, so the person can try them again.

use std::collections::BTreeSet;

use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use berth_core::EngineId;

use crate::services::engine_dashboard::{DashboardStore, UsageEntry};
use crate::services::prune::{self, Entry, Group, Lists, Progress};
use crate::services::Services;
use crate::state::{AppState, Toast, ToastTone};
use crate::ui::progress_bar::PROGRESS_BAR_CSS;
use crate::ui::{Button, ButtonVariant, CheckState, Checkbox, ControlSize, ProgressBar};

/// The sheet's CSS, rendered with it (tokens only).
pub const CLEAN_UP_CSS: &str = include_str!("../../assets/css/clean_up.css");

/// What the sheet lists instead of reading the lists (scenes and tests).
#[derive(Clone)]
pub struct SceneEntries(pub Vec<Entry>);

/// What the sheet is doing.
#[derive(Debug, Clone, PartialEq)]
enum Phase {
    Choosing,
    Running(Progress),
}

/// The entries the sheet opens with: the scene's, else built from the
/// lists Berth holds and the engine's last `disk_usage`. The dashboard asks
/// the same question to know whether Clean up has anything to offer.
pub(crate) fn initial_entries(
    state: &AppState,
    store: Option<DashboardStore>,
    engine: &EngineId,
) -> Vec<Entry> {
    if let Some(SceneEntries(entries)) = try_consume_context::<SceneEntries>() {
        return entries;
    }
    let usage = store.and_then(|s| match s.usage.read().get(engine) {
        Some(UsageEntry::Ready(usage)) => Some(usage.clone()),
        _ => None,
    });
    let containers = state.containers.read();
    let images = state.images.read();
    let volumes = state.volumes.read();
    let networks = state.networks.read();
    prune::candidates(
        engine,
        &Lists {
            containers: &containers,
            images: &images,
            volumes: &volumes,
            networks: &networks,
            usage: usage.as_ref(),
        },
    )
}

enum Msg {
    Progress(Progress),
    Done(prune::Outcome),
}

/// What a finished run leaves behind besides the sheet's own state: the
/// activity line, and a fresh `disk_usage` so the dashboard shows what is
/// reclaimable now (and offers Clean up only if something is left).
fn after_run(
    services: &Services,
    store: Option<DashboardStore>,
    engine: &EngineId,
    outcome: &prune::Outcome,
) {
    prune::record(&services.store, outcome);
    if let Some(store) = store {
        store.refresh_usage(services, engine);
    }
}

#[component]
pub fn CleanUpSheet(engine: EngineId) -> Element {
    let mut state = use_context::<AppState>();
    let services = try_consume_context::<Services>();
    let store = try_consume_context::<DashboardStore>();
    let mut entries = use_signal({
        let engine = engine.clone();
        move || initial_entries(&state, store, &engine)
    });
    let mut checked = use_signal(prune::default_checked);
    let mut phase = use_signal(|| Phase::Choosing);
    let mut notice = use_signal(|| None::<String>);
    let mut stop = use_signal(|| None::<CancellationToken>);

    let engine_name = state
        .engines
        .read()
        .iter()
        .find(|e| e.id == engine)
        .map_or_else(|| engine.0.clone(), |e| e.name.clone());
    let list = entries.read().clone();
    let ticked = checked.read().clone();
    let running = matches!(&*phase.read(), Phase::Running(_));
    let plan = prune::plan(&engine, &list, &ticked);
    let effect = plan.effect();
    let blocker = if effect.count == 0 {
        Some("Tick something to remove".to_string())
    } else {
        None
    };
    let groups: Vec<Group> = Group::ALL
        .into_iter()
        .filter(|g| list.iter().any(|e| e.group == *g))
        .collect();

    let mut start = {
        let engine = engine.clone();
        let services = services.clone();
        move |()| {
            let Some(services) = services.clone() else {
                notice.set(Some("Clean up is not connected".to_string()));
                return;
            };
            let Some(client) = services.client(&engine) else {
                notice.set(Some("The engine is not connected".to_string()));
                return;
            };
            let plan = prune::plan(&engine, &entries.peek(), &checked.peek());
            let total = plan.effect().count as usize;
            if total == 0 {
                return;
            }
            notice.set(None);
            phase.set(Phase::Running(Progress {
                done: 0,
                total,
                current: String::new(),
            }));
            let cancel = services.cancel.child_token();
            stop.set(Some(cancel.clone()));
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Msg>();
            crate::services::actions::spawn_io(async move {
                let progress = tx.clone();
                let outcome = prune::run(client.as_ref(), &plan, cancel, move |p| {
                    let _ = progress.send(Msg::Progress(p));
                })
                .await;
                let _ = tx.send(Msg::Done(outcome));
            });
            let engine = engine.clone();
            let store = store;
            spawn(async move {
                while let Some(msg) = rx.recv().await {
                    match msg {
                        Msg::Progress(p) => phase.set(Phase::Running(p)),
                        Msg::Done(outcome) => {
                            after_run(&services, store, &engine, &outcome);
                            stop.set(None);
                            phase.set(Phase::Choosing);
                            if outcome.clean() {
                                state.toasts.write().push(Toast {
                                    id: uuid::Uuid::now_v7(),
                                    text: outcome.toast_text(),
                                    tone: ToastTone::Success,
                                    action: None,
                                });
                                state.overlays.write().pop();
                            } else {
                                let rest = prune::remaining(entries.peek().clone(), &outcome);
                                entries.set(rest);
                                notice.set(Some(prune::partial_notice(&outcome)));
                            }
                            break;
                        }
                    }
                }
            });
        }
    };

    let mut toggle = move |group: Group, on: bool| {
        let mut set: BTreeSet<Group> = checked.peek().clone();
        if on {
            set.insert(group);
        } else {
            set.remove(&group);
        }
        checked.set(set);
    };

    let progress = match &*phase.read() {
        Phase::Running(p) => Some(p.clone()),
        Phase::Choosing => None,
    };
    let nothing = list.is_empty();

    rsx! {
        style { "{CLEAN_UP_CSS}" }
        style { "{PROGRESS_BAR_CSS}" }
        div {
            class: "cu-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Clean up {engine_name}",
            onclick: move |e| e.stop_propagation(),
            h2 { class: "cu-title", "Clean up {engine_name}" }
            if nothing {
                p { class: "cu-empty",
                    if notice.read().is_some() {
                        "Nothing is left to clean up on engine {engine_name}."
                    } else {
                        "Nothing to clean up on engine {engine_name}."
                    }
                }
            } else {
                p { class: "cu-lede",
                    "Select what to remove from engine {engine_name}. Nothing is removed until you press the button."
                }
                ul { class: "cu-groups",
                    for group in groups.iter().copied() {
                        {group_row(group, &list, ticked.contains(&group), running, move |on| toggle(group, on))}
                    }
                }
                p { class: "cu-note", "Containers that belong to a project are never cleaned up here." }
            }
            if let Some(text) = notice.read().clone() {
                p { class: "cu-result", role: "status", "{text}" }
            }
            if let Some(p) = progress {
                div { class: "cu-progress",
                    ProgressBar {
                        fraction: Some(p.fraction()),
                        label: if p.current.is_empty() {
                            "Removing\u{2026}".to_string()
                        } else {
                            format!("Removing {}", p.current)
                        },
                        value_text: format!("{} of {}", p.done, p.total),
                        aria_label: "Clean up progress".to_string(),
                    }
                }
            } else if !nothing {
                p { class: "cu-summary", "{prune::summary_line(&plan, &list, &ticked)}" }
            }
            div { class: "cu-actions",
                Button {
                    label: if running { "Stop".to_string() } else if nothing { "Close".to_string() } else { "Cancel".to_string() },
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: move |_| {
                        if let Some(token) = stop.peek().as_ref() {
                            token.cancel();
                        } else {
                            state.overlays.write().pop();
                        }
                    },
                }
                if !nothing {
                    Button {
                        label: if running { "Removing\u{2026}".to_string() } else { effect.button_label() },
                        variant: ButtonVariant::Danger,
                        size: ControlSize::Md,
                        loading: running,
                        disabled: blocker.is_some(),
                        disabled_reason: blocker,
                        onclick: move |_| start(()),
                    }
                }
            }
        }
    }
}

/// One group's row, and its items when the sheet lists them.
fn group_row(
    group: Group,
    entries: &[Entry],
    on: bool,
    running: bool,
    on_toggle: impl FnMut(bool) + 'static,
) -> Element {
    let members = prune::in_group(entries, group);
    let size = prune::group_size(entries, group);
    let sized = members.iter().any(|e| e.item.size.is_some());
    let count = group.count_text(members.len());
    let count = match group.hint() {
        Some(hint) => format!("{count} \u{b7} {hint}"),
        None => count,
    };
    let show_items = group.lists_items(on) || members.iter().any(|e| e.failure.is_some());
    let label = group.label();
    rsx! {
        li { key: "{group.id()}", class: "cu-group",
            div { class: "cu-row",
                Checkbox {
                    state: CheckState::from_bool(on),
                    aria_label: label.to_string(),
                    disabled: running,
                    on_toggle,
                }
                span { class: "cu-row-label", "{label}" }
                span { class: "cu-count", "{count}" }
                span { class: "cu-spacer" }
                if sized {
                    span { class: "cu-size", "{berth_core::fmt::bytes(size)}" }
                }
            }
            if show_items {
                ul { class: "cu-items",
                    for e in members.iter() {
                        li { key: "{e.item.id}", class: "cu-item",
                            span { "{e.item.name}" }
                            if let Some(detail) = &e.detail {
                                span { class: "cu-item-detail", "{detail}" }
                            }
                            if let Some(reason) = &e.failure {
                                span { class: "cu-item-failure", "{reason}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "clean_up_tests.rs"]
mod tests;
