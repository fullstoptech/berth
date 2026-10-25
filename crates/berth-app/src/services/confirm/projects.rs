//! Project confirms: a Compose command a problem's fix proposes (P5-T8), such as "Restart db" for
//! a service another one depends on. The dialog's button, text and result come from the fix's own
//! `EffectSummary`; this file builds the model and starts the confirmed command.

use berth_core::{ComposeCommand, ComposeTarget, Project};

use super::*;

/// The dialog of a fix that runs one Compose command against `project`. The one target is the
/// project, so the runner acts on it and nothing else.
#[must_use]
pub fn compose_fix_model(
    project: &Project,
    title: String,
    effect: EffectSummary,
    target: ComposeTarget,
    command: ComposeCommand,
) -> ConfirmModel {
    ConfirmModel {
        title,
        effect,
        action: EngineAction::Compose { target, command },
        checkbox: None,
        targets: vec![Subject::Project(project.id.clone())],
    }
}

/// The project the model's Compose target names, if it is still there and is the project the
/// fix was made for (same engine, same Compose name).
fn project_of(
    state: &AppState,
    id: &berth_core::ProjectId,
    target: &ComposeTarget,
) -> Option<Project> {
    state
        .projects
        .peek()
        .iter()
        .find(|p| p.id == *id && p.engine == target.engine && p.name == target.project_name)
        .cloned()
}

/// A Compose command from a problem's fix. The target is the project the fix names (AD2: the
/// model acts on the resources its action names).
pub(super) fn run(ctx: &mut Run<'_>) -> bool {
    let EngineAction::Compose { target, command } = &ctx.model.action else {
        return false;
    };
    ctx.each_named(
        |subject| matches!(subject, Subject::Project(_)),
        "This is not a project",
        |ctx, subject| {
            let Subject::Project(id) = subject else {
                return;
            };
            let Some(project) = project_of(ctx.state, id, target) else {
                ctx.out
                    .refused
                    .push("This is not the project the fix is for".to_string());
                return;
            };
            match ctx
                .actions
                .project_command(ctx.state, &project, command.clone())
            {
                Ok(()) => ctx.out.keys.push(PendingKey {
                    subject: Subject::Project(project.id.clone()),
                    action: crate::services::compose::command_action_id(command),
                }),
                Err(reason) => ctx.out.refused.push(reason),
            }
        },
    );
    true
}
