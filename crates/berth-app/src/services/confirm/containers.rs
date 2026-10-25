//! Container confirms: delete, kill, recreate with the latest image.

use super::*;

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

/// What recreating a container does (P3-T7, D18): the loss warning of the
/// edit sheet, the dialog of "Recreate with latest image", the activity
/// entry and the result all read this one summary. Settings the spec
/// cannot carry never get here: Recreate is disabled for them (D16).
#[must_use]
pub fn recreate_effect(
    container: &Container,
    volumes: &[Volume],
    latest_image: bool,
) -> EffectSummary {
    let mut lines = Vec::new();
    if latest_image {
        lines.push(line(
            EffectKind::Changes,
            format!("{} is pulled again first.", container.image.reference),
        ));
    }
    if is_active(container.state) {
        lines.push(line(
            EffectKind::Changes,
            format!(
                "{} is stopped, then the new container is started.",
                container.name
            ),
        ));
    }
    lines.push(line(
        EffectKind::Goes,
        "Files written inside the container, outside volumes, are lost.",
    ));
    lines.extend(kept_lines(&[container], volumes));
    summary("Recreate", "container", "containers", 1, lines)
}

/// The result of one recreate, in words ("Recreated 1 container").
#[must_use]
pub fn recreate_result() -> String {
    result_label(
        &summary("Recreate", "container", "containers", 1, Vec::new()),
        1,
    )
}

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
    let checkbox = Some((delete_checkbox_label(&found), false));
    let targets: Vec<Subject> = found
        .iter()
        .map(|c| Subject::Container(c.id.clone()))
        .collect();
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
            targets,
        });
    }
    // PLAN §6 has no multi-container `EngineAction`, so a bulk model's
    // action names the first container and `run` walks `targets`.
    Some(ConfirmModel {
        title: format!("Delete {}?", plural(found.len(), "container", "containers")),
        effect,
        action: EngineAction::Container {
            engine: first.engine.clone(),
            id: first.id.clone(),
            action: ContainerAction::Remove {
                force: is_active(first.state),
                volumes: false,
            },
        },
        checkbox,
        targets,
    })
}

/// The delete dialog's checkbox (frame "Containers / confirm delete"):
/// "Also delete anonymous volumes", with the named volumes that stay put
/// listed so the box cannot be read as "all volumes".
#[must_use]
pub fn delete_checkbox_label(containers: &[&Container]) -> String {
    let kept = kept_volumes(containers);
    match kept.as_slice() {
        [] => "Also delete anonymous volumes".to_string(),
        names if names.len() <= KEPT_LINES => {
            format!("Also delete anonymous volumes (not {})", names.join(", "))
        }
        names => format!(
            "Also delete anonymous volumes (not {} named volumes)",
            names.len()
        ),
    }
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
        targets: vec![Subject::Container(container.id.clone())],
    })
}

/// The dialog of "Recreate with latest image": one `EngineAction::Recreate`
/// with `pull: true` (D18). The spec is read when the dialog opens, like
/// the targets.
#[must_use]
pub fn recreate_latest_model(
    state: &AppState,
    id: &ContainerId,
    spec: berth_core::ContainerSpec,
) -> Option<ConfirmModel> {
    let containers = state.containers.peek();
    let volumes = state.volumes.peek();
    let container = containers.iter().find(|c| c.id == *id)?;
    Some(ConfirmModel {
        title: format!("Recreate {} with the latest image?", container.name),
        effect: recreate_effect(container, &volumes, true),
        action: EngineAction::Recreate {
            engine: container.engine.clone(),
            id: container.id.clone(),
            spec,
            pull: true,
        },
        checkbox: None,
        targets: vec![Subject::Container(container.id.clone())],
    })
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

fn launch_recreate(
    state: &mut AppState,
    actions: &ActionsService,
    id: &ContainerId,
    spec: &berth_core::ContainerSpec,
    pull: bool,
    out: &mut Launched,
) {
    let target = state
        .containers
        .peek()
        .iter()
        .find(|c| c.id == *id)
        .map(ContainerTarget::from);
    let Some(target) = target else {
        out.refused
            .push("This container no longer exists".to_string());
        return;
    };
    let op = crate::services::edit::EditOp::Recreate {
        spec: Box::new(spec.clone()),
        pull,
    };
    let key = PendingKey {
        subject: Subject::Container(target.id.clone()),
        action: op.action_id(),
    };
    match actions.edit(state, &target, op) {
        Ok(()) => out.keys.push(key),
        Err(reason) => out.refused.push(reason),
    }
}

/// A container action: one start per target, each with its own pending
/// state and activity entry. The selection is not consulted. A bulk model's
/// action names its first target (PLAN §6 has no multi-container action), so
/// that one must be the container the action names.
pub(super) fn run(ctx: &mut Run<'_>) -> bool {
    let EngineAction::Container { id, action, .. } = &ctx.model.action else {
        return false;
    };
    let action = with_volumes(action, ctx.checked);
    if ctx.model.targets.first() != Some(&Subject::Container(id.clone())) {
        ctx.out
            .refused
            .push("This is not the container to act on".to_string());
        return true;
    }
    ctx.each_named(
        |target| matches!(target, Subject::Container(_)),
        "This is not a container",
        |ctx, target| {
            if let Subject::Container(id) = target {
                launch_container(ctx.state, ctx.actions, id, action.clone(), &mut ctx.out);
            }
        },
    );
    true
}

/// Edit and recreate, and recreate with the latest image (D18).
pub(super) fn run_recreate(ctx: &mut Run<'_>) -> bool {
    let EngineAction::Recreate { id, spec, pull, .. } = &ctx.model.action else {
        return false;
    };
    ctx.each_named(
        |target| matches!(target, Subject::Container(target_id) if target_id == id),
        "This is not the container to recreate",
        |ctx, target| {
            if let Subject::Container(target_id) = target {
                launch_recreate(ctx.state, ctx.actions, target_id, spec, *pull, &mut ctx.out);
            }
        },
    );
    true
}
