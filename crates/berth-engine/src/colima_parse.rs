use crate::EngineError;

#[derive(Debug, serde::Deserialize)]
struct ListEntry {
    name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    arch: String,
    #[serde(default)]
    cpus: u16,
    #[serde(default)]
    memory: u64,
    #[serde(default)]
    disk: u64,
    #[serde(default)]
    runtime: String,
}

fn parse_runtime(runtime: &str, what: &str) -> Result<berth_core::Runtime, EngineError> {
    match runtime {
        "docker" | "" => Ok(berth_core::Runtime::Docker),
        "containerd" => Ok(berth_core::Runtime::Containerd),
        "incus" => Ok(berth_core::Runtime::Incus),
        other => Err(EngineError::Parse {
            what: what.to_string(),
            message: format!("unknown runtime {other:?}"),
        }),
    }
}

impl ListEntry {
    fn into_profile(self) -> Result<berth_core::ColimaProfile, EngineError> {
        Ok(berth_core::ColimaProfile {
            running: self.status.eq_ignore_ascii_case("running"),
            name: self.name,
            arch: berth_core::Arch::parse(&self.arch),
            cpus: self.cpus,
            memory: bytesize::ByteSize(self.memory),
            disk: bytesize::ByteSize(self.disk),
            runtime: parse_runtime(&self.runtime, "colima list")?,
            vm_type: None,
            rosetta: None,
            mount_type: None,
            docker_socket: None,
        })
    }
}

/// NDJSON (one profile object per line) or a JSON array of them.
pub fn parse_list(stdout: &str) -> Result<Vec<berth_core::ColimaProfile>, EngineError> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let entries: Vec<ListEntry> = if trimmed.starts_with('[') {
        serde_json::from_str(trimmed).map_err(|e| EngineError::Parse {
            what: "colima list".to_string(),
            message: e.to_string(),
        })?
    } else {
        trimmed
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).map_err(|e| EngineError::Parse {
                    what: "colima list".to_string(),
                    message: e.to_string(),
                })
            })
            .collect::<Result<_, _>>()?
    };
    entries.into_iter().map(ListEntry::into_profile).collect()
}

#[derive(Debug, serde::Deserialize)]
struct StatusEntry {
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    arch: String,
    #[serde(default)]
    runtime: String,
    #[serde(default)]
    mount_type: Option<String>,
    #[serde(default)]
    docker_socket: Option<String>,
    #[serde(default)]
    cpu: u16,
    #[serde(default)]
    memory: u64,
    #[serde(default)]
    disk: u64,
}

pub fn parse_status(stdout: &str) -> Result<berth_core::ColimaProfile, EngineError> {
    let entry: StatusEntry =
        serde_json::from_str(stdout.trim()).map_err(|e| EngineError::Parse {
            what: "colima status".to_string(),
            message: e.to_string(),
        })?;
    Ok(berth_core::ColimaProfile {
        name: entry.display_name,
        // `colima status` only succeeds for a running profile.
        running: true,
        arch: berth_core::Arch::parse(&entry.arch),
        cpus: entry.cpu,
        memory: bytesize::ByteSize(entry.memory),
        disk: bytesize::ByteSize(entry.disk),
        runtime: parse_runtime(&entry.runtime, "colima status")?,
        vm_type: None,
        rosetta: None,
        mount_type: entry.mount_type,
        docker_socket: entry
            .docker_socket
            .map(|host| std::path::PathBuf::from(host.strip_prefix("unix://").unwrap_or(&host))),
    })
}

#[derive(Debug, Default, serde::Deserialize)]
struct ColimaYaml {
    rosetta: Option<bool>,
    #[serde(rename = "vmType")]
    vm_type: Option<String>,
}

pub fn apply_yaml(profile: &mut berth_core::ColimaProfile, colima_yaml: &str) {
    let Ok(yaml) = serde_norway::from_str::<ColimaYaml>(colima_yaml) else {
        return;
    };
    if yaml.rosetta.is_some() {
        profile.rosetta = yaml.rosetta;
    }
    if let Some(vm_type) = yaml.vm_type {
        profile.vm_type = Some(match vm_type.as_str() {
            "vz" => berth_core::VmType::Vz,
            "qemu" => berth_core::VmType::Qemu,
            "krunkit" => berth_core::VmType::Krunkit,
            other => berth_core::VmType::Other(other.to_string()),
        });
    }
}

pub fn start_args(
    profile: &str,
    opts: &berth_core::ColimaStartOptions,
    macos: bool,
) -> Vec<String> {
    let mut args = vec!["start".to_string(), "-p".to_string(), profile.to_string()];
    if let Some(cpus) = opts.cpus {
        args.push("--cpus".to_string());
        args.push(cpus.to_string());
    }
    if let Some(gib) = opts.memory_gib {
        args.push("--memory".to_string());
        args.push(gib.to_string());
    }
    if let Some(gib) = opts.disk_gib {
        args.push("--disk".to_string());
        args.push(gib.to_string());
    }
    if macos {
        if let Some(vm_type) = &opts.vm_type {
            args.push("--vm-type".to_string());
            args.push(match vm_type {
                berth_core::VmType::Vz => "vz".to_string(),
                berth_core::VmType::Qemu => "qemu".to_string(),
                berth_core::VmType::Krunkit => "krunkit".to_string(),
                berth_core::VmType::Other(s) => s.clone(),
            });
        }
        match opts.rosetta {
            Some(true) => args.push("--vz-rosetta".to_string()),
            Some(false) => args.push("--vz-rosetta=false".to_string()),
            None => {}
        }
    }
    if let Some(arch) = &opts.arch {
        args.push("--arch".to_string());
        args.push(match arch {
            berth_core::Arch::Aarch64 => "aarch64".to_string(),
            berth_core::Arch::X86_64 => "x86_64".to_string(),
            berth_core::Arch::Other(s) => s.clone(),
        });
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST_FIXTURE: &str = include_str!("../../../fixtures/colima/list.json");
    const STATUS_FIXTURE: &str = include_str!("../../../fixtures/colima/status-default.json");
    const YAML_FIXTURE: &str = include_str!("../../../fixtures/colima/colima-default.yaml");

    #[test]
    fn colima_parse_list_ndjson_fixture() {
        let profiles = parse_list(LIST_FIXTURE).expect("parse list");
        assert_eq!(profiles.len(), 1);
        let p = &profiles[0];
        assert_eq!(p.name, "default");
        assert!(p.running);
        assert_eq!(p.arch, berth_core::Arch::Aarch64);
        assert_eq!(p.cpus, 2);
        assert_eq!(p.memory, bytesize::ByteSize(8_589_934_592));
        assert_eq!(p.disk, bytesize::ByteSize(64_424_509_440));
        assert_eq!(p.runtime, berth_core::Runtime::Docker);
    }

    #[test]
    fn colima_parse_list_array_form() {
        let array = format!("[{}]", LIST_FIXTURE.trim());
        let profiles = parse_list(&array).expect("parse array");
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "default");
    }

    #[test]
    fn colima_parse_list_stopped_and_containerd_profiles() {
        let ndjson = concat!(
            "{\"name\":\"default\",\"status\":\"Running\",\"arch\":\"aarch64\",\"cpus\":2,\"memory\":8589934592,\"disk\":64424509440,\"runtime\":\"docker\"}\n",
            "{\"name\":\"work\",\"status\":\"Stopped\",\"arch\":\"x86_64\",\"cpus\":4,\"memory\":4294967296,\"disk\":21474836480,\"runtime\":\"containerd\"}\n"
        );
        let profiles = parse_list(ndjson).expect("parse two");
        assert_eq!(profiles.len(), 2);
        assert!(!profiles[1].running);
        assert_eq!(profiles[1].arch, berth_core::Arch::X86_64);
        assert_eq!(profiles[1].runtime, berth_core::Runtime::Containerd);
    }

    #[test]
    fn colima_parse_list_empty_and_garbage() {
        assert_eq!(parse_list("  \n").expect("empty"), vec![]);
        assert!(matches!(
            parse_list("not json at all"),
            Err(EngineError::Parse { .. })
        ));
    }

    #[test]
    fn colima_parse_status_fixture() {
        let p = parse_status(STATUS_FIXTURE).expect("parse status");
        assert!(p.running);
        assert_eq!(p.cpus, 2);
        assert_eq!(p.mount_type.as_deref(), Some("virtiofs"));
        let socket = p.docker_socket.expect("socket");
        assert!(
            socket
                .to_string_lossy()
                .ends_with(".colima/default/docker.sock"),
            "{}",
            socket.display()
        );
    }

    #[test]
    fn colima_apply_yaml_reads_rosetta_and_vm_type() {
        let mut p = parse_list(LIST_FIXTURE).expect("list").remove(0);
        apply_yaml(&mut p, YAML_FIXTURE);
        assert_eq!(p.rosetta, Some(false));
        assert_eq!(p.vm_type, Some(berth_core::VmType::Vz));
    }

    #[test]
    fn colima_apply_yaml_qemu_and_garbage() {
        let mut p = parse_list(LIST_FIXTURE).expect("list").remove(0);
        apply_yaml(&mut p, "vmType: qemu\nrosetta: true\n");
        assert_eq!(p.vm_type, Some(berth_core::VmType::Qemu));
        assert_eq!(p.rosetta, Some(true));
        apply_yaml(&mut p, "::: not yaml [");
        assert_eq!(
            p.vm_type,
            Some(berth_core::VmType::Qemu),
            "garbage leaves values alone"
        );
    }

    fn full_opts() -> berth_core::ColimaStartOptions {
        berth_core::ColimaStartOptions {
            cpus: Some(4),
            memory_gib: Some(8),
            disk_gib: Some(60),
            vm_type: Some(berth_core::VmType::Vz),
            rosetta: Some(true),
            arch: Some(berth_core::Arch::Aarch64),
        }
    }

    #[test]
    fn colima_start_args_full_macos() {
        let args = start_args("default", &full_opts(), true);
        assert_eq!(
            args,
            [
                "start",
                "-p",
                "default",
                "--cpus",
                "4",
                "--memory",
                "8",
                "--disk",
                "60",
                "--vm-type",
                "vz",
                "--vz-rosetta",
                "--arch",
                "aarch64"
            ]
        );
    }

    #[test]
    fn colima_start_args_non_macos_omits_vz_and_rosetta() {
        let args = start_args("default", &full_opts(), false);
        assert!(
            !args
                .iter()
                .any(|a| a.contains("vm-type") || a.contains("rosetta")),
            "{args:?}"
        );
    }

    #[test]
    fn colima_start_args_minimal_and_rosetta_off() {
        let empty = berth_core::ColimaStartOptions {
            cpus: None,
            memory_gib: None,
            disk_gib: None,
            vm_type: None,
            rosetta: None,
            arch: None,
        };
        assert_eq!(start_args("work", &empty, true), ["start", "-p", "work"]);
        let off = berth_core::ColimaStartOptions {
            rosetta: Some(false),
            ..empty
        };
        assert_eq!(
            start_args("work", &off, true),
            ["start", "-p", "work", "--vz-rosetta=false"]
        );
    }
}
