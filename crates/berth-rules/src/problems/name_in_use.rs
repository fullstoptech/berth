//! NameInUse: another container already has the name a new one asks for.

use std::sync::LazyLock;

use berth_core::{
    ContainerAction, ContainerState, EffectKind, Fix, Problem, ProblemKind, Severity,
};

use super::support::{
    action_fix, caps, container_action, effect, explain_fix, line, re, shown_name, Ctx, Re,
};
use super::ContainerFacts;

static CONFLICT: Re = LazyLock::new(|| {
    re(r#"container name "/?([^"]+)" is already in use by container "([0-9a-f]+)""#)
});

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for source in ctx.sources() {
        let mut seen: Vec<String> = Vec::new();
        for text in &source.texts {
            let Some(c) = caps(&CONFLICT, text) else {
                continue;
            };
            let name = c[1].to_string();
            if seen.contains(&name) {
                continue;
            }
            seen.push(name.clone());
            // The container that holds the name. Gone already: the conflict is resolved and the
            // old message is only history.
            let holder =
                ctx.containers.iter().copied().find(|h| {
                    shown_name(&h.container) == name || h.container.id.0.starts_with(&c[2])
                });
            let Some(holder) = holder else { continue };
            // A container never conflicts with itself (its own failed start repeats the message).
            if source
                .container
                .is_some_and(|me| me.container.id == holder.container.id)
            {
                continue;
            }
            out.push(Problem {
                evidence: ctx.evidence([text]),
                fixes: fixes(ctx, holder, &name),
                ..ctx.problem(
                    ProblemKind::NameInUse { name: name.clone() },
                    Severity::Error,
                    source.subject.clone(),
                    format!("The name {name} is already taken"),
                    format!(
                        "A container called {name} already exists ({}), so a new one cannot get \
                         that name. Remove the old one, or give it another name. Its volumes \
                         stay either way.",
                        state_word(holder)
                    ),
                )
            });
        }
    }
    out
}

fn state_word(h: &ContainerFacts) -> &'static str {
    match h.container.state {
        ContainerState::Running => "running",
        ContainerState::Paused => "paused",
        ContainerState::Restarting => "restarting",
        ContainerState::Created => "created, never started",
        ContainerState::Removing => "being removed",
        ContainerState::Exited | ContainerState::Dead => "stopped",
    }
}

fn fixes(ctx: &Ctx<'_>, holder: &ContainerFacts, name: &str) -> Vec<Fix> {
    let id = &holder.container.id;
    let active = matches!(
        holder.container.state,
        ContainerState::Running | ContainerState::Paused | ContainerState::Restarting
    );
    let remove = action_fix(
        format!("Remove the old {name}"),
        container_action(
            ctx,
            id,
            ContainerAction::Remove {
                force: active,
                volumes: false,
            },
        ),
        effect(
            "Remove",
            "container",
            "containers",
            1,
            vec![
                line(
                    EffectKind::Goes,
                    format!(
                        "{name}{}",
                        if active { ", which is running now" } else { "" }
                    ),
                ),
                line(EffectKind::Stays, "Its volumes and images"),
            ],
        ),
    );
    let new_name = free_name(ctx, name);
    let rename = action_fix(
        format!("Rename the old one to {new_name}"),
        container_action(
            ctx,
            id,
            ContainerAction::Rename {
                name: new_name.clone(),
            },
        ),
        effect(
            "Rename",
            "container",
            "containers",
            1,
            vec![
                line(EffectKind::Changes, format!("{name} becomes {new_name}")),
                line(EffectKind::Stays, "Everything else about it"),
            ],
        ),
    );
    vec![
        remove,
        rename,
        explain_fix("Keep both: choose another name in the Compose file"),
    ]
}

/// `<name>-old`, or `<name>-old-2` and so on, the first no container has.
fn free_name(ctx: &Ctx<'_>, name: &str) -> String {
    let taken = |candidate: &str| ctx.container_by_name(candidate).is_some();
    let first = format!("{name}-old");
    if !taken(&first) {
        return first;
    }
    (2..)
        .map(|n| format!("{first}-{n}"))
        .find(|c| !taken(c))
        .unwrap_or(first)
}
