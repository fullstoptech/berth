#![allow(clippy::expect_used)]
//! Live Compose runner against the berthfx fixture project. Run with:
//!   scripts/it.sh -p berth-engine compose_it
//! (which sets BERTH_IT=1 and brings the fixtures up first).

mod common;

use berth_engine::colima::ColimaControl;
use berth_engine::{CliCompose, ColimaCli, ComposeRunner, EngineError};
use futures::StreamExt;

fn fixture_folder() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/compose/berthfx")
}

async fn cli_and_target() -> (CliCompose, berth_core::ComposeTarget) {
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima = ColimaCli::new(search.clone());
    let status = colima.status("default").await.expect("colima status");
    let socket = status.docker_socket.expect("docker socket");
    let folder = fixture_folder();
    let target = berth_core::ComposeTarget {
        engine: berth_core::EngineId("colima:default".to_string()),
        docker_host: format!("unix://{}", socket.display()),
        project_name: "berthfx".to_string(),
        files: vec![folder.join("compose.yaml")],
        folder,
    };
    (CliCompose::new(search), target)
}

async fn run_to_events(
    cli: &CliCompose,
    target: &berth_core::ComposeTarget,
    cmd: berth_core::ComposeCommand,
) -> Vec<berth_core::ComposeEvent> {
    let items: Vec<_> = cli
        .run(target, cmd, tokio_util::sync::CancellationToken::new())
        .collect()
        .await;
    items
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("compose run stream has no errors")
}

fn assert_done_zero(events: &[berth_core::ComposeEvent]) {
    assert_eq!(
        events.last(),
        Some(&berth_core::ComposeEvent::Done { code: 0 }),
        "{events:?}"
    );
}

async fn web_running() -> bool {
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let docker = search.find("docker").expect("docker binary");
    let args: Vec<std::ffi::OsString> = [
        "ps",
        "--filter",
        "name=berthfx-web-1",
        "--format",
        "{{.Names}}",
    ]
    .iter()
    .map(std::ffi::OsString::from)
    .collect();
    let output = berth_sys::run_capture(
        &docker,
        &args,
        berth_sys::SpawnOpts {
            path: Some(search.path_var()),
            ..Default::default()
        },
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("docker ps");
    output.stdout.contains("berthfx-web-1")
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn compose_it_restart_stop_start_config_validate() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    common::settle_fixture().await;
    let (cli, target) = cli_and_target().await;

    // Restart yields Progress events and Done{code:0}.
    let events = run_to_events(
        &cli,
        &target,
        berth_core::ComposeCommand::Restart {
            services: vec!["web".to_string()],
        },
    )
    .await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, berth_core::ComposeEvent::Progress { .. })),
        "restart emitted no progress: {events:?}"
    );
    assert_done_zero(&events);

    // Stop then Start leaves the service running.
    let stopped = run_to_events(
        &cli,
        &target,
        berth_core::ComposeCommand::Stop {
            services: vec!["web".to_string()],
        },
    )
    .await;
    assert_done_zero(&stopped);
    assert!(!web_running().await, "web still running after Stop");
    let started = run_to_events(
        &cli,
        &target,
        berth_core::ComposeCommand::Start {
            services: vec!["web".to_string()],
        },
    )
    .await;
    assert_done_zero(&started);
    assert!(web_running().await, "web not running after Start");

    // config returns the 4 fixture services.
    let config = cli.config(&target).await.expect("compose config");
    let services = config
        .get("services")
        .and_then(serde_json::Value::as_object)
        .expect("config has services");
    assert_eq!(services.len(), 4, "{services:?}");
    for name in ["web", "db", "cache", "logger"] {
        assert!(services.contains_key(name), "missing {name}: {services:?}");
    }

    // validate passes on the real fixture and names the line on a broken copy.
    cli.validate(&target.folder, &target.files)
        .await
        .expect("fixture validates");
    let dir = tempfile::tempdir().expect("tempdir");
    let broken = dir.path().join("compose.yaml");
    let text = std::fs::read_to_string(target.folder.join("compose.yaml")).expect("read fixture");
    // One extra space of indent on the first service breaks the mapping.
    let broken_text = text.replacen("\n  web:", "\n   web:", 1);
    assert_ne!(broken_text, text, "indent break applied");
    std::fs::write(&broken, broken_text).expect("write broken copy");
    std::fs::copy(target.folder.join(".env"), dir.path().join(".env")).expect("copy .env");
    let err = cli
        .validate(dir.path(), std::slice::from_ref(&broken))
        .await
        .expect_err("broken indent fails validation");
    match &err {
        EngineError::Docker(message) => assert!(
            message.contains("at L") || message.to_lowercase().contains("line"),
            "validate message names no line: {message}"
        ),
        other => panic!("expected EngineError::Docker, got {other}"),
    }
}

/// `validate_text` runs the docker CLI only: no engine, no container. It is ignored with the other live
/// tests because it needs the `docker` binary and its Compose plugin.
#[tokio::test]
#[ignore = "needs the docker CLI with the Compose plugin; run with BERTH_IT=1"]
async fn compose_it_validate_text_stdin_env_and_missing_dotenv() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let cli = CliCompose::new(search);
    let dir = tempfile::tempdir().expect("tempdir");
    let folder = dir.path();
    let pair = |k: &str, v: &str| (k.to_string(), v.to_string());

    // The text comes from stdin: the folder holds no compose file.
    let plain = "services:\n  web:\n    image: nginx\n";
    cli.validate_text(folder, plain, &[])
        .await
        .expect("plain text validates");

    // New `.env` lines reach Compose as process environment.
    let needs_var =
        "services:\n  web:\n    image: nginx\n    ports:\n      - \"${PORT:?set a port}:80\"\n";
    let err = cli
        .validate_text(folder, needs_var, &[])
        .await
        .expect_err("PORT is required");
    assert!(
        matches!(&err, EngineError::Docker(m) if m.contains("set a port")),
        "{err}"
    );
    cli.validate_text(folder, needs_var, &[pair("PORT", "8080")])
        .await
        .expect("PORT in the process environment satisfies it");

    // Broken YAML fails with Compose's own words.
    let err = cli
        .validate_text(folder, "services:\n  web:\n   image: [oops\n", &[])
        .await
        .expect_err("broken yaml");
    assert!(matches!(err, EngineError::Docker(_)), "{err}");

    // `env_file: .env` with no `.env` on disk passes when the plan adds `.env` lines, and only then.
    let with_env_file = "services:\n  web:\n    image: nginx\n    env_file: .env\n";
    cli.validate_text(folder, with_env_file, &[pair("DB_PASSWORD", "x")])
        .await
        .expect("the plan creates .env");
    let err = cli
        .validate_text(folder, with_env_file, &[])
        .await
        .expect_err("nothing creates .env");
    assert!(
        matches!(&err, EngineError::Docker(m) if m.contains("env file")),
        "{err}"
    );
    // Another missing env file is the user's mistake, plan or not.
    let other = "services:\n  web:\n    image: nginx\n    env_file: .other.env\n";
    let err = cli
        .validate_text(folder, other, &[pair("DB_PASSWORD", "x")])
        .await
        .expect_err("a plan that writes .env does not create .other.env");
    assert!(
        matches!(&err, EngineError::Docker(m) if m.contains(".other.env")),
        "{err}"
    );
    // The folder was only read: no `.env` appeared.
    assert_eq!(std::fs::read_dir(folder).expect("read dir").count(), 0);
}

/// Ids of the engine's containers made from an image id (`sha256:...`, not a name): what the
/// classic builder's steps look like. Running or not.
async fn all_container_ids() -> std::collections::BTreeSet<String> {
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let docker = search.find("docker").expect("docker binary");
    let args: Vec<std::ffi::OsString> =
        ["ps", "-a", "--no-trunc", "--format", "{{.ID}} {{.Image}}"]
            .iter()
            .map(std::ffi::OsString::from)
            .collect();
    let output = berth_sys::run_capture(
        &docker,
        &args,
        berth_sys::SpawnOpts {
            path: Some(search.path_var()),
            ..Default::default()
        },
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("docker ps");
    output
        .stdout
        .lines()
        .filter_map(|l| l.split_once(' '))
        .filter(|(_, image)| image.starts_with("sha256:"))
        .map(|(id, _)| id.to_string())
        .collect()
}

/// A build that fails at its last step must not leave the classic builder's stopped container
/// behind (random name, image `sha256:...`). Uses a temp project named `berthfx-bld-*`; nothing it
/// makes outlives the test except dangling layers.
#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn compose_it_a_failed_build_leaves_no_stopped_container() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    let (cli, mut target) = cli_and_target().await;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("Dockerfile"),
        "FROM alpine:3\nRUN echo berthfx > /a\nRUN echo failing && exit 3\n",
    )
    .expect("Dockerfile");
    std::fs::write(
        dir.path().join("compose.yaml"),
        "services:\n  app:\n    build: .\n    image: berthfx/bld:1\n    labels:\n      dev.berth.fixture: \"1\"\n",
    )
    .expect("compose.yaml");
    target.project_name = "berthfx-bld".to_string();
    target.folder = dir.path().to_path_buf();
    target.files = vec![dir.path().join("compose.yaml")];

    let before = all_container_ids().await;
    let items: Vec<_> = cli
        .run(
            &target,
            berth_core::ComposeCommand::Up {
                services: Vec::new(),
                build: true,
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .collect()
        .await;
    let events: Vec<_> = items.into_iter().filter_map(Result::ok).collect();
    let code = events.iter().find_map(|e| match e {
        berth_core::ComposeEvent::Done { code } => Some(*code),
        _ => None,
    });
    assert!(
        code.is_some_and(|c| c != 0),
        "the build must fail for this test to mean anything: {events:?}"
    );
    let ran_in = events.iter().any(|e| {
        matches!(e, berth_core::ComposeEvent::Line { text, .. } if text.contains("Running in "))
    });
    eprintln!("classic builder in use: {ran_in}");

    let left: Vec<String> = all_container_ids()
        .await
        .difference(&before)
        .cloned()
        .collect();
    assert!(
        left.is_empty(),
        "the failed build left containers behind: {left:?}"
    );
}
