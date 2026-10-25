use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::ids::{EngineId, OwnerId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EngineKind {
    Colima {
        profile: String,
    },
    Remote {
        transport: RemoteTransport,
    },
    Socket {
        path: std::path::PathBuf,
        context: Option<String>,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RemoteTransport {
    Ssh {
        host: String,
        user: Option<String>,
        port: Option<u16>,
    },
    Tcp {
        url: Url,
        tls: Option<TlsFiles>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TlsFiles {
    pub ca: std::path::PathBuf,
    pub cert: std::path::PathBuf,
    pub key: std::path::PathBuf,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EngineState {
    Running,
    Stopped,
    Starting,
    Stopping,
    Unreachable {
        reason: String,
        last_seen: Option<DateTime<Utc>>,
    },
    Unsupported {
        reason: String,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Engine {
    pub id: EngineId,
    pub name: String,
    pub kind: EngineKind,
    pub state: EngineState,
    pub docker_host: Option<String>,
    pub docker_version: Option<String>,
    pub resources: Option<VmResources>,
    pub counts: ResourceCounts,
    pub owner: OwnerId,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VmResources {
    pub cpus: u16,
    pub memory: ByteSize,
    pub disk: ByteSize,
    pub arch: Arch,
    pub vm_type: VmType,
    pub rosetta: bool,
    pub runtime: Runtime,
    pub mount_type: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Arch {
    Aarch64,
    X86_64,
    Other(String),
}
impl Arch {
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "aarch64" | "arm64" => Self::Aarch64,
            "x86_64" | "amd64" => Self::X86_64,
            other => Self::Other(other.to_string()),
        }
    }

    #[must_use]
    pub fn docker_name(&self) -> &str {
        match self {
            Self::Aarch64 => "arm64",
            Self::X86_64 => "amd64",
            Self::Other(s) => s,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VmType {
    Vz,
    Qemu,
    Krunkit,
    Other(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Runtime {
    Docker,
    Containerd,
    Incus,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceCounts {
    pub containers_running: u32,
    pub containers_total: u32,
    pub images: u32,
    pub volumes: u32,
    pub networks: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub os: String,
    pub kernel: String,
    pub docker_version: String,
    pub api_version: String,
    pub compose_version: Option<String>,
    pub storage_driver: String,
    pub ncpu: u32,
    pub mem_total: ByteSize,
    pub arch: Arch,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageGroup {
    pub count: u32,
    pub size: ByteSize,
    pub reclaimable: ByteSize,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiskUsage {
    pub images: UsageGroup,
    pub containers: UsageGroup,
    pub volumes: UsageGroup,
    pub build_cache: UsageGroup,
    pub disk_total: Option<ByteSize>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColimaProfile {
    pub name: String,
    pub running: bool,
    pub arch: Arch,
    pub cpus: u16,
    pub memory: ByteSize,
    pub disk: ByteSize,
    pub runtime: Runtime,
    pub vm_type: Option<VmType>,
    pub rosetta: Option<bool>,
    pub mount_type: Option<String>,
    pub docker_socket: Option<std::path::PathBuf>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColimaStartOptions {
    pub cpus: Option<u16>,
    pub memory_gib: Option<u32>,
    pub disk_gib: Option<u32>,
    pub vm_type: Option<VmType>,
    pub rosetta: Option<bool>,
    pub arch: Option<Arch>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_parse_aliases() {
        assert_eq!(Arch::parse("aarch64"), Arch::Aarch64);
        assert_eq!(Arch::parse("arm64"), Arch::Aarch64);
        assert_eq!(Arch::parse("x86_64"), Arch::X86_64);
        assert_eq!(Arch::parse("amd64"), Arch::X86_64);
        assert_eq!(Arch::parse("riscv64"), Arch::Other("riscv64".to_string()));
    }

    #[test]
    fn arch_docker_names() {
        assert_eq!(Arch::Aarch64.docker_name(), "arm64");
        assert_eq!(Arch::X86_64.docker_name(), "amd64");
        assert_eq!(Arch::Other("riscv64".to_string()).docker_name(), "riscv64");
    }
}
