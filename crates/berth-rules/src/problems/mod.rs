//! Problem rules, one module per `ProblemKind` (D35, D82).
//! Each rule is tested against `fixtures/problems/<kind>/`.
//!
//! `evaluate` is pure: it reads the facts it is given and the clock in `ProblemInput::now`, and
//! returns problems with plain-language explanations and ready fixes. It never touches the
//! engine, the disk or the network. Env values never reach a title, explanation, evidence line
//! or fix label (they are masked in evidence).

mod bind_missing;
mod bind_permission;
mod build_failed;
mod dependency_failed;
mod disk_full;
mod env_missing;
mod exec_format;
mod exited_no_logs;
mod image_not_found;
mod name_in_use;
mod network_not_found;
mod oom;
mod port_allocated;
mod socket_missing;
mod support;
#[cfg(test)]
mod testkit;
#[cfg(test)]
mod tests;
mod unhealthy;

#[derive(Debug, Clone)]
pub struct ContainerFacts {
    pub container: berth_core::Container,
    pub detail: Option<berth_core::ContainerDetail>,
    pub error_lines: Vec<String>,
    pub last_lines: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProjectFacts {
    pub project: berth_core::Project,
    pub compose_text: Option<String>,
    /// Replaces env_keys: the EnvVarMissing fix hashes and diffs the old .env.
    pub env_text: Option<String>,
    pub compose_error: Option<String>,
    /// <= 20 error lines of the last failed compose command (D82): port clash, name in use, network, build,
    /// dependency, pull denied.
    pub failure_lines: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProblemInput {
    pub now: chrono::DateTime<chrono::Utc>,
    /// One engine per call; the service calls evaluate per engine (D27).
    pub engine: berth_core::Engine,
    /// The Mac's arch (exec_format compares it with the image's).
    pub host_arch: berth_core::Arch,
    pub containers: Vec<ContainerFacts>,
    pub projects: Vec<ProjectFacts>,
    pub disk: Option<berth_core::DiskUsage>,
    pub taken_ports: Vec<u16>,
}

/// Runs the 15 rules over one engine's facts (D27: containers and projects of other engines are
/// ignored). One problem per `ProblemId`; a project-level problem is dropped when a container of
/// that project already carries the same kind.
#[must_use]
pub fn evaluate(input: &ProblemInput) -> Vec<berth_core::Problem> {
    let ctx = support::Ctx::new(input);
    let mut all: Vec<berth_core::Problem> = Vec::new();
    all.extend(port_allocated::check(&ctx));
    all.extend(image_not_found::check(&ctx));
    all.extend(exec_format::check(&ctx));
    all.extend(oom::check(&ctx));
    all.extend(exited_no_logs::check(&ctx));
    all.extend(unhealthy::check(&ctx));
    all.extend(bind_missing::check(&ctx));
    all.extend(bind_permission::check(&ctx));
    all.extend(disk_full::check(&ctx));
    all.extend(socket_missing::check(&ctx));
    all.extend(network_not_found::check(&ctx));
    all.extend(name_in_use::check(&ctx));
    all.extend(env_missing::check(&ctx));
    all.extend(dependency_failed::check(&ctx));
    all.extend(build_failed::check(&ctx));
    dedupe(&ctx, all)
}

fn dedupe(ctx: &support::Ctx<'_>, problems: Vec<berth_core::Problem>) -> Vec<berth_core::Problem> {
    use berth_core::Subject;
    let covered: Vec<(berth_core::ProjectId, berth_core::ProblemKind)> = problems
        .iter()
        .filter_map(|p| match &p.subject {
            Subject::Container(id) => {
                let cf = ctx.containers.iter().find(|c| &c.container.id == id)?;
                Some((cf.container.project.clone()?, p.kind.clone()))
            }
            _ => None,
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    problems
        .into_iter()
        .filter(|p| match &p.subject {
            Subject::Project(id) => !covered.contains(&(id.clone(), p.kind.clone())),
            _ => true,
        })
        .filter(|p| seen.insert(p.id.clone()))
        .collect()
}
