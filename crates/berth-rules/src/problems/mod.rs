//! Problem rules, one module per `ProblemKind` (D35, D82). P5-T7 implements.
//! Each rule is tested against `fixtures/problems/<kind>/`.

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

/// One function per `ProblemKind` (rules/port_allocated.rs ...), each tested against `fixtures/problems/<kind>/`.
// P5-T7 implements
#[must_use]
pub fn evaluate(_input: &ProblemInput) -> Vec<berth_core::Problem> {
    Vec::new()
}
