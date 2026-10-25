//! Deleting an image, volume or network (`EngineAction::Remove`, D25a).

use super::*;

/// What the activity entry and the refusals call a resource: an image's
/// first tag (else its id), a volume's name, a network's name.
fn resource_name(state: &AppState, target: &ResourceRef) -> Option<String> {
    match target {
        ResourceRef::Image(id) => state
            .images
            .peek()
            .iter()
            .find(|i| i.id == *id)
            .map(|i| i.repo_tags.first().cloned().unwrap_or_else(|| id.clone())),
        ResourceRef::Volume(name) => state
            .volumes
            .peek()
            .iter()
            .find(|v| v.name == *name)
            .map(|v| v.name.clone()),
        ResourceRef::Network(id) => state
            .networks
            .peek()
            .iter()
            .find(|n| n.id == *id)
            .map(|n| n.name.clone()),
    }
}

fn launch_remove(
    state: &mut AppState,
    actions: &ActionsService,
    engine: &EngineId,
    target: &ResourceRef,
    force: bool,
    out: &mut Launched,
) {
    let Some(name) = resource_name(state, target) else {
        out.refused
            .push(format!("This {} no longer exists", target.noun_one()));
        return;
    };
    let key = PendingKey {
        subject: target.subject(),
        action: target.action_id(),
    };
    match actions.remove(state, engine, target, &name, force) {
        Ok(()) => out.keys.push(key),
        Err(reason) => out.refused.push(reason),
    }
}

/// Exactly the resource the action names: same kind, same name or id. A
/// dialog opened for one volume cannot delete another.
pub(super) fn run(ctx: &mut Run<'_>) -> bool {
    let EngineAction::Remove {
        engine,
        target: kind,
        force,
    } = &ctx.model.action
    else {
        return false;
    };
    ctx.each_named(
        |subject| ResourceRef::from_subject(subject).is_some_and(|target| target == *kind),
        &format!("This is not the {} to delete", kind.noun_one()),
        |ctx, subject| {
            if let Some(target) = ResourceRef::from_subject(subject) {
                launch_remove(
                    ctx.state,
                    ctx.actions,
                    engine,
                    &target,
                    *force,
                    &mut ctx.out,
                );
            }
        },
    );
    true
}
