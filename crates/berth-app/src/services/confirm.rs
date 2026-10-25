//! Confirm models (D33): the one place that builds the `EffectSummary` of
//! a container or engine action that asks first. The dialog's button, its
//! text and the result message all render from that one value
//! ([`button_label`], the lines, [`result_label`]); `search.rs` (palette),
//! the row, the bulk bar and ⌫ all come here.
//!
//! [`run`] is the other half: it sends a confirmed model to the actions
//! service and returns the pending keys, so the caller can watch for the
//! outcome ([`watch_result`]) and say it from the same summary.

use berth_core::{
    ColimaOp, Container, ContainerAction, ContainerId, ContainerState, EffectKind, EffectLine,
    EffectSummary, EngineAction, PruneItem, PruneKind, PrunePlan, Subject, Volume,
};
use dioxus::prelude::{ReadableExt, WritableExt};

use crate::services::actions::{container_action_id, ActionsService, ContainerTarget};
use crate::state::{AppState, ConfirmModel, PendingKey, PendingState, Toast, ToastTone};

/// "1 container" or "3 containers".
#[must_use]
pub fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

fn line(kind: EffectKind, text: impl Into<String>) -> EffectLine {
    EffectLine {
        kind,
        text: text.into(),
    }
}

fn summary(
    verb: &str,
    noun_one: &str,
    noun_many: &str,
    count: usize,
    lines: Vec<EffectLine>,
) -> EffectSummary {
    EffectSummary {
        verb: verb.to_string(),
        noun_one: noun_one.to_string(),
        noun_many: noun_many.to_string(),
        count: u32::try_from(count).unwrap_or(u32::MAX),
        bytes: None,
        lines,
        typed_name: None,
    }
}

fn is_active(state: ContainerState) -> bool {
    matches!(
        state,
        ContainerState::Running | ContainerState::Restarting | ContainerState::Paused
    )
}

/// Docker names an anonymous volume with 64 hex digits; a named volume
/// survives `docker rm -v`.
#[must_use]
pub fn is_anonymous_volume(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The named volumes these containers mount, once each, in mount order.
fn kept_volumes<'a>(containers: &[&'a Container]) -> Vec<&'a str> {
    let mut names: Vec<&str> = Vec::new();
    for container in containers {
        for mount in &container.mounts {
            if let Some(name) = mount.volume_name.as_deref() {
                if !is_anonymous_volume(name) && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    names
}

/// How many kept volumes get a line each before they are summed up.
const KEPT_LINES: usize = 3;

fn kept_lines(containers: &[&Container], volumes: &[Volume]) -> Vec<EffectLine> {
    let names = kept_volumes(containers);
    let size_of = |name: &str| {
        volumes
            .iter()
            .find(|v| v.name == name)
            .and_then(|v| v.size)
            .map(berth_core::fmt::bytes)
    };
    if names.len() <= KEPT_LINES {
        return names
            .into_iter()
            .map(|name| {
                let text = match size_of(name) {
                    Some(size) => format!("Volume {name} ({size}) is kept."),
                    None => format!("Volume {name} is kept."),
                };
                line(EffectKind::Stays, text)
            })
            .collect();
    }
    let total = names
        .iter()
        .filter_map(|n| volumes.iter().find(|v| v.name == *n).and_then(|v| v.size))
        .reduce(|a, b| a + b);
    let sum = total.map_or_else(String::new, |t| format!(" ({})", berth_core::fmt::bytes(t)));
    vec![line(
        EffectKind::Stays,
        format!(
            "{} kept{sum}.",
            plural(names.len(), "volume is", "volumes are")
        ),
    )]
}

/// What deleting these containers does. A running container is stopped
/// first, which is its own `Changes` line (it also turns the button into
/// "Stop and delete").
#[must_use]
pub fn delete_effect(containers: &[&Container], volumes: &[Volume]) -> EffectSummary {
    let one = containers.len() == 1;
    let mut lines = vec![line(
        EffectKind::Goes,
        if one {
            "Its writable layer is deleted."
        } else {
            "Their writable layers are deleted."
        },
    )];
    lines.extend(kept_lines(containers, volumes));
    let running: Vec<&&Container> = containers.iter().filter(|c| is_active(c.state)).collect();
    match running.as_slice() {
        [] => {}
        [only] => lines.push(line(
            EffectKind::Changes,
            format!("{} is stopped first.", only.name),
        )),
        many => lines.push(line(
            EffectKind::Changes,
            format!(
                "{} are stopped first.",
                plural(many.len(), "running container", "running containers")
            ),
        )),
    }
    summary("Delete", "container", "containers", containers.len(), lines)
}

/// What killing a container does.
#[must_use]
pub fn kill_effect(container: &Container) -> EffectSummary {
    summary(
        "Kill",
        "container",
        "containers",
        1,
        vec![line(
            EffectKind::Changes,
            format!("{} stops at once without a clean shutdown", container.name),
        )],
    )
}

/// What stopping or restarting a Colima engine does to its containers.
#[must_use]
pub fn engine_effect(stop: bool, running: usize) -> EffectSummary {
    let (verb, when) = if stop {
        ("Stop", "is stopped")
    } else {
        ("Restart", "restarts")
    };
    summary(
        verb,
        "engine",
        "engines",
        1,
        vec![line(
            EffectKind::Changes,
            format!(
                "{} stop while the engine {when}",
                plural(running, "running container", "running containers")
            ),
        )],
    )
}

/// The confirm button's text, from the summary alone: "Delete container",
/// "Delete 4 containers", "Stop and delete" when something runs.
#[must_use]
pub fn button_label(effect: &EffectSummary) -> String {
    let stops_first =
        effect.verb == "Delete" && effect.lines.iter().any(|l| l.kind == EffectKind::Changes);
    match (stops_first, effect.count) {
        (true, 1) => "Stop and delete".to_string(),
        (true, n) => format!("Stop and delete {n} {}", effect.noun_many),
        (false, 1) => format!("{} {}", effect.verb, effect.noun_one),
        (false, _) => effect.button_label(),
    }
}

/// The result message when `done` of the summary's items went through:
/// the same count and nouns as the button, in the past tense.
#[must_use]
pub fn result_label(effect: &EffectSummary, done: usize) -> String {
    let mut done_effect = effect.clone();
    done_effect.count = u32::try_from(done).unwrap_or(u32::MAX);
    done_effect.bytes = None;
    done_effect.result_label()
}

// ---------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------

fn find_containers<'a>(all: &'a [Container], ids: &[ContainerId]) -> Vec<&'a Container> {
    ids.iter()
        .filter_map(|id| all.iter().find(|c| c.id == *id))
        .collect()
}

/// The delete dialog for one or more containers. `None` when none of the
/// ids is known any more.
#[must_use]
pub fn delete_model(state: &AppState, ids: &[ContainerId]) -> Option<ConfirmModel> {
    let containers = state.containers.peek();
    let volumes = state.volumes.peek();
    let found = find_containers(&containers, ids);
    let first = found.first()?;
    let effect = delete_effect(&found, &volumes);
    let checkbox = Some(("Also delete its anonymous volumes".to_string(), false));
    if let [only] = found.as_slice() {
        return Some(ConfirmModel {
            title: format!("Delete container {}?", only.name),
            effect,
            action: EngineAction::Container {
                engine: only.engine.clone(),
                id: only.id.clone(),
                action: ContainerAction::Remove {
                    force: is_active(only.state),
                    volumes: false,
                },
            },
            checkbox,
        });
    }
    // PLAN §6 has no multi-container `EngineAction`: a bulk delete is a
    // `Prune` plan of container items, each resolved to its own engine when
    // it runs.
    let items = found
        .iter()
        .map(|c| PruneItem {
            kind: PruneKind::Container,
            id: c.id.0.clone(),
            name: c.name.clone(),
            size: None,
            selected: true,
        })
        .collect();
    Some(ConfirmModel {
        title: format!("Delete {}?", plural(found.len(), "container", "containers")),
        effect,
        action: EngineAction::Prune {
            plan: PrunePlan {
                engine: first.engine.clone(),
                items,
            },
        },
        checkbox: checkbox.map(|(_, on)| ("Also delete their anonymous volumes".to_string(), on)),
    })
}

/// The kill dialog for one container.
#[must_use]
pub fn kill_model(state: &AppState, id: &ContainerId) -> Option<ConfirmModel> {
    let containers = state.containers.peek();
    let container = containers.iter().find(|c| c.id == *id)?;
    Some(ConfirmModel {
        title: format!("Kill {}?", container.name),
        effect: kill_effect(container),
        action: EngineAction::Container {
            engine: container.engine.clone(),
            id: container.id.clone(),
            action: ContainerAction::Kill { signal: None },
        },
        checkbox: None,
    })
}

/// The stop or restart dialog for a Colima engine.
#[must_use]
pub fn engine_model(
    engine_name: &str,
    profile: String,
    op: ColimaOp,
    running: usize,
) -> ConfirmModel {
    let stop = matches!(op, ColimaOp::Stop);
    let verb = if stop { "Stop" } else { "Restart" };
    ConfirmModel {
        title: format!("{verb} engine {engine_name}?"),
        effect: engine_effect(stop, running),
        action: EngineAction::Colima { profile, op },
        checkbox: None,
    }
}

/// Open the delete dialog for these containers on the overlay stack.
/// Returns false when nothing is left to delete.
pub fn open_delete(state: &mut AppState, ids: &[ContainerId]) -> bool {
    let Some(model) = delete_model(state, ids) else {
        return false;
    };
    state
        .overlays
        .write()
        .push(crate::state::Overlay::Confirm(model));
    true
}

// ---------------------------------------------------------------------
// Running a confirmed model
// ---------------------------------------------------------------------

/// What [`run`] started: the pending keys to watch and the reasons the
/// actions service refused the rest.
#[derive(Debug, Default, PartialEq)]
pub struct Launched {
    pub keys: Vec<PendingKey>,
    pub refused: Vec<String>,
}

fn launch_container(
    state: &mut AppState,
    actions: &ActionsService,
    id: &ContainerId,
    action: ContainerAction,
    out: &mut Launched,
) {
    let target = state
        .containers
        .peek()
        .iter()
        .find(|c| c.id == *id)
        .map(|c| (ContainerTarget::from(c), is_active(c.state)));
    let Some((target, active)) = target else {
        out.refused
            .push("This container no longer exists".to_string());
        return;
    };
    // A running container needs `force` to be removed.
    let action = match action {
        ContainerAction::Remove { force, volumes } => ContainerAction::Remove {
            force: force || active,
            volumes,
        },
        other => other,
    };
    let key = PendingKey {
        subject: Subject::Container(id.clone()),
        action: container_action_id(&action),
    };
    match actions.container(state, &target, action) {
        Ok(()) => out.keys.push(key),
        Err(reason) => out.refused.push(reason),
    }
}

/// The model's action with the dialog's checkbox applied: on a delete it
/// decides whether anonymous volumes go too; any other action is as is.
#[must_use]
pub fn with_volumes(action: &ContainerAction, checked: bool) -> ContainerAction {
    match action {
        ContainerAction::Remove { force, .. } => ContainerAction::Remove {
            force: *force,
            volumes: checked,
        },
        other => other.clone(),
    }
}

/// Send a confirmed model to the actions service. `checked` is the state
/// of the dialog's checkbox (it means "also delete anonymous volumes" on a
/// delete).
pub fn run(
    state: &mut AppState,
    actions: &ActionsService,
    model: &ConfirmModel,
    checked: bool,
) -> Launched {
    let mut out = Launched::default();
    match &model.action {
        EngineAction::Container { id, action, .. } => {
            launch_container(state, actions, id, with_volumes(action, checked), &mut out);
        }
        EngineAction::Prune { plan } => {
            for item in plan
                .items
                .iter()
                .filter(|i| i.kind == PruneKind::Container && i.selected)
            {
                launch_container(
                    state,
                    actions,
                    &ContainerId(item.id.clone()),
                    ContainerAction::Remove {
                        force: false,
                        volumes: checked,
                    },
                    &mut out,
                );
            }
        }
        EngineAction::Colima { profile, op } => {
            let engine = state
                .engines
                .peek()
                .iter()
                .find(|e| crate::services::colima::profile_of(e) == Some(profile.as_str()))
                .cloned();
            match engine {
                Some(engine) => match actions.engine(state, &engine, op.clone()) {
                    Ok(()) => out.keys.push(PendingKey {
                        subject: Subject::Engine(engine.id.clone()),
                        action: crate::services::colima::action_id(op),
                    }),
                    Err(reason) => out.refused.push(reason),
                },
                None => out.refused.push(format!("Engine {profile} is gone")),
            }
        }
        _ => out
            .refused
            .push("This action cannot run from here yet".to_string()),
    }
    out
}

/// How many of `keys` finished without a failure, once none is running.
/// `None` while one still runs.
#[must_use]
pub fn finished_count(
    pending: &std::collections::BTreeMap<PendingKey, PendingState>,
    keys: &[PendingKey],
) -> Option<usize> {
    if keys
        .iter()
        .any(|k| matches!(pending.get(k), Some(PendingState::Running { .. })))
    {
        return None;
    }
    Some(
        keys.iter()
            .filter(|k| !matches!(pending.get(*k), Some(PendingState::Failed { .. })))
            .count(),
    )
}

fn push_toast(state: &mut AppState, text: String, tone: ToastTone) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone,
        action: None,
    });
}

/// After [`run`]: say what was refused, then wait for the started actions
/// and say what went through, from the same summary the dialog showed.
/// Failures stay on their rows (D34), so only successes are counted.
pub async fn watch_result(mut state: AppState, effect: EffectSummary, launched: Launched) {
    for reason in launched.refused {
        push_toast(&mut state, reason, ToastTone::Danger);
    }
    if launched.keys.is_empty() {
        return;
    }
    let done = loop {
        if let Some(done) = finished_count(&state.pending.peek(), &launched.keys) {
            break done;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    if done > 0 && matches!(effect.verb.as_str(), "Delete" | "Kill") {
        push_toast(&mut state, result_label(&effect, done), ToastTone::Success);
    }
}

#[cfg(test)]
#[path = "confirm_tests.rs"]
mod tests;
