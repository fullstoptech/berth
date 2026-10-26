use super::*;

use berth_core::{EngineKind, EngineState, HarnessId, HarnessTransport};

fn engine(name: &str, state: EngineState, host: Option<&str>) -> berth_core::Engine {
    berth_core::Engine {
        id: berth_core::EngineId(format!("colima:{name}")),
        name: name.to_string(),
        kind: EngineKind::Colima {
            profile: name.to_string(),
        },
        state,
        docker_host: host.map(str::to_string),
        docker_version: Some("28.4.0".to_string()),
        resources: None,
        counts: berth_core::ResourceCounts::default(),
        owner: berth_core::OwnerId::Local,
    }
}

fn harness(id: &str, name: &str, version: Option<&str>) -> berth_core::Harness {
    berth_core::Harness {
        id: HarnessId(id.to_string()),
        name: name.to_string(),
        binary: PathBuf::from(id),
        version: version.map(str::to_string),
        transport: HarnessTransport::ClaudeStreamJson,
        available: version.is_some(),
    }
}

fn sockets_exist(_: &Path) -> bool {
    true
}

fn running_default() -> berth_core::Engine {
    engine(
        "default",
        EngineState::Running,
        Some("unix:///Users/me/.colima/default/docker.sock"),
    )
}

fn report(probe: &Probe, engines: &[berth_core::Engine]) -> Report {
    evaluate(
        probe,
        engines,
        &[
            harness("claude-code", "Claude Code", Some("2.1.309")),
            harness("muse", "Muse Code", None),
        ],
        &sockets_exist,
    )
}

// --- The rows ---

#[test]
fn first_run_all_good_passes_every_row_and_needs_no_setup() {
    let report = report(&Probe::all_found(), &[running_default()]);
    let colima = report.row("colima").expect("colima row");
    assert_eq!(colima.status, Status::Pass);
    assert_eq!(colima.value, "0.10.3 \u{b7} running");
    assert_eq!(
        report.row("docker").map(|r| r.value.as_str()),
        Some("28.4.0")
    );
    assert_eq!(report.row("compose").map(|r| r.status), Some(Status::Pass));
    assert_eq!(
        report.row("claude-code").map(|r| r.value.as_str()),
        Some("2.1.309")
    );
    assert!(!report.needs_setup());
    assert!(report.start.is_none());
}

#[test]
fn first_run_a_missing_assistant_is_listed_but_never_a_failure() {
    let report = report(&Probe::all_found(), &[running_default()]);
    let muse = report.row("muse").expect("muse row");
    assert_eq!(muse.status, Status::Absent);
    assert_eq!(muse.value, "not found");
    assert!(!report.needs_setup());
}

#[test]
fn first_run_colima_missing_fails_the_row_with_the_install_command_and_needs_setup() {
    let probe = Probe {
        colima: None,
        colima_version: None,
        ..Probe::all_found()
    };
    let report = report(&probe, &[]);
    let colima = report.row("colima").expect("colima row");
    assert_eq!(colima.status, Status::Fail);
    assert_eq!(colima.value, "not found");
    assert!(
        colima
            .hint
            .as_deref()
            .is_some_and(|h| h.contains(INSTALL_COMMAND)),
        "{colima:?}"
    );
    assert!(report.needs_setup());
    assert!(report.colima_missing && !report.docker_missing);
}

#[test]
fn first_run_docker_missing_fails_docker_and_compose_and_needs_setup() {
    let probe = Probe {
        docker: None,
        docker_version: None,
        compose_version: None,
        ..Probe::all_found()
    };
    let report = report(&probe, &[running_default()]);
    assert_eq!(report.row("docker").map(|r| r.status), Some(Status::Fail));
    assert_eq!(report.row("compose").map(|r| r.status), Some(Status::Fail));
    assert!(report.needs_setup());
    assert!(report.docker_missing && !report.colima_missing);
}

#[test]
fn first_run_a_missing_compose_plugin_fails_its_row_but_not_the_setup_screen() {
    let probe = Probe {
        compose_version: None,
        ..Probe::all_found()
    };
    let report = report(&probe, &[running_default()]);
    let compose = report.row("compose").expect("compose row");
    assert_eq!(compose.status, Status::Fail);
    assert!(compose.hint.is_some());
    // Only Colima or Docker missing opens the full-screen setup.
    assert!(!report.needs_setup());
}

#[test]
fn first_run_a_stopped_profile_warns_and_offers_to_start_it_as_it_is() {
    let stopped = engine("default", EngineState::Stopped, None);
    let report = report(&Probe::all_found(), std::slice::from_ref(&stopped));
    let colima = report.row("colima").expect("colima row");
    assert_eq!(colima.status, Status::Warn);
    assert_eq!(colima.value, "0.10.3 \u{b7} stopped");
    assert!(
        !report.needs_setup(),
        "a stopped profile is a card, not the setup screen"
    );
    let plan = report.start.expect("a start plan");
    assert_eq!(plan.engine, stopped);
    // The profile keeps its own sizes: a plain start, not Apply.
    assert_eq!(plan.op, berth_core::ColimaOp::Start);
    assert_eq!(plan.label, "Start default");
}

#[test]
fn first_run_no_profile_at_all_offers_to_create_default_with_two_cpus_and_eight_gb() {
    let report = report(&Probe::all_found(), &[]);
    let colima = report.row("colima").expect("colima row");
    assert_eq!(colima.status, Status::Warn);
    assert_eq!(colima.value, "0.10.3 \u{b7} no profile yet");
    let plan = report.start.expect("a start plan");
    assert_eq!(plan.engine.id.0, "colima:default");
    assert_eq!(plan.label, "Start default (2 CPU, 8 GB)");
    match plan.op {
        berth_core::ColimaOp::Apply(options) => {
            assert_eq!(options.cpus, Some(2));
            assert_eq!(options.memory_gib, Some(8));
            assert_eq!(options.disk_gib, None);
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn first_run_start_prefers_default_then_any_stopped_profile() {
    let work = engine("work", EngineState::Stopped, None);
    let default = engine("default", EngineState::Stopped, None);
    assert_eq!(
        start_plan(&[work.clone(), default.clone()]).engine.name,
        "default"
    );
    let plan = start_plan(&[work]);
    assert_eq!(plan.engine.name, "work");
    assert_eq!(plan.label, "Start work");
}

#[test]
fn first_run_a_running_profile_whose_socket_is_gone_fails_the_colima_row() {
    let report = evaluate(&Probe::all_found(), &[running_default()], &[], &|_| false);
    let colima = report.row("colima").expect("colima row");
    assert_eq!(colima.status, Status::Fail);
    assert!(colima.value.contains("no socket"), "{colima:?}");
    assert!(
        colima
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("docker.sock") && h.contains("colima restart default")),
        "{colima:?}"
    );
    // Socket and Colima are one row: nothing else reports it.
    assert!(report.row("socket").is_none());
}

#[test]
fn first_run_another_running_profile_satisfies_the_colima_row() {
    let work = engine(
        "work",
        EngineState::Running,
        Some("unix:///Users/me/.colima/work/docker.sock"),
    );
    let stopped_default = engine("default", EngineState::Stopped, None);
    let report = report(&Probe::all_found(), &[stopped_default, work]);
    let colima = report.row("colima").expect("colima row");
    assert_eq!(colima.status, Status::Pass);
    assert_eq!(colima.value, "0.10.3 \u{b7} work running");
    assert!(report.start.is_none());
}

// --- Finding the tools through a fake SearchPath ---

#[cfg(unix)]
fn script(dir: &Path, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

#[cfg(unix)]
fn fake_colima(dir: &Path) {
    script(dir, "colima", "echo 'colima version 0.10.3'");
}

#[cfg(unix)]
fn fake_docker(dir: &Path) {
    script(
        dir,
        "docker",
        "if [ \"$1\" = compose ]; then echo 'Docker Compose version v5.5.1'; else echo 'Docker version 29.8.0, build abc123'; fi",
    );
}

#[cfg(unix)]
#[tokio::test]
async fn first_run_probe_finds_both_tools_and_reads_their_versions() {
    let dir = tempfile::tempdir().expect("tempdir");
    fake_colima(dir.path());
    fake_docker(dir.path());
    let found = probe(&berth_sys::SearchPath::from_dirs(vec![dir
        .path()
        .to_path_buf()]))
    .await;
    assert_eq!(found.colima, Some(dir.path().join("colima")));
    assert_eq!(found.colima_version.as_deref(), Some("0.10.3"));
    assert_eq!(found.docker_version.as_deref(), Some("29.8.0"));
    assert_eq!(found.compose_version.as_deref(), Some("5.5.1"));
    let engines = [running_default()];
    assert!(!evaluate(&found, &engines, &[], &sockets_exist).needs_setup());
}

#[cfg(unix)]
#[tokio::test]
async fn first_run_probe_with_colima_missing_opens_the_setup() {
    let dir = tempfile::tempdir().expect("tempdir");
    fake_docker(dir.path());
    let found = probe(&berth_sys::SearchPath::from_dirs(vec![dir
        .path()
        .to_path_buf()]))
    .await;
    assert_eq!(found.colima, None);
    let report = evaluate(&found, &[], &[], &sockets_exist);
    assert!(report.colima_missing);
    assert!(report.needs_setup());
    assert_eq!(report.row("colima").map(|r| r.status), Some(Status::Fail));
    assert_eq!(report.row("docker").map(|r| r.status), Some(Status::Pass));
}

#[cfg(unix)]
#[tokio::test]
async fn first_run_probe_with_docker_missing_opens_the_setup() {
    let dir = tempfile::tempdir().expect("tempdir");
    fake_colima(dir.path());
    let found = probe(&berth_sys::SearchPath::from_dirs(vec![dir
        .path()
        .to_path_buf()]))
    .await;
    assert_eq!(found.docker, None);
    let report = evaluate(&found, &[running_default()], &[], &sockets_exist);
    assert!(report.docker_missing);
    assert!(report.needs_setup());
}

#[cfg(unix)]
#[tokio::test]
async fn first_run_berth_search_path_decides_what_is_found() {
    // `env -i PATH=...` still finds /opt/homebrew/bin; BERTH_SEARCH_PATH replaces the list (AD34).
    let with = tempfile::tempdir().expect("tempdir");
    fake_colima(with.path());
    fake_docker(with.path());
    let without = tempfile::tempdir().expect("tempdir");
    // nextest gives every test its own process, so setting the variable cannot race another test.
    std::env::set_var("BERTH_SEARCH_PATH", without.path());
    let empty = probe(&berth_sys::SearchPath::from_process()).await;
    std::env::set_var("BERTH_SEARCH_PATH", with.path());
    let full = probe(&berth_sys::SearchPath::from_process()).await;
    std::env::remove_var("BERTH_SEARCH_PATH");
    assert_eq!((empty.colima, empty.docker), (None, None));
    assert!(full.colima.is_some() && full.docker.is_some());
}

#[test]
fn first_run_version_lines_parse() {
    assert_eq!(
        parse_docker_version("Docker version 29.8.0, build 1234abc\n").as_deref(),
        Some("29.8.0")
    );
    assert_eq!(parse_docker_version("docker: not found"), None);
    assert_eq!(
        parse_compose_version("Docker Compose version v5.5.1\n").as_deref(),
        Some("5.5.1")
    );
    assert_eq!(
        parse_compose_version("Docker Compose version 2.39.1-desktop.1").as_deref(),
        Some("2.39.1-desktop.1")
    );
    assert_eq!(
        parse_compose_version("unknown command: docker compose"),
        None
    );
}

// --- Start default, through the actions service ---

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Host() -> Element {
    let state = crate::state::use_app_state();
    use_hook(move || SLOT.with(|slot| *slot.borrow_mut() = Some(state)));
    rsx! { div {} }
}

#[tokio::test]
async fn first_run_start_for_a_missing_profile_adds_it_to_the_engines_and_marks_it_starting() {
    let mut dom = VirtualDom::new(Host);
    let _ = dom.rebuild_to_vec();
    let mut state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("host captured the state");
    let (service, _rx) = crate::services::actions::ActionsService::new(
        crate::services::engines::client_map(),
        std::sync::Arc::new(berth_store::Store::open_in_memory().expect("store")),
        std::sync::Arc::new(berth_engine::MockCompose::bundled()),
        std::sync::Arc::new(berth_engine::MockColima::bundled()),
        tokio_util::sync::CancellationToken::new(),
    );
    let plan = start_plan(&[]);
    let result = dom.in_runtime(|| run_start(&mut state, &service, &plan));
    assert_eq!(result, Ok(()));
    let engines = dom.in_runtime(|| state.engines.peek().clone());
    assert_eq!(engines.len(), 1);
    assert_eq!(engines[0].id.0, "colima:default");
    assert_eq!(engines[0].state, EngineState::Starting);
    // A second press while it starts is refused, not queued.
    assert!(dom
        .in_runtime(|| run_start(&mut state, &service, &plan))
        .is_err());
}
