//! EngineSocketMissing: Berth cannot reach the engine of a local profile or socket.

use berth_core::{
    ColimaOp, EffectKind, EngineAction, EngineKind, EngineState, Fix, Problem, ProblemKind,
    Severity, Subject,
};

use super::support::{action_fix, effect, explain_fix, line, Ctx};

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let engine = &ctx.input.engine;
    // A remote engine that does not answer is a network problem; the socket is a local thing.
    if matches!(engine.kind, EngineKind::Remote { .. }) {
        return Vec::new();
    }
    let (severity, state_text, op) = match &engine.state {
        EngineState::Stopped => (
            Severity::Warning,
            "The engine is stopped".to_string(),
            ColimaOp::Start,
        ),
        EngineState::Unreachable { reason, .. } => (
            Severity::Error,
            format!("The engine does not answer: {reason}"),
            ColimaOp::Restart,
        ),
        _ => return Vec::new(),
    };
    let name = &engine.name;
    let fix: Fix = match &engine.kind {
        EngineKind::Colima { profile } => {
            let verb = if matches!(op, ColimaOp::Start) {
                "Start"
            } else {
                "Restart"
            };
            action_fix(
                format!("{verb} {name}"),
                EngineAction::Colima {
                    profile: profile.clone(),
                    op,
                },
                effect(
                    verb,
                    "engine",
                    "engines",
                    1,
                    vec![
                        line(
                            EffectKind::Changes,
                            format!(
                                "The Colima profile {profile} {} up",
                                if verb == "Start" {
                                    "starts"
                                } else {
                                    "restarts and comes back"
                                }
                            ),
                        ),
                        line(EffectKind::Stays, "Images, volumes and projects"),
                    ],
                ),
            )
        }
        _ => explain_fix(format!("Start the engine behind {name}")),
    };
    vec![Problem {
        evidence: ctx.evidence([state_text]),
        fixes: vec![fix],
        ..ctx.problem(
            ProblemKind::EngineSocketMissing,
            severity,
            Subject::Engine(engine.id.clone()),
            format!("Berth cannot reach {name}"),
            format!(
                "Docker listens on a socket file that {name} creates when it runs. The file is \
                 not there or nothing answers behind it, so containers cannot be listed or \
                 started. Start the engine and Berth reconnects by itself."
            ),
        )
    }]
}
