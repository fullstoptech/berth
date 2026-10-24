use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::engine::Arch;
use crate::ids::{ContainerId, EngineId, ProjectId};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ImageRef {
    pub reference: String,
    pub id: Option<String>,
}
impl ImageRef {
    #[must_use]
    pub fn repo(&self) -> &str {
        let no_digest = self.reference.split('@').next().unwrap_or(&self.reference);
        let after_slash = no_digest.rfind('/').map_or(0, |i| i + 1);
        match no_digest[after_slash..].find(':') {
            Some(i) => &no_digest[..after_slash + i],
            None => no_digest,
        }
    }

    #[must_use]
    pub fn tag(&self) -> Option<&str> {
        let no_digest = self.reference.split('@').next().unwrap_or(&self.reference);
        let after_slash = no_digest.rfind('/').map_or(0, |i| i + 1);
        no_digest[after_slash..]
            .find(':')
            .map(|i| &no_digest[after_slash + i + 1..])
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Container {
    pub id: ContainerId,
    pub engine: EngineId,
    pub name: String,
    pub project: Option<ProjectId>,
    pub compose_project: Option<String>,
    pub service: Option<String>,
    pub image: ImageRef,
    pub command: Option<String>,
    pub state: ContainerState,
    pub health: Option<Health>,
    pub exit_code: Option<i32>,
    pub restart_count: u32,
    pub created: DateTime<Utc>,
    pub started: Option<DateTime<Utc>>,
    pub ports: Vec<PortMapping>,
    pub mounts: Vec<Mount>,
    pub networks: Vec<String>,
    pub labels: std::collections::BTreeMap<String, String>,
    pub stats: Option<LiveStats>,
    pub platform: Option<Arch>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContainerState {
    Created,
    Running,
    Paused,
    Restarting,
    Removing,
    Exited,
    Dead,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Health {
    Starting,
    Healthy,
    Unhealthy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Proto {
    Tcp,
    Udp,
    Sctp,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortMapping {
    pub host_ip: Option<std::net::IpAddr>,
    pub host_port: Option<u16>,
    pub container_port: u16,
    pub proto: Proto,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MountKind {
    Bind,
    Volume,
    Tmpfs,
    Other,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    pub kind: MountKind,
    pub source: String,
    pub destination: String,
    pub read_only: bool,
    pub volume_name: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveStats {
    pub cpu_pct: f32,
    pub mem_used: ByteSize,
    pub mem_limit: ByteSize,
    pub net_rx: ByteSize,
    pub net_tx: ByteSize,
    pub at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub start: DateTime<Utc>,
    pub exit_code: i32,
    pub output: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerDetail {
    pub container: Container,
    pub entrypoint: Option<Vec<String>>,
    pub cmd: Option<Vec<String>>,
    pub env: Vec<EnvVar>,
    pub working_dir: Option<String>,
    pub user: Option<String>,
    pub restart: RestartPolicy,
    pub limits: Limits,
    pub health_log: Vec<HealthCheckResult>,
    pub inspect: serde_json::Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerAction {
    Start,
    Stop { timeout_s: Option<u32> },
    Restart { timeout_s: Option<u32> },
    Pause,
    Unpause,
    Kill { signal: Option<String> },
    Remove { force: bool, volumes: bool },
    Rename { name: String },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerSpec {
    pub name: String,
    pub image: ImageRef,
    pub command: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    pub env: Vec<(String, String)>,
    pub ports: Vec<PortMapping>,
    pub mounts: Vec<Mount>,
    pub networks: Vec<String>,
    pub labels: std::collections::BTreeMap<String, String>,
    pub restart: RestartPolicy,
    pub limits: Limits,
    pub platform: Option<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    pub cpus: Option<f32>,
    pub memory: Option<ByteSize>,
    pub memory_swap: Option<ByteSize>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RestartPolicy {
    No,
    Always,
    UnlessStopped,
    OnFailure { max_retries: u16 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecChange {
    pub field: String,
    pub before: String,
    pub after: String,
}

fn show_strings(v: Option<&Vec<String>>) -> String {
    v.map_or_else(|| "—".to_string(), |xs| xs.join(" "))
}
fn show_env(env: &[(String, String)]) -> String {
    env.iter()
        .map(|(k, _)| format!("{k}=••••"))
        .collect::<Vec<_>>()
        .join(", ")
}
fn show_ports(ports: &[PortMapping]) -> String {
    ports
        .iter()
        .map(crate::fmt::port)
        .collect::<Vec<_>>()
        .join(", ")
}
fn show_mounts(mounts: &[Mount]) -> String {
    mounts
        .iter()
        .map(|m| {
            format!(
                "{}:{}{}",
                m.source,
                m.destination,
                if m.read_only { ":ro" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}
fn show_restart(r: RestartPolicy) -> String {
    match r {
        RestartPolicy::No => "no".to_string(),
        RestartPolicy::Always => "always".to_string(),
        RestartPolicy::UnlessStopped => "unless-stopped".to_string(),
        RestartPolicy::OnFailure { max_retries } => format!("on-failure:{max_retries}"),
    }
}
fn show_limits(l: &Limits) -> String {
    let mem = |b: Option<ByteSize>| b.map_or_else(|| "—".to_string(), crate::fmt::bytes);
    let cpus = l.cpus.map_or_else(|| "—".to_string(), |c| format!("{c}"));
    format!(
        "cpus={cpus} memory={} memory_swap={}",
        mem(l.memory),
        mem(l.memory_swap)
    )
}

#[must_use]
pub fn spec_diff(old: &ContainerSpec, new: &ContainerSpec) -> Vec<SpecChange> {
    let mut out = Vec::new();
    let mut push = |field: &str, before: String, after: String, changed: bool| {
        if changed {
            out.push(SpecChange {
                field: field.to_string(),
                before,
                after,
            });
        }
    };
    push(
        "name",
        old.name.clone(),
        new.name.clone(),
        old.name != new.name,
    );
    push(
        "image",
        old.image.reference.clone(),
        new.image.reference.clone(),
        old.image != new.image,
    );
    push(
        "command",
        show_strings(old.command.as_ref()),
        show_strings(new.command.as_ref()),
        old.command != new.command,
    );
    push(
        "entrypoint",
        show_strings(old.entrypoint.as_ref()),
        show_strings(new.entrypoint.as_ref()),
        old.entrypoint != new.entrypoint,
    );
    push(
        "env",
        show_env(&old.env),
        show_env(&new.env),
        old.env != new.env,
    );
    push(
        "ports",
        show_ports(&old.ports),
        show_ports(&new.ports),
        old.ports != new.ports,
    );
    push(
        "mounts",
        show_mounts(&old.mounts),
        show_mounts(&new.mounts),
        old.mounts != new.mounts,
    );
    push(
        "networks",
        old.networks.join(", "),
        new.networks.join(", "),
        old.networks != new.networks,
    );
    let show_labels = |m: &std::collections::BTreeMap<String, String>| {
        m.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    push(
        "labels",
        show_labels(&old.labels),
        show_labels(&new.labels),
        old.labels != new.labels,
    );
    push(
        "restart",
        show_restart(old.restart),
        show_restart(new.restart),
        old.restart != new.restart,
    );
    push(
        "limits",
        show_limits(&old.limits),
        show_limits(&new.limits),
        old.limits != new.limits,
    );
    push(
        "platform",
        old.platform.clone().unwrap_or_else(|| "—".to_string()),
        new.platform.clone().unwrap_or_else(|| "—".to_string()),
        old.platform != new.platform,
    );
    out
}

#[must_use]
pub fn needs_recreate(old: &ContainerSpec, new: &ContainerSpec) -> bool {
    spec_diff(old, new)
        .iter()
        .any(|c| c.field != "limits" && c.field != "restart")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecRequest {
    pub shell: Option<String>,
    pub user: Option<String>,
    pub workdir: Option<String>,
    pub cols: u16,
    pub rows: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecSession {
    pub id: String,
    pub container: ContainerId,
    pub shell: String,
    pub cols: u16,
    pub rows: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ContainerSpec {
        ContainerSpec {
            name: "app".to_string(),
            image: ImageRef {
                reference: "postgres:17".to_string(),
                id: None,
            },
            command: None,
            entrypoint: None,
            env: vec![("PASSWORD".to_string(), "secret".to_string())],
            ports: vec![],
            mounts: vec![],
            networks: vec![],
            labels: std::collections::BTreeMap::new(),
            restart: RestartPolicy::No,
            limits: Limits::default(),
            platform: None,
        }
    }

    #[test]
    fn image_ref_repo_and_tag() {
        let r = ImageRef {
            reference: "postgres:17-alpine".to_string(),
            id: None,
        };
        assert_eq!(r.repo(), "postgres");
        assert_eq!(r.tag(), Some("17-alpine"));
        let plain = ImageRef {
            reference: "library/redis".to_string(),
            id: None,
        };
        assert_eq!(plain.repo(), "library/redis");
        assert_eq!(plain.tag(), None);
    }

    #[test]
    fn image_ref_with_registry_port_and_digest() {
        let r = ImageRef {
            reference: "localhost:5000/app:v2".to_string(),
            id: None,
        };
        assert_eq!(r.repo(), "localhost:5000/app");
        assert_eq!(r.tag(), Some("v2"));
        let d = ImageRef {
            reference: "app@sha256:abc123".to_string(),
            id: None,
        };
        assert_eq!(d.repo(), "app");
        assert_eq!(d.tag(), None);
    }

    #[test]
    fn spec_diff_empty_for_identical_specs() {
        assert_eq!(spec_diff(&spec(), &spec()), [] as [SpecChange; 0]);
        assert!(!needs_recreate(&spec(), &spec()));
    }

    #[test]
    fn spec_diff_masks_env_values_but_detects_value_change() {
        let mut changed = spec();
        changed.env = vec![("PASSWORD".to_string(), "other-secret".to_string())];
        let diff = spec_diff(&spec(), &changed);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].field, "env");
        assert!(!diff[0].before.contains("secret"), "{}", diff[0].before);
        assert!(!diff[0].after.contains("other-secret"));
        assert_eq!(diff[0].before, "PASSWORD=••••");
        assert!(needs_recreate(&spec(), &changed));
    }

    #[test]
    fn only_limits_or_restart_change_needs_no_recreate() {
        let mut changed = spec();
        changed.limits.cpus = Some(2.0);
        changed.restart = RestartPolicy::Always;
        let diff = spec_diff(&spec(), &changed);
        assert_eq!(diff.len(), 2);
        assert!(!needs_recreate(&spec(), &changed));
    }

    #[test]
    fn image_change_needs_recreate() {
        let mut changed = spec();
        changed.image.reference = "postgres:16".to_string();
        assert!(needs_recreate(&spec(), &changed));
        assert_eq!(spec_diff(&spec(), &changed)[0].after, "postgres:16");
    }
}
