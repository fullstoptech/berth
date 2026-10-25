//! Builders for the rule tests: facts around the fixtures in `fixtures/problems/<kind>/`.

use std::path::{Path, PathBuf};

use berth_core::{
    Arch, Container, ContainerDetail, ContainerId, ContainerState, Engine, EngineId, EngineKind,
    EngineState, EnvVar, ImageRef, Limits, OwnerId, Project, ProjectId, ProjectSource,
    ProjectStatus, ResourceCounts,
};
use chrono::{DateTime, Utc};

use super::{ContainerFacts, ProblemInput, ProjectFacts};

pub const ENGINE: &str = "colima:default";

pub fn fixture(kind: &str, file: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/problems")
        .join(kind)
        .join(file);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {} unreadable: {e}", path.display()))
}

pub fn lines(kind: &str, file: &str) -> Vec<String> {
    fixture(kind, file)
        .lines()
        .map(ToString::to_string)
        .collect()
}

pub fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_790_000_000, 0).expect("timestamp")
}

macro_rules! json {
    ($t:ty, $text:expr $(,)?) => {
        serde_json::from_str::<$t>($text).expect("test json")
    };
}

pub fn engine() -> Engine {
    Engine {
        id: EngineId(ENGINE.to_string()),
        name: "Colima".to_string(),
        kind: EngineKind::Colima {
            profile: "default".to_string(),
        },
        state: EngineState::Running,
        docker_host: Some("unix:///Users/me/.colima/default/docker.sock".to_string()),
        docker_version: Some("29.5.2".to_string()),
        resources: Some(json!(
            berth_core::engine::VmResources,
            r#"{"cpus":2,"memory":8589934592,"disk":64424509440,"arch":"Aarch64","vm_type":"Vz",
                "rosetta":false,"runtime":"Docker","mount_type":"virtiofs"}"#,
        )),
        counts: ResourceCounts::default(),
        owner: OwnerId::Local,
    }
}

pub fn input() -> ProblemInput {
    ProblemInput {
        now: now(),
        engine: engine(),
        host_arch: Arch::Aarch64,
        containers: Vec::new(),
        projects: Vec::new(),
        disk: None,
        taken_ports: Vec::new(),
    }
}

pub fn project_id(name: &str) -> ProjectId {
    ProjectId(format!("p-{name}"))
}

pub fn project(name: &str) -> Project {
    let folder = PathBuf::from(format!("/work/{name}"));
    Project {
        id: project_id(name),
        name: name.to_string(),
        folder: Some(folder.clone()),
        compose_files: vec![folder.join("compose.yaml")],
        engine: EngineId(ENGINE.to_string()),
        source: ProjectSource::Compose,
        services: Vec::new(),
        git: None,
        status: ProjectStatus::Stopped,
        pinned: false,
    }
}

pub fn project_facts(name: &str, compose: Option<&str>, failure: &[String]) -> ProjectFacts {
    ProjectFacts {
        project: project(name),
        compose_text: compose.map(ToString::to_string),
        env_text: None,
        compose_error: None,
        failure_lines: failure.to_vec(),
    }
}

/// A container of the project `project` (a Compose service), or a loose one when `project` is empty.
pub fn container(
    name: &str,
    service: Option<&str>,
    project: &str,
    state: ContainerState,
) -> Container {
    Container {
        id: ContainerId(format!("{:0<64}", hex_of(name))),
        engine: EngineId(ENGINE.to_string()),
        name: name.to_string(),
        project: (!project.is_empty()).then(|| project_id(project)),
        compose_project: (!project.is_empty()).then(|| project.to_string()),
        service: service.map(ToString::to_string),
        image: ImageRef {
            reference: "alpine:3".to_string(),
            id: None,
        },
        command: None,
        state,
        health: None,
        exit_code: None,
        restart_count: 0,
        created: now(),
        started: None,
        ports: Vec::new(),
        mounts: Vec::new(),
        networks: Vec::new(),
        labels: std::collections::BTreeMap::new(),
        stats: None,
        platform: None,
    }
}

fn hex_of(name: &str) -> String {
    name.bytes().map(|b| format!("{b:02x}")).collect()
}

pub fn facts(container: Container) -> ContainerFacts {
    ContainerFacts {
        container,
        detail: None,
        error_lines: Vec::new(),
        last_lines: Vec::new(),
    }
}

/// Adds the detail the engine would build from this inspect JSON: env (masked in fixtures),
/// memory limit and health log.
pub fn with_inspect(mut cf: ContainerFacts, inspect: &str) -> ContainerFacts {
    let inspect = json!(serde_json::Value, inspect);
    let memory = inspect
        .pointer("/HostConfig/Memory")
        .and_then(serde_json::Value::as_u64)
        .filter(|m| *m > 0);
    let limits = json!(
        Limits,
        &format!(
            r#"{{"cpus":null,"memory":{},"memory_swap":null}}"#,
            memory.map_or_else(|| "null".to_string(), |m| m.to_string())
        )
    );
    let env = inspect
        .pointer("/Config/Env")
        .and_then(serde_json::Value::as_array)
        .map(|vars| {
            vars.iter()
                .filter_map(|v| v.as_str())
                .filter_map(|v| v.split_once('='))
                .map(|(k, v)| EnvVar {
                    key: k.to_string(),
                    value: v.to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let health_log = inspect
        .pointer("/State/Health/Log")
        .and_then(serde_json::Value::as_array)
        .map(|runs| {
            runs.iter()
                .map(|r| berth_core::HealthCheckResult {
                    start: now(),
                    exit_code: r
                        .get("ExitCode")
                        .and_then(serde_json::Value::as_i64)
                        .and_then(|c| i32::try_from(c).ok())
                        .unwrap_or(0),
                    output: r
                        .get("Output")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    cf.detail = Some(ContainerDetail {
        container: cf.container.clone(),
        entrypoint: None,
        cmd: None,
        env,
        working_dir: None,
        user: None,
        restart: berth_core::RestartPolicy::No,
        limits,
        health_log,
        inspect,
    });
    cf
}
