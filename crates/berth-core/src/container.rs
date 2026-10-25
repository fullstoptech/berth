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
    // AD16: how the container runs, kept across a recreate.
    pub working_dir: Option<String>,
    pub user: Option<String>,
    pub healthcheck: Option<HealthCheck>,
    pub cap_add: Vec<String>,
    pub cap_drop: Vec<String>,
    /// Network name to the aliases the container answers to there.
    pub network_aliases: std::collections::BTreeMap<String, Vec<String>>,
}
/// A container health check; durations in whole seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthCheck {
    pub test: Vec<String>,
    pub interval_s: Option<u64>,
    pub timeout_s: Option<u64>,
    pub retries: Option<u32>,
    pub start_period_s: Option<u64>,
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

fn json_strings(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn nanos_to_seconds(value: Option<&serde_json::Value>) -> Option<u64> {
    value
        .and_then(serde_json::Value::as_u64)
        .filter(|ns| *ns > 0)
        .map(|ns| ns.div_ceil(1_000_000_000))
}

impl ContainerSpec {
    /// The spec a recreate starts from, read from a container's detail:
    /// the list row and the typed fields where the detail has them, the raw
    /// inspect for the rest. Env values come from `detail.env` (the inspect
    /// is masked). The command and entrypoint are the container's own, the
    /// image's defaults included, so a recreate keeps what runs.
    #[must_use]
    pub fn from_detail(detail: &ContainerDetail) -> Self {
        let c = &detail.container;
        let non_empty = |v: &Option<Vec<String>>| v.clone().filter(|v| !v.is_empty());
        let platform = match &c.platform {
            Some(arch @ (Arch::Aarch64 | Arch::X86_64)) => {
                Some(format!("linux/{}", arch.docker_name()))
            }
            _ => None,
        };
        let inspect = &detail.inspect;
        let healthcheck = inspect.pointer("/Config/Healthcheck").and_then(|h| {
            let test = json_strings(h.get("Test"));
            (!test.is_empty()).then(|| HealthCheck {
                test,
                interval_s: nanos_to_seconds(h.get("Interval")),
                timeout_s: nanos_to_seconds(h.get("Timeout")),
                retries: h
                    .get("Retries")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|n| *n > 0)
                    .and_then(|n| u32::try_from(n).ok()),
                start_period_s: nanos_to_seconds(h.get("StartPeriod")),
            })
        });
        let short_id: String = c.id.0.chars().take(12).collect();
        let mut network_aliases = std::collections::BTreeMap::new();
        if let Some(networks) = inspect
            .pointer("/NetworkSettings/Networks")
            .and_then(serde_json::Value::as_object)
        {
            for (name, endpoint) in networks {
                // Docker adds the container's short id itself.
                let aliases: Vec<String> = json_strings(endpoint.get("Aliases"))
                    .into_iter()
                    .filter(|alias| *alias != short_id)
                    .collect();
                if !aliases.is_empty() {
                    network_aliases.insert(name.clone(), aliases);
                }
            }
        }
        Self {
            name: c.name.clone(),
            image: c.image.clone(),
            command: non_empty(&detail.cmd),
            entrypoint: non_empty(&detail.entrypoint),
            env: detail
                .env
                .iter()
                .map(|var| (var.key.clone(), var.value.clone()))
                .collect(),
            ports: published_ports(detail),
            mounts: c.mounts.clone(),
            networks: c.networks.clone(),
            labels: c.labels.clone(),
            restart: detail.restart,
            limits: detail.limits.clone(),
            platform,
            working_dir: detail.working_dir.clone().filter(|s| !s.is_empty()),
            user: detail.user.clone().filter(|s| !s.is_empty()),
            healthcheck,
            cap_add: json_strings(inspect.pointer("/HostConfig/CapAdd")),
            cap_drop: json_strings(inspect.pointer("/HostConfig/CapDrop")),
            network_aliases,
        }
    }

    /// Run settings the container has that this spec still cannot carry. A
    /// recreate would drop them, so the sheet disables Recreate and names
    /// them (AD16). Read from the raw inspect; empty for a plain container.
    ///
    /// Not listed: the stop signal and the health check (the image's
    /// defaults are merged into the inspect, so a plain container always
    /// shows them).
    #[must_use]
    pub fn uncarried(detail: &ContainerDetail) -> Vec<String> {
        let inspect = &detail.inspect;
        let host = |name: &str| inspect.pointer(&format!("/HostConfig/{name}"));
        let list = |name: &str| {
            host(name)
                .and_then(serde_json::Value::as_array)
                .is_some_and(|a| !a.is_empty())
        };
        let map = |name: &str| {
            host(name)
                .and_then(serde_json::Value::as_object)
                .is_some_and(|a| !a.is_empty())
        };
        let text = |name: &str| host(name).and_then(serde_json::Value::as_str).unwrap_or("");
        let flag = |name: &str| host(name).and_then(serde_json::Value::as_bool) == Some(true);
        let number = |name: &str| host(name).and_then(serde_json::Value::as_i64).unwrap_or(0);
        let mut out = Vec::new();
        let mut note = |condition: bool, label: &str| {
            if condition {
                out.push(label.to_string());
            }
        };
        note(flag("Privileged"), "privileged mode");
        note(list("Devices"), "devices");
        note(list("DeviceCgroupRules"), "device cgroup rules");
        note(map("Sysctls"), "sysctls");
        note(list("Ulimits"), "ulimits");
        note(list("SecurityOpt"), "security options");
        note(list("ExtraHosts"), "extra hosts");
        note(
            list("Dns") || list("DnsSearch") || list("DnsOptions"),
            "DNS settings",
        );
        note(map("Tmpfs"), "tmpfs mounts");
        note(flag("Init"), "init process");
        note(!text("PidMode").is_empty(), "PID mode");
        note(
            !matches!(text("IpcMode"), "" | "private" | "shareable"),
            "IPC mode",
        );
        note(!text("UsernsMode").is_empty(), "user namespace");
        note(
            matches!(text("NetworkMode"), "host" | "none")
                || text("NetworkMode").starts_with("container:"),
            "network mode",
        );
        note(!matches!(text("Runtime"), "" | "runc"), "container runtime");
        note(list("VolumesFrom"), "volumes from other containers");
        note(list("Links"), "links");
        note(list("GroupAdd"), "extra groups");
        note(!text("CpusetCpus").is_empty(), "CPU set");
        note(number("CpuShares") > 0, "CPU shares");
        note(number("PidsLimit") > 0, "process limit");
        note(number("MemoryReservation") > 0, "memory reservation");
        note(flag("ReadonlyRootfs"), "read-only root filesystem");
        note(
            inspect
                .pointer("/HostConfig/LogConfig/Type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|t| !matches!(t, "" | "json-file" | "local")),
            "log driver",
        );
        note(
            inspect
                .pointer("/Config/Tty")
                .and_then(serde_json::Value::as_bool)
                == Some(true),
            "terminal (tty)",
        );
        note(
            inspect
                .pointer("/Config/OpenStdin")
                .and_then(serde_json::Value::as_bool)
                == Some(true),
            "open stdin",
        );
        let short_id: String = detail.container.id.0.chars().take(12).collect();
        let hostname = inspect
            .pointer("/Config/Hostname")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        note(
            !hostname.is_empty() && hostname != short_id && !short_id.is_empty(),
            "hostname",
        );
        note(
            inspect
                .pointer("/Config/Domainname")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|d| !d.is_empty()),
            "domain name",
        );
        out
    }
}

/// The published ports of a container: the configured `HostConfig`
/// bindings of the raw inspect when it has them (a stopped container has
/// no live ones), else the live ports that have a host side. Exposed-only
/// ports are left out: an empty binding in a create request would publish
/// them on a random host port.
#[must_use]
pub fn published_ports(detail: &ContainerDetail) -> Vec<PortMapping> {
    let Some(configured) = detail
        .inspect
        .pointer("/HostConfig/PortBindings")
        .and_then(serde_json::Value::as_object)
    else {
        return detail
            .container
            .ports
            .iter()
            .filter(|p| p.host_port.is_some())
            .copied()
            .collect();
    };
    let mut out = Vec::new();
    for (key, bindings) in configured {
        let (port, proto) = key.split_once('/').unwrap_or((key.as_str(), "tcp"));
        let Ok(container_port) = port.parse::<u16>() else {
            continue;
        };
        let proto = match proto {
            "udp" => Proto::Udp,
            "sctp" => Proto::Sctp,
            _ => Proto::Tcp,
        };
        let Some(bindings) = bindings.as_array() else {
            continue;
        };
        for binding in bindings {
            let text = |name: &str| {
                binding
                    .get(name)
                    .and_then(serde_json::Value::as_str)
                    .filter(|s| !s.is_empty())
            };
            out.push(PortMapping {
                host_ip: text("HostIp").and_then(|ip| ip.parse().ok()),
                host_port: text("HostPort").and_then(|p| p.parse().ok()),
                container_port,
                proto,
            });
        }
    }
    out
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
    let mem = |b: Option<ByteSize>| b.map_or_else(|| "—".to_string(), crate::fmt::memory);
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
    out.extend(run_setting_changes(old, new));
    out
}

fn run_setting_changes(old: &ContainerSpec, new: &ContainerSpec) -> Vec<SpecChange> {
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
    let or_dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "—".to_string());
    push(
        "working_dir",
        or_dash(&old.working_dir),
        or_dash(&new.working_dir),
        old.working_dir != new.working_dir,
    );
    push(
        "user",
        or_dash(&old.user),
        or_dash(&new.user),
        old.user != new.user,
    );
    push(
        "healthcheck",
        show_healthcheck(old.healthcheck.as_ref()),
        show_healthcheck(new.healthcheck.as_ref()),
        old.healthcheck != new.healthcheck,
    );
    push(
        "cap_add",
        old.cap_add.join(", "),
        new.cap_add.join(", "),
        old.cap_add != new.cap_add,
    );
    push(
        "cap_drop",
        old.cap_drop.join(", "),
        new.cap_drop.join(", "),
        old.cap_drop != new.cap_drop,
    );
    let show_aliases = |m: &std::collections::BTreeMap<String, Vec<String>>| {
        m.iter()
            .map(|(network, aliases)| format!("{network}: {}", aliases.join(" ")))
            .collect::<Vec<_>>()
            .join(", ")
    };
    push(
        "network_aliases",
        show_aliases(&old.network_aliases),
        show_aliases(&new.network_aliases),
        old.network_aliases != new.network_aliases,
    );
    out
}

fn show_healthcheck(h: Option<&HealthCheck>) -> String {
    let Some(h) = h else {
        return "—".to_string();
    };
    let secs = |name: &str, v: Option<u64>| v.map(|v| format!(" {name}={v}s")).unwrap_or_default();
    let retries = h
        .retries
        .map(|r| format!(" retries={r}"))
        .unwrap_or_default();
    format!(
        "{}{}{}{retries}{}",
        h.test.join(" "),
        secs("interval", h.interval_s),
        secs("timeout", h.timeout_s),
        secs("start_period", h.start_period_s)
    )
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
            working_dir: None,
            user: None,
            healthcheck: None,
            cap_add: vec![],
            cap_drop: vec![],
            network_aliases: std::collections::BTreeMap::new(),
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
