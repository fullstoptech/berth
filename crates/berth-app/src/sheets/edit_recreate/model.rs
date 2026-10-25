//! The edit and recreate sheet's model (P3-T7, brief §6.7): pure
//! functions from a container's detail to the form's draft, from a draft
//! back to a [`ContainerSpec`], and from two specs to what the sheet
//! offers. Nothing here reads a signal or calls an engine.
//!
//! Env values never enter the draft's text: an existing variable keeps its
//! value unless the user types a new one, and the diff shows `KEY=••••`
//! (`berth_core::spec_diff`).

use std::collections::BTreeMap;
use std::net::IpAddr;

use bytesize::ByteSize;

use berth_core::{
    needs_recreate, spec_diff, ContainerDetail, ContainerSpec, ImageRef, Limits, Mount, MountKind,
    PortMapping, Proto, RestartPolicy,
};

// ---------------------------------------------------------------------
// Spec from a detail
// ---------------------------------------------------------------------

/// The published ports of a container: the configured `HostConfig`
/// bindings of the raw inspect when it has them (a stopped container has
/// no live ones), else the live ports that have a host side. Exposed-only
/// ports are left out: an empty binding in a create request would publish
/// them on a random host port.
#[must_use]
pub fn published_ports(detail: &ContainerDetail) -> Vec<PortMapping> {
    let configured = detail
        .inspect
        .pointer("/HostConfig/PortBindings")
        .and_then(serde_json::Value::as_object);
    let Some(configured) = configured else {
        return detail
            .container
            .ports
            .iter()
            .filter(|p| p.host_port.is_some())
            .cloned()
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

/// What Edit and recreate starts from. The command and entrypoint are the
/// container's own (the image's defaults included), so a recreate keeps
/// what runs.
#[must_use]
pub fn spec_from_detail(detail: &ContainerDetail) -> ContainerSpec {
    let c = &detail.container;
    let non_empty = |v: &Option<Vec<String>>| v.clone().filter(|v| !v.is_empty());
    let platform = match &c.platform {
        Some(arch @ (berth_core::Arch::Aarch64 | berth_core::Arch::X86_64)) => {
            Some(format!("linux/{}", arch.docker_name()))
        }
        _ => None,
    };
    ContainerSpec {
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
    }
}

/// Settings Docker holds for the container that a [`ContainerSpec`] cannot
/// carry: a recreate drops them. Only what the raw inspect shows as set.
#[must_use]
pub fn not_carried(detail: &ContainerDetail) -> Vec<String> {
    let host = |name: &str| detail.inspect.pointer(&format!("/HostConfig/{name}"));
    let non_empty_list = |name: &str| {
        host(name)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|a| !a.is_empty())
    };
    let non_empty_map = |name: &str| {
        host(name)
            .and_then(serde_json::Value::as_object)
            .is_some_and(|a| !a.is_empty())
    };
    let mut out = Vec::new();
    if host("Privileged").and_then(serde_json::Value::as_bool) == Some(true) {
        out.push("privileged mode".to_string());
    }
    for (name, label) in [
        ("CapAdd", "added capabilities"),
        ("CapDrop", "dropped capabilities"),
        ("Devices", "devices"),
        ("ExtraHosts", "extra hosts"),
        ("Ulimits", "ulimits"),
        ("SecurityOpt", "security options"),
        ("Dns", "DNS servers"),
    ] {
        if non_empty_list(name) {
            out.push(label.to_string());
        }
    }
    for (name, label) in [("Tmpfs", "tmpfs mounts"), ("Sysctls", "sysctls")] {
        if non_empty_map(name) {
            out.push(label.to_string());
        }
    }
    out
}

/// True when the container sets a working directory, user or health check
/// of its own, which the new container takes from the image instead.
#[must_use]
pub fn takes_from_image(detail: &ContainerDetail) -> bool {
    let has_healthcheck = detail
        .inspect
        .pointer("/Config/Healthcheck/Test")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|t| !t.is_empty());
    detail.working_dir.is_some() || detail.user.is_some() || has_healthcheck
}

// ---------------------------------------------------------------------
// Text forms
// ---------------------------------------------------------------------

/// Split a command line the way a shell would, without expanding
/// anything: spaces separate, `'…'` and `"…"` group, `\` escapes in `"…"`
/// and outside quotes.
///
/// # Errors
/// An unterminated quote or a trailing backslash.
pub fn split_command(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => current.push(c),
                        None => return Err("A ' is not closed".to_string()),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c) if c == '"' || c == '\\' => current.push(c),
                            Some(c) => {
                                current.push('\\');
                                current.push(c);
                            }
                            None => return Err("A \" is not closed".to_string()),
                        },
                        Some(c) => current.push(c),
                        None => return Err("A \" is not closed".to_string()),
                    }
                }
            }
            '\\' => {
                started = true;
                match chars.next() {
                    Some(c) => current.push(c),
                    None => return Err("The text ends with a backslash".to_string()),
                }
            }
            c => {
                started = true;
                current.push(c);
            }
        }
    }
    if started {
        out.push(current);
    }
    Ok(out)
}

/// The inverse of [`split_command`].
#[must_use]
pub fn join_command(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            let plain = !arg.is_empty()
                && !arg
                    .chars()
                    .any(|c| c.is_whitespace() || matches!(c, '\'' | '"' | '\\'));
            if plain {
                arg.clone()
            } else {
                format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `127.0.0.1:56080:80`, `8080:80/udp`, `80`: Docker's `-p` spelling.
#[must_use]
pub fn format_port(p: &PortMapping) -> String {
    let proto = match p.proto {
        Proto::Tcp => String::new(),
        Proto::Udp => "/udp".to_string(),
        Proto::Sctp => "/sctp".to_string(),
    };
    let ip = |ip: &IpAddr| match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };
    match (p.host_ip.as_ref(), p.host_port) {
        (Some(address), Some(host)) => {
            format!("{}:{host}:{}{proto}", ip(address), p.container_port)
        }
        (Some(address), None) => format!("{}::{}{proto}", ip(address), p.container_port),
        (None, Some(host)) => format!("{host}:{}{proto}", p.container_port),
        (None, None) => format!("{}{proto}", p.container_port),
    }
}

/// # Errors
/// A sentence saying what is wrong with the text.
pub fn parse_port(text: &str) -> Result<PortMapping, String> {
    let text = text.trim();
    let (rest, proto) = match text.rsplit_once('/') {
        Some((rest, "tcp")) => (rest, Proto::Tcp),
        Some((rest, "udp")) => (rest, Proto::Udp),
        Some((rest, "sctp")) => (rest, Proto::Sctp),
        Some((_, other)) => return Err(format!("Unknown protocol {other}; use tcp, udp or sctp")),
        None => (text, Proto::Tcp),
    };
    let port = |s: &str, what: &str| {
        s.parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .ok_or_else(|| format!("{what} must be a port number from 1 to 65535"))
    };
    // A bracketed IPv6 address leads the text; its colons are not separators.
    let (ip, rest) = if let Some(after) = rest.strip_prefix('[') {
        let (address, tail) = after
            .split_once(']')
            .ok_or_else(|| "A [ is not closed".to_string())?;
        let address: IpAddr = address
            .parse()
            .map_err(|_| format!("{address} is not an IP address"))?;
        let tail = tail
            .strip_prefix(':')
            .ok_or_else(|| "Write [address]:host:container".to_string())?;
        (Some(address), tail)
    } else {
        (None, rest)
    };
    let parts: Vec<&str> = rest.split(':').collect();
    let (host_ip, host_port, container) = match (ip, parts.as_slice()) {
        (None, [container]) => (None, None, *container),
        (None, [host, container]) => (None, Some(port(host, "The host port")?), *container),
        (None, [address, host, container]) => {
            let address = if address.is_empty() {
                None
            } else {
                Some(
                    address
                        .parse::<IpAddr>()
                        .map_err(|_| format!("{address} is not an IP address"))?,
                )
            };
            let host = if host.is_empty() {
                None
            } else {
                Some(port(host, "The host port")?)
            };
            (address, host, *container)
        }
        (Some(address), [host, container]) => {
            let host = if host.is_empty() {
                None
            } else {
                Some(port(host, "The host port")?)
            };
            (Some(address), host, *container)
        }
        _ => {
            return Err("Write host:container, ip:host:container or just the container port".into())
        }
    };
    Ok(PortMapping {
        host_ip,
        host_port,
        container_port: port(container, "The container port")?,
        proto,
    })
}

/// `/Users/me/app:/app:ro`, `pgdata:/var/lib/postgresql/data`,
/// `tmpfs:/run`. A volume literally named `tmpfs` cannot be written.
#[must_use]
pub fn format_mount(m: &Mount) -> String {
    let mode = if m.read_only { ":ro" } else { "" };
    match m.kind {
        MountKind::Tmpfs => format!("tmpfs:{}{mode}", m.destination),
        MountKind::Volume => {
            let name = m.volume_name.as_deref().unwrap_or(&m.source);
            format!("{name}:{}{mode}", m.destination)
        }
        MountKind::Bind | MountKind::Other => format!("{}:{}{mode}", m.source, m.destination),
    }
}

/// # Errors
/// A sentence saying what is wrong with the text.
pub fn parse_mount(text: &str) -> Result<Mount, String> {
    let text = text.trim();
    let parts: Vec<&str> = text.split(':').collect();
    let (source, destination, read_only) = match parts.as_slice() {
        [source, destination] => (*source, *destination, false),
        [source, destination, "ro"] => (*source, *destination, true),
        [source, destination, "rw"] => (*source, *destination, false),
        _ => return Err("Write source:destination, with :ro to mount it read-only".to_string()),
    };
    if !destination.starts_with('/') {
        return Err("The destination must be an absolute path inside the container".to_string());
    }
    if source == "tmpfs" {
        return Ok(Mount {
            kind: MountKind::Tmpfs,
            source: String::new(),
            destination: destination.to_string(),
            read_only,
            volume_name: None,
        });
    }
    if source.starts_with('/') {
        return Ok(Mount {
            kind: MountKind::Bind,
            source: source.to_string(),
            destination: destination.to_string(),
            read_only,
            volume_name: None,
        });
    }
    if source.starts_with('.') || source.starts_with('~') {
        return Err("A bind source must be an absolute path".to_string());
    }
    if !valid_name(source) {
        return Err(format!(
            "{source} is not a volume name; a bind source starts with /"
        ));
    }
    Ok(Mount {
        kind: MountKind::Volume,
        source: source.to_string(),
        destination: destination.to_string(),
        read_only,
        volume_name: Some(source.to_string()),
    })
}

/// Docker's rule for container, volume and network names.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Memory the way Docker spells it: `256m`, `1.5g`, `512k` or bytes.
/// `m`, `mb` and `mib` all mean 1024 × 1024 bytes, as `docker run -m`.
///
/// # Errors
/// A sentence saying what is wrong with the text.
pub fn parse_memory(text: &str) -> Result<Option<ByteSize>, String> {
    let text = text.trim().to_ascii_lowercase();
    if text.is_empty() {
        return Ok(None);
    }
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let value: f64 = number
        .parse()
        .map_err(|_| "Write a size such as 256m or 1.5g".to_string())?;
    let factor: f64 = match unit.trim() {
        "" | "b" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        other => return Err(format!("Unknown unit {other}; use k, m or g")),
    };
    let bytes = value * factor;
    if !bytes.is_finite() || bytes >= 9.0e18 {
        return Err("That is too large".to_string());
    }
    if bytes < 1.0 {
        return Ok(None);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bytes = bytes.round() as u64;
    if bytes < 6 * 1024 * 1024 {
        return Err("Docker needs at least 6m".to_string());
    }
    Ok(Some(ByteSize(bytes)))
}

/// The inverse of [`parse_memory`] for a value Docker reports.
#[must_use]
pub fn format_memory(value: Option<ByteSize>) -> String {
    let Some(ByteSize(bytes)) = value else {
        return String::new();
    };
    for (unit, factor) in [("g", 1_u64 << 30), ("m", 1 << 20), ("k", 1 << 10)] {
        if bytes >= factor && bytes % factor == 0 {
            return format!("{}{unit}", bytes / factor);
        }
    }
    bytes.to_string()
}

/// # Errors
/// A sentence saying what is wrong with the text.
pub fn parse_cpus(text: &str) -> Result<Option<f32>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let value: f32 = text
        .parse()
        .map_err(|_| "Write a number of CPUs such as 0.5 or 2".to_string())?;
    if !value.is_finite() || value <= 0.0 {
        return Err("Write more than 0 CPUs, or leave it empty for no limit".to_string());
    }
    if value > 1024.0 {
        return Err("That is more CPUs than Docker accepts".to_string());
    }
    Ok(Some(value))
}

#[must_use]
pub fn format_cpus(value: Option<f32>) -> String {
    value.map_or_else(String::new, |cpus| format!("{cpus}"))
}

// ---------------------------------------------------------------------
// The draft
// ---------------------------------------------------------------------

/// Which field a message belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Image,
    Command,
    EnvKey(u32),
    Port(u32),
    Mount(u32),
    Networks,
    Cpus,
    Memory,
    Swap,
    Retries,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub field: Field,
    pub message: String,
}

fn error(field: Field, message: impl Into<String>) -> FieldError {
    FieldError {
        field,
        message: message.into(),
    }
}

/// One environment row. An existing variable has no typed value until the
/// user types one (`value: None` keeps what the container has); a new one
/// carries what was typed.
#[derive(Clone, PartialEq, Eq)]
pub struct EnvRow {
    pub id: u32,
    pub key: String,
    pub value: Option<String>,
    pub existing: bool,
}

// Not derived: a derived `Debug` would print a typed value.
impl std::fmt::Debug for EnvRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvRow")
            .field("id", &self.id)
            .field("key", &self.key)
            .field("typed_value", &self.value.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RestartKind {
    #[default]
    No,
    Always,
    UnlessStopped,
    OnFailure,
}

impl RestartKind {
    pub const ALL: [RestartKind; 4] =
        [Self::No, Self::Always, Self::UnlessStopped, Self::OnFailure];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Always => "always",
            Self::UnlessStopped => "unless-stopped",
            Self::OnFailure => "on-failure",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::No => "No restart",
            Self::Always => "Always",
            Self::UnlessStopped => "Unless stopped",
            Self::OnFailure => "On failure",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    #[must_use]
    pub fn of(policy: RestartPolicy) -> Self {
        match policy {
            RestartPolicy::No => Self::No,
            RestartPolicy::Always => Self::Always,
            RestartPolicy::UnlessStopped => Self::UnlessStopped,
            RestartPolicy::OnFailure { .. } => Self::OnFailure,
        }
    }
}

/// What the form holds, as the text the user sees. Rows have stable ids
/// so a removed row does not shift the inputs below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub image: String,
    pub command: String,
    pub env: Vec<EnvRow>,
    pub ports: Vec<(u32, String)>,
    pub mounts: Vec<(u32, String)>,
    pub networks: String,
    pub restart: RestartKind,
    pub retries: String,
    pub cpus: String,
    pub memory: String,
    pub swap: String,
    next_id: u32,
}

impl Draft {
    #[must_use]
    pub fn from_spec(spec: &ContainerSpec) -> Self {
        let mut next_id = 0;
        let mut id = || {
            next_id += 1;
            next_id
        };
        let env = spec
            .env
            .iter()
            .map(|(key, _)| EnvRow {
                id: id(),
                key: key.clone(),
                value: None,
                existing: true,
            })
            .collect();
        let ports = spec.ports.iter().map(|p| (id(), format_port(p))).collect();
        let mounts = spec
            .mounts
            .iter()
            .map(|m| (id(), format_mount(m)))
            .collect();
        let retries = match spec.restart {
            RestartPolicy::OnFailure { max_retries } if max_retries > 0 => max_retries.to_string(),
            _ => String::new(),
        };
        Self {
            image: spec.image.reference.clone(),
            command: spec
                .command
                .as_deref()
                .map(join_command)
                .unwrap_or_default(),
            env,
            ports,
            mounts,
            networks: spec.networks.join(", "),
            restart: RestartKind::of(spec.restart),
            retries,
            cpus: format_cpus(spec.limits.cpus),
            memory: format_memory(spec.limits.memory),
            swap: format_memory(spec.limits.memory_swap),
            next_id,
        }
    }

    fn take_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    pub fn add_env(&mut self) -> u32 {
        let id = self.take_id();
        self.env.push(EnvRow {
            id,
            key: String::new(),
            value: Some(String::new()),
            existing: false,
        });
        id
    }

    pub fn add_port(&mut self) -> u32 {
        let id = self.take_id();
        self.ports.push((id, String::new()));
        id
    }

    pub fn add_mount(&mut self) -> u32 {
        let id = self.take_id();
        self.mounts.push((id, String::new()));
        id
    }

    pub fn remove_env(&mut self, id: u32) {
        self.env.retain(|row| row.id != id);
    }

    pub fn remove_port(&mut self, id: u32) {
        self.ports.retain(|(row, _)| *row != id);
    }

    pub fn remove_mount(&mut self, id: u32) {
        self.mounts.retain(|(row, _)| *row != id);
    }

    pub fn set_env_key(&mut self, id: u32, key: &str) {
        if let Some(row) = self.env.iter_mut().find(|row| row.id == id) {
            key.clone_into(&mut row.key);
        }
    }

    /// An existing row with an empty text goes back to keeping its value.
    pub fn set_env_value(&mut self, id: u32, value: &str) {
        if let Some(row) = self.env.iter_mut().find(|row| row.id == id) {
            row.value = if row.existing && value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
        }
    }

    pub fn set_port(&mut self, id: u32, text: &str) {
        if let Some((_, row)) = self.ports.iter_mut().find(|(row, _)| *row == id) {
            text.clone_into(row);
        }
    }

    pub fn set_mount(&mut self, id: u32, text: &str) {
        if let Some((_, row)) = self.mounts.iter_mut().find(|(row, _)| *row == id) {
            text.clone_into(row);
        }
    }

    /// The spec this draft describes, on top of `base` (what the form does
    /// not show stays as `base` has it). Every problem is reported, not
    /// only the first.
    ///
    /// # Errors
    /// One [`FieldError`] per field that cannot be read.
    pub fn to_spec(&self, base: &ContainerSpec) -> Result<ContainerSpec, Vec<FieldError>> {
        let mut errors = Vec::new();
        let mut spec = base.clone();

        let image = self.image.trim();
        if image.is_empty() || image.contains(char::is_whitespace) {
            errors.push(error(Field::Image, "Write an image such as nginx:alpine"));
        } else if image != base.image.reference {
            spec.image = ImageRef {
                reference: image.to_string(),
                id: None,
            };
        }

        match split_command(&self.command) {
            Ok(args) => spec.command = (!args.is_empty()).then_some(args),
            Err(message) => errors.push(error(Field::Command, message)),
        }

        spec.env = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for row in &self.env {
            let key = row.key.trim();
            if key.is_empty() || key.contains('=') || key.contains(char::is_whitespace) {
                errors.push(error(
                    Field::EnvKey(row.id),
                    "A name has no spaces or = and is not empty",
                ));
                continue;
            }
            if seen.contains(&key) {
                errors.push(error(Field::EnvKey(row.id), format!("{key} appears twice")));
                continue;
            }
            seen.push(key);
            let value = match &row.value {
                Some(value) => value.clone(),
                None => base
                    .env
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default(),
            };
            spec.env.push((key.to_string(), value));
        }

        spec.ports = Vec::new();
        for (id, text) in &self.ports {
            match reuse_or_parse(text, &base.ports, format_port, parse_port) {
                Ok(port) => spec.ports.push(port),
                Err(message) => errors.push(error(Field::Port(*id), message)),
            }
        }
        spec.mounts = Vec::new();
        for (id, text) in &self.mounts {
            match reuse_or_parse(text, &base.mounts, format_mount, parse_mount) {
                Ok(mount) => spec.mounts.push(mount),
                Err(message) => errors.push(error(Field::Mount(*id), message)),
            }
        }

        let networks: Vec<String> = self
            .networks
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .collect();
        match networks.iter().find(|n| !valid_name(n)) {
            Some(bad) => errors.push(error(
                Field::Networks,
                format!("{bad} is not a network name"),
            )),
            None => spec.networks = networks,
        }

        match limits_and_restart(self, base) {
            Ok((limits, restart)) => {
                spec.limits = limits;
                spec.restart = restart;
            }
            Err(more) => errors.extend(more),
        }

        if errors.is_empty() {
            Ok(spec)
        } else {
            Err(errors)
        }
    }

    /// The problems of this draft, or none.
    #[must_use]
    pub fn errors(&self, base: &ContainerSpec) -> Vec<FieldError> {
        self.to_spec(base).err().unwrap_or_default()
    }
}

/// A row that still reads as an existing item keeps that item whole (a
/// volume's own fields survive the round trip through text).
fn reuse_or_parse<T: Clone>(
    text: &str,
    existing: &[T],
    format: fn(&T) -> String,
    parse: fn(&str) -> Result<T, String>,
) -> Result<T, String> {
    if let Some(found) = existing.iter().find(|item| format(item) == text.trim()) {
        return Ok(found.clone());
    }
    parse(text)
}

/// The limits and restart policy a draft describes.
///
/// # Errors
/// One [`FieldError`] per unreadable field.
pub fn limits_and_restart(
    draft: &Draft,
    base: &ContainerSpec,
) -> Result<(Limits, RestartPolicy), Vec<FieldError>> {
    let mut errors = Vec::new();
    let cpus = parse_cpus(&draft.cpus).unwrap_or_else(|message| {
        errors.push(error(Field::Cpus, message));
        base.limits.cpus
    });
    let memory = parse_memory(&draft.memory).unwrap_or_else(|message| {
        errors.push(error(Field::Memory, message));
        base.limits.memory
    });
    let memory_swap = parse_memory(&draft.swap).unwrap_or_else(|message| {
        errors.push(error(Field::Swap, message));
        base.limits.memory_swap
    });
    if let (Some(memory), Some(swap)) = (memory, memory_swap) {
        if swap < memory && !errors.iter().any(|e| e.field == Field::Swap) {
            errors.push(error(
                Field::Swap,
                "Memory plus swap cannot be less than the memory limit",
            ));
        }
    }
    let restart = match draft.restart {
        RestartKind::No => RestartPolicy::No,
        RestartKind::Always => RestartPolicy::Always,
        RestartKind::UnlessStopped => RestartPolicy::UnlessStopped,
        RestartKind::OnFailure => {
            let text = draft.retries.trim();
            let max_retries = if text.is_empty() {
                0
            } else {
                text.parse::<u16>().unwrap_or_else(|_| {
                    errors.push(error(
                        Field::Retries,
                        "Write a number of retries, or leave it empty",
                    ));
                    0
                })
            };
            RestartPolicy::OnFailure { max_retries }
        }
    };
    if errors.is_empty() {
        Ok((
            Limits {
                cpus,
                memory,
                memory_swap,
            },
            restart,
        ))
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------------
// What the sheet offers
// ---------------------------------------------------------------------

/// The one primary button of the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// Nothing differs.
    Nothing,
    /// Only limits or restart policy differ, and Docker can apply them to
    /// the running container.
    ApplyLimits,
    /// Anything else, or a limit that was removed (Docker's update cannot
    /// clear one).
    Recreate,
}

/// A limit that had a value and now has none cannot be cleared by
/// `update_limits` (an absent field leaves it as it is).
#[must_use]
pub fn removes_a_limit(old: &Limits, new: &Limits) -> bool {
    (old.cpus.is_some() && new.cpus.is_none())
        || (old.memory.is_some() && new.memory.is_none())
        || (old.memory_swap.is_some() && new.memory_swap.is_none())
}

#[must_use]
pub fn offer(old: &ContainerSpec, new: &ContainerSpec) -> Offer {
    if spec_diff(old, new).is_empty() {
        Offer::Nothing
    } else if needs_recreate(old, new) || removes_a_limit(&old.limits, &new.limits) {
        Offer::Recreate
    } else {
        Offer::ApplyLimits
    }
}

/// One line of the diff the sheet shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRow {
    pub field: String,
    pub before: String,
    pub after: String,
}

/// `spec_diff` as rows. Env is already `KEY=••••` in `spec_diff`; when a
/// value alone changed both sides read the same, so say so.
#[must_use]
pub fn diff_rows(old: &ContainerSpec, new: &ContainerSpec) -> Vec<DiffRow> {
    spec_diff(old, new)
        .into_iter()
        .map(|change| {
            let after = if change.field == "env" && change.before == change.after {
                "values changed (hidden)".to_string()
            } else {
                change.after
            };
            DiffRow {
                field: change.field,
                before: change.before,
                after,
            }
        })
        .collect()
}

/// The sheet's message after a recreate failed. The engine puts the old
/// container back; when that failed too, say so instead.
#[must_use]
pub fn failure_text(message: &str) -> String {
    if message.contains("restoring the old container also failed") {
        format!("The recreate failed and the old container could not be restored: {message}")
    } else {
        format!("The old container was kept. {message}")
    }
}

// ---------------------------------------------------------------------
// Duplicate and rename
// ---------------------------------------------------------------------

/// `web-copy`, then `web-copy-2`, … the first name nobody holds.
#[must_use]
pub fn duplicate_name(name: &str, taken: &[&str]) -> String {
    let first = format!("{name}-copy");
    if !taken.contains(&first.as_str()) {
        return first;
    }
    (2..)
        .map(|n| format!("{first}-{n}"))
        .find(|candidate| !taken.contains(&candidate.as_str()))
        .unwrap_or(first)
}

/// The spec of a duplicate: a new name, Compose's own labels gone (the
/// copy is not that service), anonymous volumes gone (the copy gets its
/// own from the image). Named volumes stay shared.
#[must_use]
pub fn duplicate_spec(spec: &ContainerSpec, taken: &[&str]) -> ContainerSpec {
    let mut copy = spec.clone();
    copy.name = duplicate_name(&spec.name, taken);
    copy.labels = copy
        .labels
        .into_iter()
        .filter(|(key, _)| !key.starts_with("com.docker.compose."))
        .collect::<BTreeMap<_, _>>();
    copy.mounts.retain(|m| {
        m.kind != MountKind::Volume
            || m.volume_name
                .as_deref()
                .is_none_or(|name| !crate::services::confirm::is_anonymous_volume(name))
    });
    copy
}

/// Why `name` cannot be a container's name, or `None`.
#[must_use]
pub fn rename_problem(current: &str, name: &str, taken: &[&str]) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return Some("Write a name".to_string());
    }
    if name == current {
        return Some("That is already its name".to_string());
    }
    if !valid_name(name) {
        return Some("Use letters, digits, _ . and -, starting with a letter or digit".to_string());
    }
    if taken.contains(&name) {
        return Some(format!("{name} is already used on this engine"));
    }
    None
}
