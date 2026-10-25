//! Live problem rules against the broken fixture set (P5-T7 done-when). Run with:
//!   scripts/fx-up.sh --broken && scripts/it.sh -p berth-rules problems_it
//!
//! Facts come from `docker inspect`, `docker logs` and one `docker compose create` per service of the
//! `berthfx-broken` project (it creates what is missing and reports what cannot be made:
//! the image that does not exist, the folder that does not exist). Only containers of that
//! project are read; nothing is started or stopped. Inspect output holds env values,
//! so nothing of it is printed.

// Helpers sit outside the #[test] fn, so clippy.toml's test allowance does not reach them.
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use berth_core::{
    Arch, Container, ContainerDetail, ContainerId, ContainerState, Engine, EngineId, EngineKind,
    EngineState, EnvVar, Health, HealthCheckResult, ImageRef, Limits, Mount, MountKind, OwnerId,
    Problem, ProblemKind, Project, ProjectId, ProjectSource, ProjectStatus, ResourceCounts,
    RestartPolicy, Subject,
};
use berth_rules::problems::{evaluate, ContainerFacts, ProblemInput, ProjectFacts};
use serde_json::Value;

const ENGINE: &str = "colima:default";
const PROJECT: &str = "berthfx-broken";

fn broken_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/compose/berthfx-broken")
}

fn docker(args: &[&str]) -> (bool, String, String) {
    let out = Command::new("docker")
        .args(args)
        .output()
        .expect("docker runs");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn text<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer).and_then(Value::as_str)
}

fn state_of(status: &str) -> ContainerState {
    match status {
        "running" => ContainerState::Running,
        "paused" => ContainerState::Paused,
        "restarting" => ContainerState::Restarting,
        "removing" => ContainerState::Removing,
        "exited" => ContainerState::Exited,
        "dead" => ContainerState::Dead,
        _ => ContainerState::Created,
    }
}

fn health_of(v: &Value) -> Option<Health> {
    match text(v, "/State/Health/Status")? {
        "healthy" => Some(Health::Healthy),
        "unhealthy" => Some(Health::Unhealthy),
        "starting" => Some(Health::Starting),
        _ => None,
    }
}

fn facts_of(v: &Value, project: &ProjectId) -> ContainerFacts {
    let label = |key: &str| text(v, &format!("/Config/Labels/{key}")).map(ToString::to_string);
    let name = text(v, "/Name")
        .unwrap_or("")
        .trim_start_matches('/')
        .to_string();
    let id = ContainerId(text(v, "/Id").unwrap_or("").to_string());
    let mounts: Vec<Mount> = v
        .pointer("/Mounts")
        .and_then(Value::as_array)
        .map(|ms| {
            ms.iter()
                .map(|m| Mount {
                    kind: if text(m, "/Type") == Some("bind") {
                        MountKind::Bind
                    } else {
                        MountKind::Volume
                    },
                    source: text(m, "/Source").unwrap_or("").to_string(),
                    destination: text(m, "/Destination").unwrap_or("").to_string(),
                    read_only: m.get("RW").and_then(Value::as_bool) == Some(false),
                    volume_name: None,
                })
                .collect()
        })
        .unwrap_or_default();
    let container = Container {
        id: id.clone(),
        engine: EngineId(ENGINE.to_string()),
        name,
        project: Some(project.clone()),
        compose_project: label("com.docker.compose.project"),
        service: label("com.docker.compose.service"),
        image: ImageRef {
            reference: text(v, "/Config/Image").unwrap_or("").to_string(),
            id: None,
        },
        command: None,
        state: state_of(text(v, "/State/Status").unwrap_or("created")),
        health: health_of(v),
        exit_code: v
            .pointer("/State/ExitCode")
            .and_then(Value::as_i64)
            .and_then(|c| i32::try_from(c).ok()),
        restart_count: 0,
        created: chrono::Utc::now(),
        started: None,
        ports: Vec::new(),
        mounts,
        networks: Vec::new(),
        labels: std::collections::BTreeMap::new(),
        stats: None,
        platform: None,
    };
    let memory = v
        .pointer("/HostConfig/Memory")
        .and_then(Value::as_u64)
        .filter(|m| *m > 0);
    let limits: Limits = serde_json::from_value(serde_json::json!({
        "cpus": null, "memory": memory, "memory_swap": null
    }))
    .expect("limits");
    let env = v
        .pointer("/Config/Env")
        .and_then(Value::as_array)
        .map(|vars| {
            vars.iter()
                .filter_map(|e| e.as_str()?.split_once('='))
                .map(|(k, val)| EnvVar {
                    key: k.to_string(),
                    value: val.to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let health_log = v
        .pointer("/State/Health/Log")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .map(|r| HealthCheckResult {
                    start: chrono::Utc::now(),
                    exit_code: r
                        .get("ExitCode")
                        .and_then(Value::as_i64)
                        .and_then(|c| i32::try_from(c).ok())
                        .unwrap_or(0),
                    output: r
                        .get("Output")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let (ok, out, err) = docker(&["logs", "--tail", "40", &id.0]);
    assert!(ok, "docker logs failed: {err}");
    let last_lines: Vec<String> = out
        .lines()
        .chain(err.lines())
        .map(ToString::to_string)
        .collect();
    let error_lines = last_lines
        .iter()
        .filter(|l| l.to_lowercase().contains("error"))
        .cloned()
        .collect();
    ContainerFacts {
        detail: Some(ContainerDetail {
            container: container.clone(),
            entrypoint: None,
            cmd: None,
            env,
            working_dir: None,
            user: None,
            restart: RestartPolicy::No,
            limits,
            health_log,
            inspect: v.clone(),
        }),
        container,
        error_lines,
        last_lines,
    }
}

fn engine() -> Engine {
    Engine {
        id: EngineId(ENGINE.to_string()),
        name: "Colima".to_string(),
        kind: EngineKind::Colima {
            profile: "default".to_string(),
        },
        state: EngineState::Running,
        docker_host: None,
        docker_version: None,
        resources: None,
        counts: ResourceCounts::default(),
        owner: OwnerId::Local,
    }
}

fn gather() -> ProblemInput {
    let folder = broken_dir().canonicalize().expect("fixture folder");
    let project_id = ProjectId(format!("it-{PROJECT}"));
    let compose = folder.join("compose.yaml");
    // Compose stops a whole `create` at the first service it cannot make (the pull of the
    // image that does not exist), so the folder that does not exist would never be
    // reported. Create each service on its own, as `scripts/fx-up.sh --broken` does, and never
    // recreate: a container that never started (the port clash) holds the engine's error.
    let compose_args = [
        "compose",
        "--project-directory",
        &folder.to_string_lossy(),
        "-f",
        &compose.to_string_lossy(),
        "-p",
        PROJECT,
    ];
    let (_, services, _) = docker(&[&compose_args[..], &["config", "--services"]].concat());
    let mut create_err = String::new();
    for service in services.split_whitespace() {
        let (_, _, err) =
            docker(&[&compose_args[..], &["create", "--no-recreate", service]].concat());
        create_err.push_str(&err);
    }
    let failure_lines: Vec<String> = create_err
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(ToString::to_string)
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let (ok, ids, err) = docker(&[
        "ps",
        "-aq",
        "--filter",
        &format!("label=com.docker.compose.project={PROJECT}"),
        "--filter",
        "label=dev.berth.fixture=1",
    ]);
    assert!(ok, "docker ps failed: {err}");
    let ids: Vec<&str> = ids.split_whitespace().collect();
    assert!(!ids.is_empty(), "run scripts/fx-up.sh --broken first");
    let mut args = vec!["inspect"];
    args.extend(ids.iter().copied());
    let (ok, json, err) = docker(&args);
    assert!(ok, "docker inspect failed: {err}");
    let inspected: Vec<Value> = serde_json::from_str(&json).expect("inspect json");

    ProblemInput {
        now: chrono::Utc::now(),
        engine: engine(),
        host_arch: Arch::Aarch64,
        containers: inspected.iter().map(|v| facts_of(v, &project_id)).collect(),
        projects: vec![ProjectFacts {
            project: Project {
                id: project_id,
                name: PROJECT.to_string(),
                folder: Some(folder),
                compose_files: vec![compose.clone()],
                engine: EngineId(ENGINE.to_string()),
                source: ProjectSource::Compose,
                services: Vec::new(),
                git: None,
                status: ProjectStatus::Failing,
                pinned: false,
            },
            compose_text: Some(std::fs::read_to_string(&compose).expect("compose text")),
            env_text: None,
            compose_error: None,
            failure_lines,
        }],
        disk: None,
        taken_ports: Vec::new(),
    }
}

/// The service a problem is about: the container's own, else named by the problem's text.
fn about_service(input: &ProblemInput, p: &Problem, service: &str) -> bool {
    match &p.subject {
        Subject::Container(id) => input
            .containers
            .iter()
            .find(|c| &c.container.id == id)
            .is_some_and(|c| c.container.service.as_deref() == Some(service)),
        _ => {
            p.title.contains(service)
                || p.fixes.iter().any(|f| f.label.contains(service))
                || p.explanation.contains(service)
        }
    }
}

fn missing(input: &ProblemInput, found: &[Problem]) -> Vec<&'static str> {
    let has = |wanted: &dyn Fn(&ProblemKind) -> bool, service: &str| {
        found
            .iter()
            .any(|p| wanted(&p.kind) && about_service(input, p, service))
    };
    let mut out = Vec::new();
    if !has(
        &|k| matches!(k, ProblemKind::PortAllocated { port: 56080 }),
        "portclash",
    ) {
        out.push("PortAllocated for portclash");
    }
    if !has(
        &|k| matches!(k, ProblemKind::ImageNotFound { reference } if reference == "berthfx/does-not-exist:1"),
        "badimage",
    ) {
        out.push("ImageNotFound for badimage");
    }
    if !has(
        &|k| matches!(k, ProblemKind::ExitedNoLogs { code: 1 }),
        "crash",
    ) {
        out.push("ExitedNoLogs for crash");
    }
    if !has(&|k| matches!(k, ProblemKind::OutOfMemory), "oom") {
        out.push("OutOfMemory for oom");
    }
    if !found
        .iter()
        .any(|p| matches!(&p.kind, ProblemKind::EnvVarMissing { var } if var == "BERTHFX_MISSING"))
    {
        out.push("EnvVarMissing for needsvar");
    }
    if !has(&|k| matches!(k, ProblemKind::Unhealthy), "unhealthy") {
        out.push("Unhealthy for unhealthy");
    }
    if !has(
        &|k| matches!(k, ProblemKind::BindPathMissing { path } if path == "/nonexistent-berthfx-path"),
        "badmount",
    ) {
        out.push("BindPathMissing for badmount");
    }
    out
}

#[test]
#[ignore = "needs the live Colima engine and `scripts/fx-up.sh --broken`; run with BERTH_IT=1 via scripts/it.sh"]
fn problems_it_the_broken_fixtures_raise_their_problems() {
    if std::env::var("BERTH_IT").as_deref() != Ok("1") {
        return;
    }
    // The unhealthy check needs a few probes and the memory hog a moment to be killed.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let input = gather();
        let found = evaluate(&input);
        let absent = missing(&input, &found);
        if absent.is_empty() {
            for p in &found {
                assert!(!p.fixes.is_empty(), "{} has no fix", p.title);
            }
            return;
        }
        if Instant::now() > deadline {
            let titles: Vec<&str> = found.iter().map(|p| p.title.as_str()).collect();
            panic!("missing: {absent:?}; raised: {titles:?}");
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}
