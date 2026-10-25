//! Engine confirms: stop or restart a Colima profile.

use super::*;

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

/// The stop or restart dialog for a Colima engine.
#[must_use]
pub fn engine_model(
    engine: &EngineId,
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
        targets: vec![Subject::Engine(engine.clone())],
    }
}

/// How many container names a "stop" line spells out before "and N more".
const STOP_NAMES: usize = 4;

/// "shop-api-db-1, shop-api-api-1 and 13 more": the containers that stop.
#[must_use]
pub fn stopped_names(names: &[&str]) -> String {
    if names.len() <= STOP_NAMES {
        return match names {
            [] => String::new(),
            [only] => (*only).to_string(),
            [init @ .., last] => format!("{} and {last}", init.join(", ")),
        };
    }
    format!(
        "{} and {} more",
        names[..STOP_NAMES].join(", "),
        names.len() - STOP_NAMES
    )
}

/// The sentence for what a Colima stop does to the containers on the engine.
fn stop_line(running: &[&Container], when: &str) -> EffectLine {
    let names: Vec<&str> = running.iter().map(|c| c.name.as_str()).collect();
    let text = if names.is_empty() {
        "No containers are running.".to_string()
    } else {
        format!(
            "{} {} {when}: {}.",
            plural(names.len(), "running container", "running containers"),
            if names.len() == 1 { "stops" } else { "stop" },
            stopped_names(&names)
        )
    };
    line(EffectKind::Changes, text)
}

/// The confirm for a Colima operation that changes a running engine: Stop,
/// Restart, or Apply (new resources, `changes` one line each). The
/// `Changes` line names the containers that stop. `None` for Start, which
/// stops nothing, and for an Apply that changes nothing.
#[must_use]
pub fn colima_model(
    engine: &berth_core::Engine,
    op: ColimaOp,
    running: &[&Container],
    changes: &[String],
) -> Option<ConfirmModel> {
    let profile = crate::services::colima::profile_of(engine)?.to_string();
    let kept = line(
        EffectKind::Stays,
        "Containers, images and volumes stay on the engine's disk.",
    );
    let (title, effect) = match &op {
        ColimaOp::Start => return None,
        ColimaOp::Stop => (
            format!("Stop engine {}?", engine.name),
            summary(
                "Stop",
                "engine",
                "engines",
                1,
                vec![stop_line(running, "when the engine stops"), kept],
            ),
        ),
        ColimaOp::Restart => (
            format!("Restart engine {}?", engine.name),
            summary(
                "Restart",
                "engine",
                "engines",
                1,
                vec![stop_line(running, "for about 20 s"), kept],
            ),
        ),
        ColimaOp::Apply(_) => {
            if changes.is_empty() {
                return None;
            }
            let mut lines: Vec<EffectLine> = changes
                .iter()
                .map(|change| line(EffectKind::Changes, change.clone()))
                .collect();
            lines.push(stop_line(running, "for about 20 s"));
            lines.push(kept);
            (
                format!("Apply new resources to engine {}?", engine.name),
                summary(
                    "Apply",
                    "resource change",
                    "resource changes",
                    changes.len(),
                    lines,
                ),
            )
        }
    };
    Some(ConfirmModel {
        title,
        effect,
        action: EngineAction::Colima { profile, op },
        checkbox: None,
        targets: vec![Subject::Engine(engine.id.clone())],
    })
}

/// A Colima command. The one target is the engine the profile names.
pub(super) fn run(ctx: &mut Run<'_>) -> bool {
    let EngineAction::Colima { profile, op } = &ctx.model.action else {
        return false;
    };
    let engine = ctx
        .state
        .engines
        .peek()
        .iter()
        .find(|e| crate::services::colima::profile_of(e) == Some(profile.as_str()))
        .cloned();
    let engine = engine.filter(|e| ctx.model.targets == [Subject::Engine(e.id.clone())]);
    match engine {
        Some(engine) => match ctx.actions.engine(ctx.state, &engine, op.clone()) {
            Ok(()) => ctx.out.keys.push(PendingKey {
                subject: Subject::Engine(engine.id.clone()),
                action: crate::services::colima::action_id(op),
            }),
            Err(reason) => ctx.out.refused.push(reason),
        },
        None => ctx.out.refused.push(format!(
            "Engine {profile} is gone, or is not the engine to act on"
        )),
    }
    true
}
