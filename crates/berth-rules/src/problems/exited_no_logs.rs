//! ExitedNoLogs: a container stopped with an error code and said nothing.

use berth_core::{ContainerState, Problem, ProblemKind, Severity, Subject};

use super::support::{explain_fix, oom_killed, shown_name, state_error, Ctx};

/// Codes a person causes by stopping a container (`docker stop`, `docker kill`, Ctrl-C), not
/// failures: 130 interrupt, 137 killed, 143 terminated.
const STOPPED_BY_HAND: [i32; 3] = [130, 137, 143];

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for cf in &ctx.containers {
        let c = &cf.container;
        let Some(code) = c.exit_code else { continue };
        if !matches!(c.state, ContainerState::Exited | ContainerState::Dead)
            || code == 0
            || STOPPED_BY_HAND.contains(&code)
        {
            continue;
        }
        // Silent only when the log is empty and the engine has no word either; those have their
        // own rules (OutOfMemory, PortAllocated, bind mounts ...).
        if !cf.last_lines.is_empty()
            || !cf.error_lines.is_empty()
            || state_error(cf).is_some()
            || oom_killed(cf)
        {
            continue;
        }
        let name = shown_name(c);
        let runs = c
            .command
            .as_deref()
            .filter(|s| !s.is_empty())
            .map_or_else(String::new, |cmd| format!(" It ran: {cmd}."));
        out.push(Problem {
            evidence: ctx.evidence([
                format!("Exited with code {code}"),
                "The log is empty".to_string(),
            ]),
            fixes: vec![explain_fix(format!(
                "Run the command of {name} by hand to see why it stops"
            ))],
            ..ctx.problem(
                ProblemKind::ExitedNoLogs { code },
                Severity::Error,
                Subject::Container(c.id.clone()),
                format!("{name} stopped with code {code} and wrote no log"),
                format!(
                    "{name} ended with exit code {code}, which means it failed, but it printed \
                     nothing first.{runs} A command that exits at once, a wrong entrypoint or a \
                     program that fails quietly do this. Run the same command by hand, or add a \
                     line that prints the reason."
                ),
            )
        });
    }
    out
}
