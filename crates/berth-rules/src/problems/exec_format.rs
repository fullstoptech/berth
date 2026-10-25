//! ExecFormat: the image was built for another CPU than the engine's.

use std::sync::LazyLock;

use berth_core::{
    Arch, ColimaOp, ColimaStartOptions, EffectKind, EngineAction, EngineKind, Fix, Problem,
    ProblemKind, Severity, VmType,
};

use super::support::{
    action_fix, caps, compose_rel, edit_fix, edit_plan, effect, explain_fix, is_match, line, re,
    shown_name, Ctx, Re,
};
use crate::compose_edit;

static EXEC_FORMAT: Re = LazyLock::new(|| re(r"(?i)exec format error|rosetta error"));
static PLATFORM_WARNING: Re = LazyLock::new(|| {
    re(r"requested image's platform \(linux/(amd64|arm64|arm|386)[^)]*\) does not match")
});

fn nice(arch: &Arch) -> String {
    match arch {
        Arch::Aarch64 => "arm64 (Apple silicon)".to_string(),
        Arch::X86_64 => "x86-64 (Intel)".to_string(),
        Arch::Other(s) => s.clone(),
    }
}

pub(super) fn check(ctx: &Ctx<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    let host = &ctx.input.host_arch;
    for cf in &ctx.containers {
        let texts = super::support::container_texts(cf);
        let Some(hit) = texts.iter().find(|t| is_match(&EXEC_FORMAT, t)) else {
            continue;
        };
        let from_text = texts
            .iter()
            .find_map(|t| caps(&PLATFORM_WARNING, t))
            .map(|c| Arch::parse(&c[1]));
        let image_arch = cf.container.platform.clone().or(from_text);
        // The same CPU: the error comes from the program itself (a script with the wrong line
        // endings or a missing interpreter), not from the architecture.
        if image_arch.as_ref() == Some(host) {
            continue;
        }
        let name = shown_name(&cf.container);
        let kind = ProblemKind::ExecFormat {
            image_arch: image_arch.clone(),
        };
        let mut fixes: Vec<Fix> = Vec::new();
        if let Some(fix) = rosetta_fix(ctx, image_arch.as_ref()) {
            fixes.push(fix);
        } else if let Some(fix) = platform_fix(ctx, cf, &kind) {
            fixes.push(fix);
        }
        if fixes.is_empty() {
            fixes.push(explain_fix(format!(
                "Use an image of {} for {name}",
                nice(host)
            )));
        }
        let image_says = image_arch.as_ref().map_or_else(
            || "a different CPU".to_string(),
            |a| format!("{} processors", nice(a)),
        );
        out.push(Problem {
            evidence: ctx.evidence([hit.as_str()]),
            fixes,
            ..ctx.problem(
                kind,
                Severity::Error,
                berth_core::Subject::Container(cf.container.id.clone()),
                format!("{name} was built for another CPU"),
                format!(
                    "The image of {name} is made for {image_says}, but this engine runs on {}. \
                     The engine cannot run the program inside it. Rosetta lets a Colima machine \
                     run Intel images; otherwise pick an image built for {}.",
                    nice(host),
                    nice(host)
                ),
            )
        });
    }
    out
}

/// "Restart Colima with Rosetta": only on a Colima machine of type vz that does not have it yet,
/// and only when the image is Intel (or unknown) on an Apple silicon Mac.
fn rosetta_fix(ctx: &Ctx<'_>, image_arch: Option<&Arch>) -> Option<Fix> {
    let EngineKind::Colima { profile } = &ctx.input.engine.kind else {
        return None;
    };
    let resources = ctx.input.engine.resources.as_ref()?;
    if resources.vm_type != VmType::Vz || resources.rosetta {
        return None;
    }
    if ctx.input.host_arch != Arch::Aarch64 || image_arch.is_some_and(|a| *a != Arch::X86_64) {
        return None;
    }
    let summary = effect(
        "Restart",
        "engine",
        "engines",
        1,
        vec![
            line(
                EffectKind::Changes,
                format!("Rosetta is turned on for the Colima profile {profile}"),
            ),
            line(
                EffectKind::Goes,
                "Running containers stop while the machine restarts",
            ),
            line(EffectKind::Stays, "Images, volumes and projects"),
        ],
    );
    Some(action_fix(
        "Restart Colima with Rosetta",
        EngineAction::Colima {
            profile: profile.clone(),
            op: ColimaOp::Apply(ColimaStartOptions {
                cpus: None,
                memory_gib: None,
                disk_gib: None,
                vm_type: Some(VmType::Vz),
                rosetta: Some(true),
                arch: None,
            }),
        },
        summary,
    ))
}

/// `platform: linux/<host arch>` on the service in the Compose file.
fn platform_fix(ctx: &Ctx<'_>, cf: &super::ContainerFacts, kind: &ProblemKind) -> Option<Fix> {
    let service = cf.container.service.as_deref()?;
    let project_facts = ctx.project_of(&cf.container)?;
    let text = project_facts.compose_text.as_deref()?;
    let rel = compose_rel(&project_facts.project)?;
    let platform = format!("linux/{}", ctx.input.host_arch.docker_name());
    let new_text = compose_edit::set_service_key(text, service, "platform", &platform).ok()?;
    if new_text == text {
        return None;
    }
    let plan = edit_plan(
        &project_facts.project,
        kind,
        &rel,
        Some(text),
        &new_text,
        &[service],
    )?;
    Some(edit_fix(format!("Run {service} as {platform}"), plan))
}
