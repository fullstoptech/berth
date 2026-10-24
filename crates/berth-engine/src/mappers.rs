//! Pure mapping from bollard models to core types (PLAN §6.5), tested
//! against `fixtures/docker/*.json` captured from the berthfx fixtures.

use std::collections::{BTreeMap, HashMap};

use bollard::models::{
    ContainerInspectResponse, ContainerStatsResponse, ContainerSummary, EventMessage,
    EventMessageTypeEnum, HealthStatusEnum, ImageSummary, MountPoint, Network,
    SystemDataUsageResponse, Volume,
};
use chrono::{DateTime, Utc};

const MASK: &str = "••••";
const COMPOSE_PROJECT: &str = "com.docker.compose.project";
const COMPOSE_SERVICE: &str = "com.docker.compose.service";

fn labels(map: Option<&HashMap<String, String>>) -> BTreeMap<String, String> {
    map.map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

fn from_unix(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

fn parse_ts(s: Option<&String>) -> Option<DateTime<Utc>> {
    let s = s?;
    if s.starts_with("0001-") {
        return None;
    }
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn non_empty(s: &Option<String>) -> Option<String> {
    s.clone().filter(|v| !v.is_empty())
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn to_u32(v: i64) -> u32 {
    u32::try_from(v).unwrap_or(0)
}

fn state_from_str(s: &str) -> berth_core::ContainerState {
    match s {
        "running" => berth_core::ContainerState::Running,
        "paused" => berth_core::ContainerState::Paused,
        "restarting" => berth_core::ContainerState::Restarting,
        "removing" => berth_core::ContainerState::Removing,
        "dead" => berth_core::ContainerState::Dead,
        "created" => berth_core::ContainerState::Created,
        // "exited", "stopping" and anything unknown read as Exited.
        _ => berth_core::ContainerState::Exited,
    }
}

fn health_from_str(s: &str) -> Option<berth_core::Health> {
    match s {
        "starting" => Some(berth_core::Health::Starting),
        "healthy" => Some(berth_core::Health::Healthy),
        "unhealthy" => Some(berth_core::Health::Unhealthy),
        _ => None,
    }
}

fn health_from_enum(h: Option<&HealthStatusEnum>) -> Option<berth_core::Health> {
    h.and_then(|h| health_from_str(h.as_ref()))
}

/// Docker's summary status for an exited container reads "Exited (1) ...".
fn exit_code_from_status(status: Option<&String>) -> Option<i32> {
    let status = status?;
    let inner = status.strip_prefix("Exited (")?;
    inner.split(')').next()?.parse().ok()
}

fn proto_from_str(s: &str) -> berth_core::Proto {
    match s {
        "udp" => berth_core::Proto::Udp,
        "sctp" => berth_core::Proto::Sctp,
        _ => berth_core::Proto::Tcp,
    }
}

fn mounts(points: Option<&Vec<MountPoint>>) -> Vec<berth_core::Mount> {
    points
        .map(|points| {
            points
                .iter()
                .map(|m| {
                    let kind = match m.typ.as_deref() {
                        Some("bind") => berth_core::MountKind::Bind,
                        Some("volume") => berth_core::MountKind::Volume,
                        Some("tmpfs") => berth_core::MountKind::Tmpfs,
                        _ => berth_core::MountKind::Other,
                    };
                    berth_core::Mount {
                        kind,
                        source: m.source.clone().unwrap_or_default(),
                        destination: m.destination.clone().unwrap_or_default(),
                        read_only: !m.rw.unwrap_or(true),
                        volume_name: m.name.clone(),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn network_names<V>(networks: Option<&HashMap<String, V>>) -> Vec<String> {
    let mut names: Vec<String> = networks
        .map(|n| n.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
}

pub fn container(engine: &berth_core::EngineId, s: &ContainerSummary) -> berth_core::Container {
    let labels = labels(s.labels.as_ref());
    let state = state_from_str(s.state.as_ref().map_or("", AsRef::as_ref));
    let ports = s
        .ports
        .as_ref()
        .map(|ports| {
            ports
                .iter()
                .map(|p| berth_core::PortMapping {
                    host_ip: p
                        .ip
                        .as_ref()
                        .filter(|ip| !ip.is_empty())
                        .and_then(|ip| ip.parse().ok()),
                    host_port: p.public_port,
                    container_port: p.private_port,
                    proto: proto_from_str(p.typ.as_ref().map_or("", AsRef::as_ref)),
                })
                .collect()
        })
        .unwrap_or_default();
    berth_core::Container {
        id: berth_core::ContainerId(s.id.clone().unwrap_or_default()),
        engine: engine.clone(),
        name: s
            .names
            .as_ref()
            .and_then(|n| n.first())
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_default(),
        project: None,
        compose_project: labels.get(COMPOSE_PROJECT).cloned(),
        service: labels.get(COMPOSE_SERVICE).cloned(),
        image: berth_core::ImageRef {
            reference: s.image.clone().unwrap_or_default(),
            id: s.image_id.clone(),
        },
        command: s.command.clone(),
        state,
        health: s
            .health
            .as_ref()
            .and_then(|h| h.status.as_ref())
            .and_then(|h| health_from_str(h.as_ref())),
        exit_code: exit_code_from_status(s.status.as_ref()),
        restart_count: 0,
        created: from_unix(s.created.unwrap_or(0)),
        started: None,
        ports,
        mounts: mounts(s.mounts.as_ref()),
        networks: network_names(
            s.network_settings
                .as_ref()
                .and_then(|n| n.networks.as_ref()),
        ),
        labels,
        stats: None,
        platform: s
            .image_manifest_descriptor
            .as_ref()
            .and_then(|d| d.platform.as_ref())
            .and_then(|p| p.architecture.as_ref())
            .map(|a| berth_core::Arch::parse(a)),
    }
}

fn ports_from_port_map(map: Option<&bollard::models::PortMap>) -> Vec<berth_core::PortMapping> {
    let mut out = Vec::new();
    if let Some(map) = map {
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for key in keys {
            let (port, proto) = match key.split_once('/') {
                Some((p, proto)) => (p, proto),
                None => (key.as_str(), "tcp"),
            };
            let Ok(container_port) = port.parse::<u16>() else {
                continue;
            };
            match map.get(key) {
                Some(Some(bindings)) if !bindings.is_empty() => {
                    for b in bindings {
                        out.push(berth_core::PortMapping {
                            host_ip: b
                                .host_ip
                                .as_ref()
                                .filter(|ip| !ip.is_empty())
                                .and_then(|ip| ip.parse().ok()),
                            host_port: b
                                .host_port
                                .as_ref()
                                .filter(|p| !p.is_empty())
                                .and_then(|p| p.parse().ok()),
                            container_port,
                            proto: proto_from_str(proto),
                        });
                    }
                }
                _ => out.push(berth_core::PortMapping {
                    host_ip: None,
                    host_port: None,
                    container_port,
                    proto: proto_from_str(proto),
                }),
            }
        }
    }
    out
}

fn restart_policy(p: Option<&bollard::models::RestartPolicy>) -> berth_core::RestartPolicy {
    match p.and_then(|p| p.name.as_ref()) {
        Some(bollard::models::RestartPolicyNameEnum::ALWAYS) => berth_core::RestartPolicy::Always,
        Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED) => {
            berth_core::RestartPolicy::UnlessStopped
        }
        Some(bollard::models::RestartPolicyNameEnum::ON_FAILURE) => {
            berth_core::RestartPolicy::OnFailure {
                max_retries: p
                    .and_then(|p| p.maximum_retry_count)
                    .map_or(0, |n| u16::try_from(n).unwrap_or(0)),
            }
        }
        _ => berth_core::RestartPolicy::No,
    }
}

fn limits(host: Option<&bollard::models::HostConfig>) -> berth_core::Limits {
    let Some(host) = host else {
        return berth_core::Limits::default();
    };
    let cpus = match host.nano_cpus {
        Some(n) if n > 0 => Some(n as f32 / 1_000_000_000.0),
        _ => match (host.cpu_quota, host.cpu_period) {
            (Some(q), Some(p)) if q > 0 && p > 0 => Some(q as f32 / p as f32),
            _ => None,
        },
    };
    let positive = |v: Option<i64>| match v {
        Some(n) if n > 0 => Some(bytesize::ByteSize(to_u64(n))),
        _ => None,
    };
    berth_core::Limits {
        cpus,
        memory: positive(host.memory),
        memory_swap: positive(host.memory_swap),
    }
}

fn parse_env(env: Option<&Vec<String>>) -> Vec<berth_core::EnvVar> {
    env.map(|env| {
        env.iter()
            .map(|e| match e.split_once('=') {
                Some((k, v)) => berth_core::EnvVar {
                    key: k.to_string(),
                    value: v.to_string(),
                },
                None => berth_core::EnvVar {
                    key: e.clone(),
                    value: String::new(),
                },
            })
            .collect()
    })
    .unwrap_or_default()
}

/// Mask every `Env` array's values in a serialized inspect value (CLAUDE rules:
/// env values never leave through `inspect`; `ContainerDetail::env` keeps them).
fn mask_env(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if key == "Env" {
                    if let serde_json::Value::Array(items) = v {
                        for item in items {
                            if let serde_json::Value::String(s) = item {
                                let name = s.split('=').next().unwrap_or("");
                                *s = format!("{name}={MASK}");
                            }
                        }
                    }
                } else {
                    mask_env(v);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                mask_env(item);
            }
        }
        _ => {}
    }
}

pub fn detail(
    engine: &berth_core::EngineId,
    i: &ContainerInspectResponse,
) -> berth_core::ContainerDetail {
    let config = i.config.as_ref();
    let state = i.state.as_ref();
    let labels = labels(config.and_then(|c| c.labels.as_ref()));
    let image_id = i.image.clone();
    let reference = config
        .and_then(|c| non_empty(&c.image))
        .or_else(|| image_id.clone())
        .unwrap_or_default();
    let command = match (i.path.as_ref(), i.args.as_ref()) {
        (Some(path), Some(args)) if !args.is_empty() => Some(format!("{path} {}", args.join(" "))),
        (Some(path), _) => Some(path.clone()),
        _ => None,
    };
    let health = state.and_then(|s| s.health.as_ref());
    let container = berth_core::Container {
        id: berth_core::ContainerId(i.id.clone().unwrap_or_default()),
        engine: engine.clone(),
        name: i
            .name
            .as_ref()
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_default(),
        project: None,
        compose_project: labels.get(COMPOSE_PROJECT).cloned(),
        service: labels.get(COMPOSE_SERVICE).cloned(),
        image: berth_core::ImageRef {
            reference,
            id: image_id,
        },
        command,
        state: state_from_str(
            state
                .and_then(|s| s.status.as_ref())
                .map_or("", AsRef::as_ref),
        ),
        health: health_from_enum(health.and_then(|h| h.status.as_ref())),
        exit_code: state
            .and_then(|s| s.exit_code)
            .map(|c| i32::try_from(c).unwrap_or(0)),
        restart_count: i.restart_count.map_or(0, to_u32),
        created: i.created.unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
        started: parse_ts(state.and_then(|s| s.started_at.as_ref())),
        ports: ports_from_port_map(i.network_settings.as_ref().and_then(|n| n.ports.as_ref())),
        mounts: mounts(i.mounts.as_ref()),
        networks: network_names(
            i.network_settings
                .as_ref()
                .and_then(|n| n.networks.as_ref()),
        ),
        labels,
        stats: None,
        platform: i.platform.as_ref().and_then(|p| {
            // Inspect's Platform is the OS ("linux"); the arch lives on the
            // image manifest descriptor when present.
            let _ = p;
            i.image_manifest_descriptor
                .as_ref()
                .and_then(|d| d.platform.as_ref())
                .and_then(|pl| pl.architecture.as_ref())
                .map(|a| berth_core::Arch::parse(a))
        }),
    };
    let health_log = health
        .and_then(|h| h.log.as_ref())
        .map(|log| {
            log.iter()
                .map(|r| berth_core::HealthCheckResult {
                    start: r.start.unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
                    exit_code: r.exit_code.map_or(0, |c| i32::try_from(c).unwrap_or(0)),
                    output: r.output.clone().unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut inspect = serde_json::to_value(i).unwrap_or(serde_json::Value::Null);
    mask_env(&mut inspect);
    berth_core::ContainerDetail {
        container,
        entrypoint: config.and_then(|c| c.entrypoint.clone()),
        cmd: config.and_then(|c| c.cmd.clone()),
        env: parse_env(config.and_then(|c| c.env.as_ref())),
        working_dir: config.and_then(|c| non_empty(&c.working_dir)),
        user: config.and_then(|c| non_empty(&c.user)),
        restart: restart_policy(
            i.host_config
                .as_ref()
                .and_then(|h| h.restart_policy.as_ref()),
        ),
        limits: limits(i.host_config.as_ref()),
        health_log,
        inspect,
    }
}

fn used_by_image(
    image_id: &str,
    repo_tags: &[String],
    used_by: &[berth_core::Container],
) -> Vec<berth_core::ContainerId> {
    used_by
        .iter()
        .filter(|c| {
            c.image.id.as_deref() == Some(image_id)
                || repo_tags.iter().any(|t| t == &c.image.reference)
        })
        .map(|c| c.id.clone())
        .collect()
}

pub fn image(s: &ImageSummary, used_by: &[berth_core::Container]) -> berth_core::Image {
    let repo_tags: Vec<String> = s
        .repo_tags
        .iter()
        .filter(|t| *t != "<none>:<none>")
        .cloned()
        .collect();
    let digests: Vec<String> = s
        .repo_digests
        .iter()
        .filter(|t| *t != "<none>@<none>")
        .cloned()
        .collect();
    let platform = s.manifests.as_ref().and_then(|manifests| {
        manifests.iter().find_map(|m| {
            m.image_data.as_ref().and_then(|d| {
                d.platform.architecture.as_ref().map(|arch| {
                    (
                        berth_core::Arch::parse(arch),
                        d.platform.os.clone().unwrap_or_default(),
                    )
                })
            })
        })
    });
    let (arch, os) = platform.unwrap_or_else(|| {
        (
            berth_core::Arch::Other("unknown".to_string()),
            String::new(),
        )
    });
    berth_core::Image {
        id: s.id.clone(),
        dangling: repo_tags.is_empty(),
        used_by: used_by_image(&s.id, &repo_tags, used_by),
        repo_tags,
        digests,
        size: bytesize::ByteSize(to_u64(s.size)),
        created: from_unix(s.created),
        arch,
        os,
    }
}

/// Image built from an inspect response (used by `EngineClient::image`).
pub(crate) fn image_from_inspect(
    i: &bollard::models::ImageInspect,
    used_by: &[berth_core::Container],
) -> berth_core::Image {
    let repo_tags: Vec<String> = i
        .repo_tags
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t != "<none>:<none>")
        .collect();
    let digests: Vec<String> = i
        .repo_digests
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t != "<none>@<none>")
        .collect();
    let id = i.id.clone().unwrap_or_default();
    berth_core::Image {
        dangling: repo_tags.is_empty(),
        used_by: used_by_image(&id, &repo_tags, used_by),
        id,
        repo_tags,
        digests,
        size: bytesize::ByteSize(i.size.map_or(0, to_u64)),
        created: i.created.unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
        arch: i.architecture.as_deref().map_or_else(
            || berth_core::Arch::Other("unknown".to_string()),
            berth_core::Arch::parse,
        ),
        os: i.os.clone().unwrap_or_default(),
    }
}

pub fn volume(v: &Volume, used_by: &[berth_core::Container]) -> berth_core::Volume {
    berth_core::Volume {
        name: v.name.clone(),
        driver: v.driver.clone(),
        size: v
            .usage_data
            .as_ref()
            .filter(|u| u.size >= 0)
            .map(|u| bytesize::ByteSize(to_u64(u.size))),
        mountpoint: v.mountpoint.clone(),
        used_by: used_by
            .iter()
            .filter(|c| {
                c.mounts
                    .iter()
                    .any(|m| m.volume_name.as_deref() == Some(v.name.as_str()))
            })
            .map(|c| c.id.clone())
            .collect(),
        project: None,
        created: v.created_at,
        labels: labels(Some(&v.labels)),
    }
}

pub fn network(n: &Network) -> berth_core::Network {
    let name = n.name.clone().unwrap_or_default();
    let first_ipam = n
        .ipam
        .as_ref()
        .and_then(|ipam| ipam.config.as_ref())
        .and_then(|configs| configs.first());
    berth_core::Network {
        id: n.id.clone().unwrap_or_default(),
        built_in: matches!(name.as_str(), "bridge" | "host" | "none"),
        name,
        driver: n.driver.clone().unwrap_or_default(),
        subnet: first_ipam.and_then(|c| c.subnet.clone()),
        gateway: first_ipam.and_then(|c| c.gateway.clone()),
        internal: n.internal.unwrap_or(false),
        // bollard's Network model carries no attached-container list; the
        // engine fills this from the container list (see BollardEngine).
        containers: Vec::new(),
        project: None,
    }
}

pub fn event(engine: &berth_core::EngineId, e: &EventMessage) -> berth_core::EngineEvent {
    let actor = e.actor.as_ref();
    let actor_id = actor.and_then(|a| a.id.clone()).unwrap_or_default();
    let action = e.action.clone().unwrap_or_default();
    let kind = match e.typ {
        Some(EventMessageTypeEnum::CONTAINER) => berth_core::EventKind::Container {
            id: berth_core::ContainerId(actor_id),
            action,
        },
        Some(EventMessageTypeEnum::IMAGE) => berth_core::EventKind::Image {
            id: actor_id,
            action,
        },
        Some(EventMessageTypeEnum::VOLUME) => berth_core::EventKind::Volume {
            name: actor_id,
            action,
        },
        Some(EventMessageTypeEnum::NETWORK) => berth_core::EventKind::Network {
            id: actor_id,
            action,
        },
        _ => berth_core::EventKind::Other {
            ty: e.typ.as_ref().map_or(String::new(), |t| t.to_string()),
            action,
        },
    };
    let at = match e.time_nano {
        Some(nano) if nano != 0 => DateTime::from_timestamp_nanos(nano),
        _ => from_unix(e.time.unwrap_or(0)),
    };
    berth_core::EngineEvent {
        engine: engine.clone(),
        at,
        kind,
        attributes: labels(actor.and_then(|a| a.attributes.as_ref())),
    }
}

/// Docker CLI formula: (cpu_delta / system_delta) * online_cpus * 100.
pub fn cpu_percent(s: &ContainerStatsResponse) -> Option<f32> {
    let cpu = s.cpu_stats.as_ref()?;
    let pre = s.precpu_stats.as_ref()?;
    let cpu_total = cpu.cpu_usage.as_ref()?.total_usage?;
    let pre_total = pre.cpu_usage.as_ref()?.total_usage?;
    let system = cpu.system_cpu_usage?;
    let pre_system = pre.system_cpu_usage?;
    let cpu_delta = cpu_total.checked_sub(pre_total)?;
    let system_delta = system.checked_sub(pre_system)?;
    if system_delta == 0 {
        return None;
    }
    let online = cpu
        .online_cpus
        .map(f64::from)
        .filter(|n| *n > 0.0)
        .or_else(|| {
            cpu.cpu_usage
                .as_ref()
                .and_then(|u| u.percpu_usage.as_ref())
                .map(|p| p.len() as f64)
                .filter(|n| *n > 0.0)
        })
        .unwrap_or(1.0);
    Some((cpu_delta as f64 / system_delta as f64 * online * 100.0) as f32)
}

fn usage_group(
    total_count: Option<i64>,
    total_size: Option<i64>,
    reclaimable: Option<i64>,
) -> berth_core::UsageGroup {
    berth_core::UsageGroup {
        count: total_count.map_or(0, to_u32),
        size: bytesize::ByteSize(total_size.map_or(0, to_u64)),
        reclaimable: bytesize::ByteSize(reclaimable.map_or(0, to_u64)),
    }
}

pub fn disk_usage(d: &SystemDataUsageResponse) -> berth_core::DiskUsage {
    berth_core::DiskUsage {
        images: d.image_usage.as_ref().map_or_else(Default::default, |u| {
            usage_group(u.total_count, u.total_size, u.reclaimable)
        }),
        containers: d
            .container_usage
            .as_ref()
            .map_or_else(Default::default, |u| {
                usage_group(u.total_count, u.total_size, u.reclaimable)
            }),
        volumes: d.volume_usage.as_ref().map_or_else(Default::default, |u| {
            usage_group(u.total_count, u.total_size, u.reclaimable)
        }),
        build_cache: d
            .build_cache_usage
            .as_ref()
            .map_or_else(Default::default, |u| {
                usage_group(u.total_count, u.total_size, u.reclaimable)
            }),
        disk_total: None,
    }
}

/// Volume name -> size, from the `VolumeUsage.Items` of a system df response.
pub(crate) fn volume_sizes(d: &SystemDataUsageResponse) -> BTreeMap<String, bytesize::ByteSize> {
    let mut out = BTreeMap::new();
    if let Some(items) = d.volume_usage.as_ref().and_then(|u| u.items.as_ref()) {
        for item in items {
            let name = item.get("Name").and_then(serde_json::Value::as_str);
            let size = item
                .get("UsageData")
                .and_then(|u| u.get("Size"))
                .and_then(serde_json::Value::as_i64);
            if let (Some(name), Some(size)) = (name, size) {
                if size >= 0 {
                    out.insert(name.to_string(), bytesize::ByteSize(to_u64(size)));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{
        ContainerInspectResponse, ContainerStatsResponse, ContainerSummary, EventMessage,
        ImageSummary, Network, SystemDataUsageResponse, VolumeListResponse,
    };

    fn engine() -> berth_core::EngineId {
        berth_core::EngineId("colima:default".to_string())
    }

    fn summaries() -> Vec<ContainerSummary> {
        serde_json::from_str(include_str!("../../../fixtures/docker/containers.json"))
            .expect("containers fixture parses")
    }

    fn containers() -> Vec<berth_core::Container> {
        summaries()
            .iter()
            .map(|s| container(&engine(), s))
            .collect()
    }

    fn by_name<'a>(cs: &'a [berth_core::Container], name: &str) -> &'a berth_core::Container {
        cs.iter().find(|c| c.name == name).expect("named container")
    }

    fn inspect(fixture: &str) -> ContainerInspectResponse {
        let text = match fixture {
            "web" => include_str!("../../../fixtures/docker/inspect-web.json"),
            _ => include_str!("../../../fixtures/docker/inspect-db.json"),
        };
        serde_json::from_str(text).expect("inspect fixture parses")
    }

    fn df() -> SystemDataUsageResponse {
        serde_json::from_str(include_str!("../../../fixtures/docker/df.json"))
            .expect("df fixture parses")
    }

    #[test]
    fn mappers_containers_all_four_in_berthfx() {
        let cs = containers();
        assert_eq!(cs.len(), 4);
        assert!(cs
            .iter()
            .all(|c| c.compose_project.as_deref() == Some("berthfx")));
    }

    #[test]
    fn mappers_container_names_have_no_leading_slash() {
        let cs = containers();
        let mut names: Vec<&str> = cs.iter().map(|c| c.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "berthfx-cache-1",
                "berthfx-db-1",
                "berthfx-logger-1",
                "berthfx-web-1"
            ]
        );
    }

    #[test]
    fn mappers_container_services_from_labels() {
        let cs = containers();
        assert_eq!(
            by_name(&cs, "berthfx-web-1").service.as_deref(),
            Some("web")
        );
        assert_eq!(by_name(&cs, "berthfx-db-1").service.as_deref(), Some("db"));
    }

    #[test]
    fn mappers_web_port_is_56080_to_80_tcp_on_loopback() {
        let cs = containers();
        let web = by_name(&cs, "berthfx-web-1");
        assert_eq!(web.ports.len(), 1);
        assert_eq!(web.ports[0].host_port, Some(56080));
        assert_eq!(web.ports[0].container_port, 80);
        assert_eq!(web.ports[0].proto, berth_core::Proto::Tcp);
        assert_eq!(
            web.ports[0].host_ip.map(|ip| ip.to_string()),
            Some("127.0.0.1".to_string())
        );
    }

    #[test]
    fn mappers_web_and_db_are_healthy_cache_has_no_health() {
        let cs = containers();
        assert_eq!(
            by_name(&cs, "berthfx-web-1").health,
            Some(berth_core::Health::Healthy)
        );
        assert_eq!(
            by_name(&cs, "berthfx-db-1").health,
            Some(berth_core::Health::Healthy)
        );
        assert_eq!(by_name(&cs, "berthfx-cache-1").health, None);
    }

    #[test]
    fn mappers_containers_are_running_with_images() {
        let cs = containers();
        assert!(cs
            .iter()
            .all(|c| c.state == berth_core::ContainerState::Running));
        assert_eq!(
            by_name(&cs, "berthfx-web-1").image.reference,
            "nginx:alpine"
        );
        assert_eq!(
            by_name(&cs, "berthfx-db-1").image.reference,
            "postgres:17-alpine"
        );
        assert!(by_name(&cs, "berthfx-web-1").image.id.is_some());
    }

    #[test]
    fn mappers_containers_share_the_fixture_network() {
        let cs = containers();
        assert!(cs
            .iter()
            .all(|c| c.networks == ["berthfx_default".to_string()]));
    }

    #[test]
    fn mappers_exited_status_yields_state_and_exit_code() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/docker/containers.json"))
                .expect("fixture");
        value[0]["State"] = serde_json::json!("exited");
        value[0]["Status"] = serde_json::json!("Exited (137) 2 hours ago");
        let summaries: Vec<ContainerSummary> =
            serde_json::from_value(value).expect("mutated fixture parses");
        let c = container(&engine(), &summaries[0]);
        assert_eq!(c.state, berth_core::ContainerState::Exited);
        assert_eq!(c.exit_code, Some(137));
    }

    #[test]
    fn mappers_detail_web_ports_and_health_log() {
        let d = detail(&engine(), &inspect("web"));
        assert_eq!(d.container.name, "berthfx-web-1");
        assert_eq!(d.container.ports.len(), 1);
        assert_eq!(d.container.ports[0].host_port, Some(56080));
        assert_eq!(d.container.ports[0].container_port, 80);
        assert_eq!(d.container.health, Some(berth_core::Health::Healthy));
        assert!(!d.health_log.is_empty());
        assert!(d.health_log.iter().all(|r| r.exit_code == 0));
        assert!(d.container.started.is_some());
    }

    #[test]
    fn mappers_detail_db_env_kept_but_inspect_masked() {
        let d = detail(&engine(), &inspect("db"));
        let keys: Vec<&str> = d.env.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&"POSTGRES_PASSWORD"), "{keys:?}");
        assert!(keys.contains(&"PATH"));
        let inspect = serde_json::to_string(&d.inspect).expect("inspect serializes");
        // The fixture password may only appear in fixtures/ and scripts/.
        let fixture_password = ["berthfx", "dev"].join("-");
        assert!(!inspect.contains(&fixture_password), "password leaked");
        assert!(
            !inspect.contains("/usr/local/sbin"),
            "PATH value leaked through inspect"
        );
        let env = d
            .inspect
            .pointer("/Config/Env")
            .and_then(serde_json::Value::as_array)
            .expect("masked Env array");
        assert!(env
            .iter()
            .all(|e| { e.as_str().is_some_and(|s| s.ends_with("=••••")) }));
        // env keeps the real values for the one-click reveal.
        let path = d.env.iter().find(|e| e.key == "PATH").expect("PATH kept");
        assert!(path.value.contains("/usr/local/sbin"));
    }

    #[test]
    fn mappers_detail_db_volume_mount_and_defaults() {
        let d = detail(&engine(), &inspect("db"));
        assert_eq!(d.container.mounts.len(), 1);
        let mount = &d.container.mounts[0];
        assert_eq!(mount.kind, berth_core::MountKind::Volume);
        assert_eq!(mount.volume_name.as_deref(), Some("berthfx_berthfx_pgdata"));
        assert_eq!(mount.destination, "/var/lib/postgresql/data");
        assert!(!mount.read_only);
        assert_eq!(d.restart, berth_core::RestartPolicy::No);
        assert_eq!(d.limits, berth_core::Limits::default());
    }

    #[test]
    fn mappers_images_tags_sizes_and_dangling() {
        let summaries: Vec<ImageSummary> =
            serde_json::from_str(include_str!("../../../fixtures/docker/images.json"))
                .expect("images fixture parses");
        let images: Vec<berth_core::Image> = summaries.iter().map(|s| image(s, &[])).collect();
        assert_eq!(images.len(), 4);
        let mut tags: Vec<&str> = images
            .iter()
            .filter_map(|i| i.repo_tags.first())
            .map(String::as_str)
            .collect();
        tags.sort_unstable();
        assert_eq!(
            tags,
            [
                "alpine:3",
                "nginx:alpine",
                "postgres:17-alpine",
                "redis:7-alpine"
            ]
        );
        assert!(images.iter().all(|i| i.size.0 > 0 && !i.dangling));
        assert!(images.iter().all(|i| !i.digests.is_empty()));
    }

    #[test]
    fn mappers_image_used_by_matches_containers() {
        let summaries: Vec<ImageSummary> =
            serde_json::from_str(include_str!("../../../fixtures/docker/images.json"))
                .expect("images fixture parses");
        let cs = containers();
        let nginx = summaries
            .iter()
            .find(|s| s.repo_tags.contains(&"nginx:alpine".to_string()))
            .expect("nginx");
        let mapped = image(nginx, &cs);
        assert_eq!(mapped.used_by, [by_name(&cs, "berthfx-web-1").id.clone()]);
    }

    #[test]
    fn mappers_volume_from_list_has_no_size_and_knows_its_container() {
        let list: VolumeListResponse =
            serde_json::from_str(include_str!("../../../fixtures/docker/volumes.json"))
                .expect("volumes fixture parses");
        let volumes = list.volumes.expect("volumes present");
        assert_eq!(volumes.len(), 1);
        let cs = containers();
        // Summary mounts may not name the volume on every API version; use
        // the inspect-derived db container, which does.
        let db = detail(&engine(), &inspect("db")).container;
        let _ = &cs;
        let v = volume(&volumes[0], std::slice::from_ref(&db));
        assert_eq!(v.name, "berthfx_berthfx_pgdata");
        assert_eq!(v.driver, "local");
        assert_eq!(v.size, None);
        assert!(v.mountpoint.ends_with("berthfx_berthfx_pgdata/_data"));
        assert_eq!(v.used_by, vec![db.id.clone()]);
        assert_eq!(
            v.labels.get("dev.berth.fixture").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn mappers_network_subnet_gateway_driver() {
        let networks: Vec<Network> =
            serde_json::from_str(include_str!("../../../fixtures/docker/networks.json"))
                .expect("networks fixture parses");
        assert_eq!(networks.len(), 1);
        let n = network(&networks[0]);
        assert_eq!(n.name, "berthfx_default");
        assert_eq!(n.driver, "bridge");
        assert_eq!(n.subnet.as_deref(), Some("172.28.0.0/16"));
        assert_eq!(n.gateway.as_deref(), Some("172.28.0.1"));
        assert!(!n.internal && !n.built_in);
    }

    #[test]
    fn mappers_disk_usage_groups_from_df() {
        let usage = disk_usage(&df());
        assert_eq!(usage.images.count, 44);
        assert!(usage.images.size.0 > 0);
        assert!(usage.images.reclaimable.0 > 0);
        assert_eq!(usage.containers.count, 7);
        assert_eq!(usage.volumes.count, 119);
        assert!(usage.volumes.size.0 > usage.volumes.reclaimable.0);
        assert_eq!(usage.disk_total, None);
    }

    #[test]
    fn mappers_volume_sizes_from_df_items() {
        let sizes = volume_sizes(&df());
        assert_eq!(sizes.len(), 1);
        let size = sizes
            .get("berthfx_berthfx_pgdata")
            .expect("fixture volume size");
        assert!(size.0 > 1_000_000, "pgdata should be megabytes: {size}");
    }

    #[test]
    fn mappers_events_decode_kinds_and_attributes() {
        let messages: Vec<EventMessage> =
            serde_json::from_str(include_str!("../../../fixtures/docker/events.json"))
                .expect("events fixture parses");
        assert!(!messages.is_empty());
        let events: Vec<berth_core::EngineEvent> =
            messages.iter().map(|m| event(&engine(), m)).collect();
        assert!(events
            .iter()
            .all(|e| matches!(e.kind, berth_core::EventKind::Container { .. })));
        let actions: Vec<&str> = events
            .iter()
            .filter_map(|e| match &e.kind {
                berth_core::EventKind::Container { action, .. } => Some(action.as_str()),
                _ => None,
            })
            .collect();
        assert!(actions.contains(&"start"), "{actions:?}");
        assert!(events.iter().all(|e| e.attributes.contains_key("name")));
        assert!(events.iter().all(|e| e.at.timestamp() > 0));
    }

    #[test]
    fn mappers_cpu_percent_uses_docker_cli_formula() {
        let stats: ContainerStatsResponse = serde_json::from_value(serde_json::json!({
            "cpu_stats": {
                "cpu_usage": {"total_usage": 300, "percpu_usage": [150, 150]},
                "system_cpu_usage": 1000,
                "online_cpus": 2
            },
            "precpu_stats": {
                "cpu_usage": {"total_usage": 100, "percpu_usage": [50, 50]},
                "system_cpu_usage": 500,
                "online_cpus": 2
            }
        }))
        .expect("stats parse");
        // (200 / 500) * 2 * 100 = 80
        assert_eq!(cpu_percent(&stats), Some(80.0));
    }

    #[test]
    fn mappers_cpu_percent_none_without_delta() {
        let stats: ContainerStatsResponse =
            serde_json::from_value(serde_json::json!({})).expect("empty stats parse");
        assert_eq!(cpu_percent(&stats), None);
    }

    // PLAN §5 pins insta to json+yaml, so snapshots are scrubbed by hand
    // instead of using insta's `redactions` feature.
    fn scrub_container(c: &mut berth_core::Container) {
        c.id = berth_core::ContainerId("[id]".to_string());
        c.created = DateTime::<Utc>::UNIX_EPOCH;
        c.started = None;
        c.labels.clear();
        c.image.id = Some("[image-id]".to_string());
    }

    #[test]
    fn mappers_containers_snapshot() {
        let mut cs = containers();
        cs.iter_mut().for_each(scrub_container);
        cs.sort_by(|a, b| a.name.cmp(&b.name));
        insta::assert_json_snapshot!(cs);
    }

    #[test]
    fn mappers_detail_web_snapshot() {
        let mut d = detail(&engine(), &inspect("web"));
        scrub_container(&mut d.container);
        d.health_log.clear();
        d.inspect = serde_json::json!("[inspect]");
        insta::assert_json_snapshot!(d);
    }

    #[test]
    fn mappers_images_snapshot() {
        let summaries: Vec<ImageSummary> =
            serde_json::from_str(include_str!("../../../fixtures/docker/images.json"))
                .expect("images fixture parses");
        let mut images: Vec<berth_core::Image> = summaries.iter().map(|s| image(s, &[])).collect();
        for image in &mut images {
            image.id = "[id]".to_string();
            image.created = DateTime::<Utc>::UNIX_EPOCH;
            image.digests.clear();
        }
        images.sort_by(|a, b| a.repo_tags.cmp(&b.repo_tags));
        insta::assert_json_snapshot!(images);
    }

    #[test]
    fn mappers_disk_usage_snapshot() {
        insta::assert_json_snapshot!(disk_usage(&df()));
    }
}
