#![allow(clippy::expect_used)]
//! Live Compose runner against the berthfx fixture project. Run with:
//!   scripts/it.sh -p berth-engine compose_it
//! (which sets BERTH_IT=1 and brings the fixtures up first).

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
