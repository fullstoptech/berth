//! PortAllocated: a published host port is already held by something else.

use std::sync::LazyLock;

use berth_core::{ContainerSpec, EffectKind, EngineAction, Fix, Problem, ProblemKind, Severity};

use super::support::{
    action_fix, caps, compose_rel, compose_services, edit_fix, edit_plan, effect, explain_fix,
    line, re, Ctx, Re, Source,
};
use crate::{compose_edit, ports};

static BIND_FAILED: Re =
    LazyLock::new(|| re(r"Bind for \S*:(\d+) failed: port is already allocated"));
static EXPOSING: Re = LazyLock::new(|| re(r"exposing port (?:TCP|UDP|SCTP) \S*:(\d+) ->"));
static LISTEN: Re =
    LazyLock::new(|| re(r"listen (?:tcp|udp)[46]? \S*:(\d+): bind: address already in use"));

/// The host port a line says is taken.
fn port_in(text: &str) -> Option<u16> {
    [&BIND_FAILED, &EXPOSING, &LISTEN]
        .into_iter()
        .find_map(|re| caps(re, text))
        .and_then(|c| c.get(1)?.as_str().parse().ok())
}

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    for source in ctx.sources() {
        let mut found: Vec<(u16, &str)> = Vec::new();
        for text in &source.texts {
            if let Some(port) = port_in(text) {
                if !found.iter().any(|(p, _)| *p == port) {
                    found.push((port, text));
                }
            }
        }
        for (port, text) in found {
            out.push(problem(ctx, &source, port, text));
        }
    }
    out
}

fn problem(ctx: &Ctx<'_>, source: &Source<'_>, port: u16, text: &str) -> Problem {
    let compose_text = source.project.and_then(|p| p.compose_text.as_deref());
    let defs = compose_text.map(compose_services).unwrap_or_default();

    // Free means free on this Mac and free among the ports this Compose file publishes.
    let mut taken = ctx.input.taken_ports.clone();
    taken.push(port);
    taken.extend(
        defs.iter()
            .flat_map(|s| s.ports.iter().filter_map(|p| p.host)),
    );
    let suggested = ports::suggest(port, &taken);

    // The service that asked for the port: the container's own, else the one whose compose entry publishes it.
    let service = source
        .container
        .and_then(|c| c.container.service.clone())
        .or_else(|| {
            defs.iter()
                .find(|s| s.ports.iter().any(|p| p.host == Some(port)))
                .map(|s| s.name.clone())
        });
    let who = service.clone().unwrap_or_else(|| "A container".to_string());

    // Another container Berth knows about may be the one holding it.
    let holder = ctx.containers.iter().find(|c| {
        Some(&c.container.id) != source.container.map(|s| &s.container.id)
            && c.container.state == berth_core::ContainerState::Running
            && c.container.ports.iter().any(|p| p.host_port == Some(port))
    });
    let held_by = holder.map_or_else(
        || "Another program on this Mac, or another container, already uses it.".to_string(),
        |h| {
            format!(
                "The container {} already publishes it.",
                super::support::shown_name(&h.container)
            )
        },
    );

    let kind = ProblemKind::PortAllocated { port };
    let mut fixes: Vec<Fix> = Vec::new();
    if let Some(fix) = compose_fix(source, &kind, service.as_deref(), port, suggested) {
        fixes.push(fix);
    } else if let Some(fix) = recreate_fix(ctx, source, port, suggested) {
        fixes.push(fix);
    }
    if fixes.is_empty() {
        fixes.push(explain_fix(format!(
            "Stop what uses port {port}, or publish {who} on another port"
        )));
    }

    Problem {
        evidence: ctx.evidence([text]),
        fixes,
        ..ctx.problem(
            kind,
            Severity::Error,
            source.subject.clone(),
            format!("Port {port} is already in use"),
            format!(
                "{who} wants to publish port {port} on this Mac, but it is taken. {held_by} \
                 Publish {who} on a free port such as {suggested}, or free port {port} first."
            ),
        )
    }
}

fn compose_fix(
    source: &Source<'_>,
    kind: &ProblemKind,
    service: Option<&str>,
    port: u16,
    suggested: u16,
) -> Option<Fix> {
    let project_facts = source.project?;
    let service = service?;
    let text = project_facts.compose_text.as_deref()?;
    let rel = compose_rel(&project_facts.project)?;
    let def = compose_services(text)
        .into_iter()
        .find(|s| s.name == service)?;
    let container_port = def.ports.iter().find(|p| p.host == Some(port))?.container;
    let new_text = compose_edit::set_host_port(text, service, container_port, suggested).ok()?;
    let plan = edit_plan(
        &project_facts.project,
        kind,
        &rel,
        Some(text),
        &new_text,
        &[service],
    )?;
    Some(edit_fix(format!("Map {service} to port {suggested}"), plan))
}

/// A container with no Compose file behind it: recreate it with the host port changed.
fn recreate_fix(ctx: &Ctx<'_>, source: &Source<'_>, port: u16, suggested: u16) -> Option<Fix> {
    let cf = source.container?;
    let detail = cf.detail.as_ref()?;
    let mut spec = ContainerSpec::from_detail(detail);
    let mut changed = false;
    for p in &mut spec.ports {
        if p.host_port == Some(port) {
            p.host_port = Some(suggested);
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    let name = super::support::shown_name(&cf.container).to_string();
    let summary = effect(
        "Recreate",
        "container",
        "containers",
        1,
        vec![
            line(
                EffectKind::Changes,
                format!("{name} publishes port {suggested} instead of {port}"),
            ),
            line(EffectKind::Stays, "Its image, volumes and settings"),
        ],
    );
    Some(action_fix(
        format!("Recreate {name} on port {suggested}"),
        EngineAction::Recreate {
            engine: ctx.input.engine.id.clone(),
            id: cf.container.id.clone(),
            spec,
            pull: false,
        },
        summary,
    ))
}
