use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::bridge::{parse_event, Event};
use crate::services::backing::Backing;

const NOT_FOUND: &str = "OCI runtime exec failed: exec failed: unable to start container process: \
exec: \"bash\": executable file not found in $PATH: unknown";

fn tab_of(title: &str, exited: bool, unread: bool) -> TerminalTab {
    TerminalTab {
        id: Uuid::now_v7(),
        title: title.to_string(),
        session: None,
        exited,
        unread,
    }
}

fn exec_spec(shell: Option<&str>) -> ExecSpec {
    ExecSpec {
        container: berth_core::ContainerId("c1".to_string()),
        name: "shop-api-api-1".to_string(),
        label: "shop-api/api-1".to_string(),
        image: "shop-api-api:latest".to_string(),
        shell: shell.map(str::to_string),
        user: None,
        workdir: None,
        wait_running: false,
    }
}

fn spec(engine: &str) -> Spec {
    Spec {
        engine: berth_core::EngineId(engine.to_string()),
        project: None,
        kind: Kind::Exec(exec_spec(None)),
    }
}

// --- The probe ---

/// Run the probe over scripted outcomes and report the shells it tried.
async fn run_probe(
    requested: Option<&str>,
    outcomes: &[(&str, Result<u32, OpenError>)],
) -> (Result<(String, u32), ProbeError>, Vec<String>) {
    let tried = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&tried);
    let script: Vec<(String, Result<u32, OpenError>)> = outcomes
        .iter()
        .map(|(shell, outcome)| ((*shell).to_string(), outcome.clone()))
        .collect();
    let result = probe(requested, move |shell| {
        log.lock().expect("log").push(shell.clone());
        let outcome = script
            .iter()
            .find(|(name, _)| *name == shell)
            .map_or(Err(OpenError::NoShell), |(_, outcome)| outcome.clone());
        async move { outcome }
    })
    .await;
    let tried = tried.lock().expect("log").clone();
    (result, tried)
}

#[tokio::test]
async fn exec_probe_tries_bash_then_sh_and_stops_at_the_first_that_opens() {
    let (result, tried) =
        run_probe(None, &[("bash", Err(OpenError::NoShell)), ("sh", Ok(7))]).await;
    assert_eq!(result, Ok(("sh".to_string(), 7)));
    assert_eq!(tried, ["bash", "sh"]);

    let (result, tried) = run_probe(None, &[("bash", Ok(1)), ("sh", Ok(2))]).await;
    assert_eq!(result, Ok(("bash".to_string(), 1)));
    assert_eq!(tried, ["bash"], "sh is never tried when bash opens");
}

#[tokio::test]
async fn exec_probe_with_neither_shell_is_the_no_shell_state() {
    let (result, tried) = run_probe(None, &[]).await;
    assert_eq!(
        result,
        Err(ProbeError::NoShell {
            tried: vec!["bash".to_string(), "sh".to_string()]
        })
    );
    assert_eq!(tried, ["bash", "sh"]);
    assert_eq!(
        status_of(result.expect_err("no shell")),
        Status::NoShell {
            tried: vec!["bash".to_string(), "sh".to_string()]
        }
    );
}

#[tokio::test]
async fn exec_probe_tries_only_the_shell_the_user_picked() {
    let (result, tried) = run_probe(Some("zsh"), &[("zsh", Ok(3)), ("bash", Ok(1))]).await;
    assert_eq!(result, Ok(("zsh".to_string(), 3)));
    assert_eq!(tried, ["zsh"]);

    let (result, tried) = run_probe(Some("ash"), &[("bash", Ok(1))]).await;
    assert_eq!(
        result,
        Err(ProbeError::NoShell {
            tried: vec!["ash".to_string()]
        })
    );
    assert_eq!(tried, ["ash"], "a picked shell never falls back");
}

#[tokio::test]
async fn exec_probe_stops_at_an_error_that_is_not_a_missing_shell() {
    let (result, tried) = run_probe(
        None,
        &[
            (
                "bash",
                Err(OpenError::Other("container is not running".into())),
            ),
            ("sh", Ok(2)),
        ],
    )
    .await;
    assert_eq!(
        result,
        Err(ProbeError::Failed("container is not running".to_string()))
    );
    assert_eq!(tried, ["bash"]);
}

#[test]
fn exec_missing_shell_is_recognised_in_dockers_wording_only() {
    assert!(is_missing_executable(NOT_FOUND));
    assert!(is_missing_executable(
        "OCI runtime exec failed: stat /bin/bash: no such file or directory"
    ));
    assert!(!is_missing_executable("Container abc is not running"));
    assert!(!is_missing_executable("permission denied"));
    assert_eq!(
        classify(&berth_engine::EngineError::Docker(NOT_FOUND.to_string())),
        OpenError::NoShell
    );
    assert_eq!(
        classify(&berth_engine::EngineError::Docker("boom".to_string())),
        OpenError::Other("boom".to_string())
    );
    // Docker may answer with text on the stream instead of an error.
    assert!(chunk_is_exec_failure(NOT_FOUND.as_bytes()));
    assert!(!chunk_is_exec_failure(b"root@a3f9:/app# "));
    assert!(is_not_running("Container 1f2e is not running"));
    assert!(!is_not_running(NOT_FOUND));
}

// --- Tab state ---

#[test]
fn exec_tab_state_follows_the_session_to_its_exit() {
    let live = tab_of("shop-api/api-1 \u{b7} bash", false, false);
    let id = live.id;
    let mut tabs = vec![live];
    let mut view = PaneView {
        title: "shop-api/api-1 \u{b7} bash".to_string(),
        status: Status::Live,
        spec: spec("e"),
        session: None,
        shell_used: Some("bash".to_string()),
        generation: 1,
    };
    apply_ui(
        &mut tabs,
        Some(id),
        &UiMsg::Snapshot {
            tab: id,
            view: Box::new(view.clone()),
        },
    );
    assert!(!tabs[0].exited);
    assert_eq!(tab_label(&tabs[0]), "shop-api/api-1 \u{b7} bash");

    view.status = Status::Exited;
    apply_ui(
        &mut tabs,
        Some(id),
        &UiMsg::Snapshot {
            tab: id,
            view: Box::new(view.clone()),
        },
    );
    assert!(tabs[0].exited, "the session ended");
    assert_eq!(tab_label(&tabs[0]), "shop-api/api-1 \u{b7} Exited");

    // A failed open reads as ended too; a no-shell state does not.
    view.status = Status::Failed("boom".to_string());
    apply_ui(
        &mut tabs,
        Some(id),
        &UiMsg::Snapshot {
            tab: id,
            view: Box::new(view.clone()),
        },
    );
    assert!(tabs[0].exited);
    view.status = Status::NoShell {
        tried: vec!["bash".to_string(), "sh".to_string()],
    };
    view.title = view.spec.title(&view.status, None);
    apply_ui(
        &mut tabs,
        Some(id),
        &UiMsg::Snapshot {
            tab: id,
            view: Box::new(view),
        },
    );
    assert!(!tabs[0].exited);
    assert_eq!(tabs[0].title, "shop-api/api-1", "no shell: the name alone");
}

#[test]
fn exec_background_output_marks_a_tab_unread_only_when_it_is_not_shown() {
    let shown = tab_of("a \u{b7} bash", false, false);
    let hidden = tab_of("b \u{b7} sh", false, false);
    let (shown_id, hidden_id) = (shown.id, hidden.id);
    let mut tabs = vec![shown, hidden];
    apply_ui(&mut tabs, Some(shown_id), &UiMsg::Output { tab: shown_id });
    apply_ui(&mut tabs, Some(shown_id), &UiMsg::Output { tab: hidden_id });
    assert!(!tabs[0].unread, "the tab in view is read");
    assert!(tabs[1].unread, "background output leaves a dot");
    // The panel closed: nothing is in view.
    apply_ui(&mut tabs, None, &UiMsg::Output { tab: shown_id });
    assert!(tabs[0].unread);
}

#[test]
fn exec_tab_titles_name_the_shell_or_the_command() {
    let spec = spec("e");
    assert_eq!(
        spec.title(&Status::Connecting, None),
        "shop-api/api-1 \u{b7} bash"
    );
    assert_eq!(
        spec.title(&Status::Live, Some("sh")),
        "shop-api/api-1 \u{b7} sh"
    );
    let docker = Spec {
        kind: Kind::Docker {
            args: vec!["ps".to_string(), "-a".to_string()],
        },
        ..spec
    };
    assert_eq!(docker.title(&Status::Live, None), "docker ps -a");
}

// --- Pure helpers ---

#[test]
fn exec_service_label_drops_the_project_prefix() {
    assert_eq!(
        service_label(Some("shop-api"), "shop-api-api-1"),
        "shop-api/api-1"
    );
    assert_eq!(
        service_label(None, "metrics-exporter-1"),
        "metrics-exporter-1"
    );
    assert_eq!(service_label(Some("shop"), "shopper-1"), "shopper-1");
    assert_eq!(service_label(Some("shop-api"), "shop-api-"), "shop-api-");
}

#[test]
fn exec_terminal_is_offered_unless_the_container_cannot_take_an_exec() {
    use berth_core::ContainerState as S;
    for ok in [S::Running, S::Exited, S::Created, S::Restarting] {
        assert_eq!(terminal_disabled_reason(ok, Backing::Live), None, "{ok:?}");
    }
    for refused in [S::Paused, S::Removing, S::Dead] {
        assert!(
            terminal_disabled_reason(refused, Backing::Live).is_some(),
            "{refused:?}"
        );
    }
}

#[test]
fn exec_terminal_under_mock_data_is_refused_for_every_container_state_with_the_reason() {
    use berth_core::ContainerState as S;
    for state in [S::Running, S::Exited, S::Created, S::Restarting, S::Paused] {
        assert_eq!(
            terminal_disabled_reason(state, Backing::Mock).as_deref(),
            Some("Terminals need a real Docker engine. You're viewing mock data."),
            "{state:?}"
        );
    }
}

#[test]
fn exec_docker_command_text_splits_like_a_shell_would() {
    assert_eq!(split_args("ps -a"), ["ps", "-a"]);
    assert_eq!(
        split_args("run --name 'my box' \"a b\" c\\ d"),
        ["run", "--name", "my box", "a b", "c d"]
    );
    assert_eq!(split_args("  "), Vec::<String>::new());
    assert_eq!(split_args("exec x ''"), ["exec", "x", ""]);
}

#[test]
fn exec_view_ids_round_trip_and_reject_foreign_ids() {
    let tab = Uuid::now_v7();
    assert_eq!(parse_view_id(&view_id(tab, 3)), Some((tab, 3)));
    assert_eq!(parse_view_id("echo"), None);
    assert_eq!(parse_view_id(&format!("{tab}.x")), None);
}

#[test]
fn exec_scrollback_stays_bounded_and_starts_on_a_line() {
    let mut back = Scrollback::default();
    let line = b"0123456789abcdef\r\n";
    for _ in 0..(SCROLLBACK_BYTES / line.len() + 100) {
        back.push(line);
    }
    assert!(back.bytes().len() <= SCROLLBACK_BYTES);
    assert!(
        back.bytes().starts_with(b"0123456789abcdef"),
        "cut at a line start"
    );
    back.clear();
    assert!(back.bytes().is_empty());
}

// --- The core ---

struct Rig {
    core: ExecCore,
    ui: UnboundedReceiver<UiMsg>,
    store: Arc<berth_store::Store>,
}

fn rig() -> Rig {
    let clients = super::super::engines::client_map();
    let default = berth_core::EngineId("colima:default".to_string());
    clients.lock().expect("clients").insert(
        default.clone(),
        Arc::new(berth_engine::MockEngine::bundled(&default))
            as Arc<dyn berth_engine::EngineClient>,
    );
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let (core, ui) = ExecCore::new(clients, Arc::clone(&store), CancellationToken::new());
    Rig { core, ui, store }
}

async fn next_view(rx: &mut UnboundedReceiver<ViewMsg>) -> ViewMsg {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("a message within 5 s")
        .expect("channel open")
}

#[test]
fn exec_resize_from_the_page_reaches_the_slot_and_ignores_empty_sizes() {
    let rig = rig();
    let tab = Uuid::now_v7();
    rig.core.register(tab, spec("colima:default"));
    assert_eq!(rig.core.size(tab), Some(DEFAULT_SIZE));
    let raw = format!(
        r#"{{"type":"term.resize","id":"{}","cols":120,"rows":40}}"#,
        view_id(tab, 0)
    );
    let Some(Event::Term(event)) = parse_event(&raw) else {
        panic!("a term.resize message parses");
    };
    rig.core.term_event(event);
    assert_eq!(rig.core.size(tab), Some((120, 40)));
    let raw = format!(
        r#"{{"type":"term.resize","id":"{}","cols":0,"rows":0}}"#,
        view_id(tab, 0)
    );
    let Some(Event::Term(event)) = parse_event(&raw) else {
        panic!("parses");
    };
    rig.core.term_event(event);
    assert_eq!(
        rig.core.size(tab),
        Some((120, 40)),
        "a hidden host's 0x0 is ignored"
    );
}

#[test]
fn exec_replay_goes_to_a_viewer_once_it_is_ready_and_never_twice() {
    let mut rig = rig();
    let tab = Uuid::now_v7();
    rig.core.seed(
        tab,
        spec("colima:default"),
        Status::Live,
        Some("bash"),
        b"hello\r\n",
    );
    let mut viewer = rig.core.attach(tab).expect("attached");
    // Output before the host is ready is held back: the replay covers it.
    rig.core.inner.output(tab, 1, b"early\r\n");
    assert!(viewer.rx.try_recv().is_err(), "nothing before ready");
    rig.core.term_event(TermEvent::Ready {
        id: view_id(tab, viewer.viewer),
    });
    assert_eq!(
        viewer.rx.try_recv().ok(),
        Some(ViewMsg::Replay(b"hello\r\nearly\r\n".to_vec()))
    );
    rig.core.inner.output(tab, 1, b"late\r\n");
    assert_eq!(
        viewer.rx.try_recv().ok(),
        Some(ViewMsg::Output(b"late\r\n".to_vec()))
    );
    // A stale generation's output is dropped.
    rig.core.inner.output(tab, 0, b"stale");
    assert!(viewer.rx.try_recv().is_err());
    // Clear wipes the history too.
    rig.core.clear(tab);
    assert_eq!(viewer.rx.try_recv().ok(), Some(ViewMsg::Clear));
    let second = rig.core.attach(tab).expect("attached");
    assert_eq!(second.view.status, Status::Live);
    rig.core.term_event(TermEvent::Ready {
        id: view_id(tab, second.viewer),
    });
    let mut second = second;
    assert!(
        second.rx.try_recv().is_err(),
        "an empty history has no replay"
    );
    // The unread notice fires once until the tab is seen.
    assert!(matches!(rig.ui.try_recv(), Ok(UiMsg::Output { .. })));
    assert!(rig.ui.try_recv().is_err());
    rig.core.seen(tab);
    rig.core.inner.output(tab, 1, b"again");
    assert!(matches!(rig.ui.try_recv(), Ok(UiMsg::Output { .. })));
}

#[tokio::test]
async fn exec_first_ready_opens_the_session_and_a_refusal_ends_as_a_failed_tab() {
    let mut rig = rig();
    let world = berth_engine::MockWorld::bundled();
    let container = world
        .containers
        .iter()
        .find(|c| c.name == "shop-api-api-1")
        .expect("mock container");
    let tab = Uuid::now_v7();
    let mut spec = container_spec(container, Some("shop-api"), None);
    if let Kind::Exec(exec) = &mut spec.kind {
        exec.wait_running = false;
    }
    rig.core.register(tab, spec);
    let mut viewer = rig.core.attach(tab).expect("attached");
    assert_eq!(viewer.view.status, Status::Connecting);
    rig.core.term_event(TermEvent::Ready {
        id: view_id(tab, viewer.viewer),
    });
    // The mock engine cannot exec: the tab says so instead of hanging.
    let view = loop {
        if let ViewMsg::Snapshot(view) = next_view(&mut viewer.rx).await {
            if matches!(view.status, Status::Failed(_)) {
                break view;
            }
        }
    };
    assert_eq!(
        view.status,
        Status::Failed("exec is not supported by this engine".to_string())
    );
    // The tab strip hears of it, and the failure counts as ended.
    let mut tabs = vec![tab_of("x", false, false)];
    tabs[0].id = tab;
    while let Ok(msg) = rig.ui.try_recv() {
        apply_ui(&mut tabs, None, &msg);
    }
    assert!(tabs[0].exited);

    // One Berth entry, naming the container and the outcome, no command.
    let entries = rig
        .store
        .activity_query(&berth_store::ActivityFilter::default(), None, 10)
        .expect("activity");
    let entry = entries
        .iter()
        .find(|e| e.action == "terminal.open")
        .expect("terminal.open entry");
    assert_eq!(entry.subject_name, "shop-api-api-1");
    assert_eq!(entry.origin, berth_core::ActivityOrigin::Berth);
    assert!(matches!(entry.outcome, berth_core::Outcome::Failed { .. }));
    assert!(
        !format!("{entry:?}").contains("bash"),
        "no shell or command is stored"
    );
}

#[tokio::test]
async fn exec_closing_a_tab_forgets_it_and_cancels_its_session() {
    let rig = rig();
    let tab = Uuid::now_v7();
    rig.core.register(tab, spec("colima:default"));
    assert!(rig.core.view(tab).is_some());
    rig.core.close(tab);
    assert!(rig.core.view(tab).is_none());
    assert!(rig.core.attach(tab).is_none());
    // Input for a closed tab goes nowhere and does not panic.
    rig.core.term_event(TermEvent::Input {
        id: view_id(tab, 0),
        data: b"ls\n".to_vec(),
    });
}
