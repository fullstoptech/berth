//! Live grouping check (P1-T7): the four berthfx fixture containers group
//! into one project whose folder is the canonical fixtures/compose/berthfx,
//! and that folder's git branch matches `git branch --show-current`.
//!
//! Container facts come from `docker inspect` of containers labelled
//! `dev.berth.fixture=1` only, mapped by hand (P1-T4's client is not a
//! dependency of this task). Run with `scripts/it.sh -p berth-engine
//! projects_it`.

// Test helpers sit outside the #[test] fn, so clippy.toml's test allowance
// does not reach them.
#![allow(clippy::expect_used)]

mod common;

fn os(args: &[&str]) -> Vec<std::ffi::OsString> {
    args.iter().map(std::ffi::OsString::from).collect()
}

async fn capture(binary: &std::path::Path, args: &[std::ffi::OsString]) -> berth_sys::Output {
    let out = berth_sys::run_capture(
        binary,
        args,
        berth_sys::SpawnOpts::default(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("capture runs");
    assert_eq!(out.code, Some(0), "{binary:?} {args:?}: {}", out.stderr);
    out
}

fn str_at<'a>(v: &'a serde_json::Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for key in path {
        cur = cur.get(key)?;
    }
    cur.as_str()
}

fn container_from_inspect(
    engine: &berth_core::EngineId,
    v: &serde_json::Value,
) -> berth_core::Container {
    let labels = v
        .pointer("/Config/Labels")
        .and_then(serde_json::Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let state = match str_at(v, &["State", "Status"]) {
        Some("running") => berth_core::ContainerState::Running,
        Some("paused") => berth_core::ContainerState::Paused,
        Some("restarting") => berth_core::ContainerState::Restarting,
        Some("removing") => berth_core::ContainerState::Removing,
        Some("created") => berth_core::ContainerState::Created,
        Some("dead") => berth_core::ContainerState::Dead,
        _ => berth_core::ContainerState::Exited,
    };
    let health = match str_at(v, &["State", "Health", "Status"]) {
        Some("healthy") => Some(berth_core::Health::Healthy),
        Some("unhealthy") => Some(berth_core::Health::Unhealthy),
        Some("starting") => Some(berth_core::Health::Starting),
        _ => None,
    };
    let mounts = v
        .get("Mounts")
        .and_then(serde_json::Value::as_array)
        .map(|ms| {
            ms.iter()
                .map(|m| berth_core::Mount {
                    kind: match str_at(m, &["Type"]) {
                        Some("bind") => berth_core::MountKind::Bind,
                        Some("volume") => berth_core::MountKind::Volume,
                        Some("tmpfs") => berth_core::MountKind::Tmpfs,
                        _ => berth_core::MountKind::Other,
                    },
                    source: str_at(m, &["Source"]).unwrap_or_default().to_string(),
                    destination: str_at(m, &["Destination"]).unwrap_or_default().to_string(),
                    read_only: !m
                        .get("RW")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true),
                    volume_name: str_at(m, &["Name"]).map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default();
    let created = str_at(v, &["Created"])
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&chrono::Utc))
        .expect("inspect Created parses");
    berth_core::Container {
        id: berth_core::ContainerId(str_at(v, &["Id"]).expect("Id").to_string()),
        engine: engine.clone(),
        name: str_at(v, &["Name"])
            .expect("Name")
            .trim_start_matches('/')
            .to_string(),
        project: None,
        compose_project: v
            .pointer("/Config/Labels/com.docker.compose.project")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        service: v
            .pointer("/Config/Labels/com.docker.compose.service")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        image: berth_core::ImageRef {
            reference: str_at(v, &["Config", "Image"])
                .unwrap_or_default()
                .to_string(),
            id: str_at(v, &["Image"]).map(str::to_string),
        },
        command: None,
        state,
        health,
        exit_code: v
            .pointer("/State/ExitCode")
            .and_then(serde_json::Value::as_i64)
            .map(|n| i32::try_from(n).unwrap_or(0)),
        restart_count: u32::try_from(
            v.get("RestartCount")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(0),
        created,
        started: None,
        ports: Vec::new(),
        mounts,
        networks: v
            .pointer("/NetworkSettings/Networks")
            .and_then(serde_json::Value::as_object)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default(),
        labels,
        stats: None,
        platform: None,
    }
}

#[tokio::test]
#[ignore = "needs the live engine with berthfx fixtures; run with BERTH_IT=1"]
async fn projects_it_groups_berthfx_and_reads_git_branch() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    common::settle_fixture().await;
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let docker = search.find("docker").expect("docker binary");
    let git = search.find("git").expect("git binary");

    let ids = capture(
        &docker,
        &os(&["ps", "-aq", "--filter", "label=dev.berth.fixture=1"]),
    )
    .await;
    let ids: Vec<&str> = ids.stdout.lines().filter(|l| !l.is_empty()).collect();
    assert!(
        !ids.is_empty(),
        "berthfx fixtures are up (scripts/fx-up.sh)"
    );
    let mut inspect_args = os(&["inspect"]);
    inspect_args.extend(ids.iter().map(std::ffi::OsString::from));
    let inspected = capture(&docker, &inspect_args).await;
    let values: Vec<serde_json::Value> =
        serde_json::from_str(&inspected.stdout).expect("inspect JSON");

    let engine = berth_core::EngineId("colima:default".to_string());
    let containers: Vec<berth_core::Container> = values
        .iter()
        .map(|v| container_from_inspect(&engine, v))
        .filter(|c| c.compose_project.as_deref() == Some("berthfx"))
        .collect();
    assert_eq!(
        containers.len(),
        4,
        "four berthfx containers: {containers:?}"
    );

    let projects = berth_engine::projects::group(&engine, &containers, &[], false);
    assert_eq!(projects.len(), 1, "one project: {projects:?}");
    let project = &projects[0];
    assert_eq!(project.name, "berthfx");
    let total: usize = project.services.iter().map(|s| s.containers.len()).sum();
    assert_eq!(total, 4, "all four containers in the project");

    let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/compose/berthfx")
        .canonicalize()
        .expect("fixture dir exists");
    let folder = project.folder.clone().expect("project folder");
    assert_eq!(
        folder.canonicalize().expect("labelled folder exists"),
        fixture_dir,
        "project folder equals canonical fixtures/compose/berthfx"
    );

    let shown = capture(
        &git,
        &[
            "-C".into(),
            fixture_dir.as_os_str().to_os_string(),
            "branch".into(),
            "--show-current".into(),
        ],
    )
    .await;
    let expected = {
        let b = shown.stdout.trim();
        (!b.is_empty()).then(|| b.to_string())
    };
    let cli = berth_engine::GitCli::new(search);
    let info = cli
        .git_info(&folder, tokio_util::sync::CancellationToken::new())
        .await
        .expect("git info runs")
        .expect("fixture folder is in a git repo");
    println!(
        "project={} folder={} branch={:?} (show-current {:?}) status={:?}",
        project.name,
        folder.display(),
        info.branch,
        expected,
        project.status
    );
    assert_eq!(info.branch, expected);
}
