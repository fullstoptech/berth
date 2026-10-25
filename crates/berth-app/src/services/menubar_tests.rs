use super::*;
use std::cell::RefCell;

use berth_core::{ColimaOp, EngineAction};

use crate::state::Overlay;

// --- Builders: small worlds of exactly the shape under test. ---

fn engine(name: &str, kind: EngineKind, state: EngineState) -> Engine {
    let mut e = berth_engine::MockWorld::bundled()
        .engines
        .into_iter()
        .next()
        .expect("a mock engine");
    e.id = EngineId(format!("test:{name}"));
    e.name = name.to_string();
    e.kind = kind;
    e.state = state;
    e
}

fn colima(name: &str, state: EngineState) -> Engine {
    engine(
        name,
        EngineKind::Colima {
            profile: name.to_string(),
        },
        state,
    )
}

fn project(name: &str, status: ProjectStatus, pinned: bool) -> Project {
    let mut p = berth_engine::MockWorld::bundled()
        .projects
        .into_iter()
        .next()
        .expect("a mock project");
    p.id = ProjectId(format!("p:{name}"));
    p.name = name.to_string();
    p.status = status;
    p.pinned = pinned;
    p.services = vec![];
    p
}

fn container(
    name: &str,
    project: Option<&str>,
    state: ContainerState,
    health: Option<Health>,
    exit: Option<i32>,
) -> Container {
    let mut c = berth_engine::MockWorld::bundled()
        .containers
        .into_iter()
        .next()
        .expect("a mock container");
    c.id = ContainerId(format!("c:{name}"));
    c.name = name.to_string();
    c.project = project.map(|p| ProjectId(format!("p:{p}")));
    c.state = state;
    c.health = health;
    c.exit_code = exit;
    c
}

fn running(name: &str, project: &str) -> Container {
    container(name, Some(project), ContainerState::Running, None, None)
}

fn snapshot(
    engines: Vec<Engine>,
    projects: Vec<Project>,
    containers: Vec<Container>,
) -> AppSnapshot {
    AppSnapshot {
        engines,
        projects,
        containers,
    }
}

fn summary_of(model: &MenuModel) -> (&str, Tone) {
    match model.items.first() {
        Some(MenuItem::Summary { text, tone }) => (text, *tone),
        other => panic!("the first item is the summary, got {other:?}"),
    }
}

/// (label, detail, nested, command names) of every Row, in order.
fn rows(model: &MenuModel) -> Vec<(&str, &str, bool, Vec<&str>)> {
    model
        .items
        .iter()
        .filter_map(|item| match item {
            MenuItem::Row {
                label,
                detail,
                nested,
                commands,
                ..
            } => Some((
                label.as_str(),
                detail.as_str(),
                *nested,
                commands.iter().map(|(n, _)| n.as_str()).collect(),
            )),
            _ => None,
        })
        .collect()
}

fn row_command(model: &MenuModel, label: &str, command: &str) -> MenuCommand {
    model
        .items
        .iter()
        .find_map(|item| match item {
            MenuItem::Row {
                label: l, commands, ..
            } if l == label => commands
                .iter()
                .find(|(name, _)| name == command)
                .map(|(_, c)| c.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {command} on {label}"))
}

fn headers(model: &MenuModel) -> Vec<&'static str> {
    model
        .items
        .iter()
        .filter_map(|i| match i {
            MenuItem::Header(h) => Some(*h),
            _ => None,
        })
        .collect()
}

/// The frame "Menu bar / failing / 320": a failing project with two unwell containers, a
/// partial one, a healthy one, a running engine, a stopped one and an unreachable remote.
fn frame_world() -> AppSnapshot {
    snapshot(
        vec![
            colima("default", EngineState::Running),
            colima("work", EngineState::Stopped),
            engine(
                "hetzner-1",
                EngineKind::Remote {
                    transport: RemoteTransport::Ssh {
                        host: "h".into(),
                        user: None,
                        port: None,
                    },
                },
                EngineState::Unreachable {
                    reason: "timeout".into(),
                    last_seen: None,
                },
            ),
        ],
        vec![
            project("berth-site", ProjectStatus::AllRunning, false),
            project(
                "analytics",
                ProjectStatus::Partial {
                    running: 3,
                    total: 4,
                },
                false,
            ),
            project("shop-api", ProjectStatus::Failing, false),
        ],
        vec![
            running("shop-api-api-1", "shop-api"),
            running("shop-api-cache-1", "shop-api"),
            running("shop-api-queue-1", "shop-api"),
            container(
                "shop-api-db-1",
                Some("shop-api"),
                ContainerState::Exited,
                None,
                Some(1),
            ),
            container(
                "shop-api-worker-1",
                Some("shop-api"),
                ContainerState::Restarting,
                None,
                None,
            ),
            running("analytics-a-1", "analytics"),
            running("analytics-b-1", "analytics"),
            running("analytics-c-1", "analytics"),
            container(
                "analytics-d-1",
                Some("analytics"),
                ContainerState::Exited,
                None,
                Some(0),
            ),
            running("berth-site-web-1", "berth-site"),
            running("berth-site-db-1", "berth-site"),
        ],
    )
}

// --- menu_model ---

#[test]
fn menubar_failing_frame_has_the_frames_order_and_text() {
    let model = menu_model(&frame_world());
    assert_eq!(model.icon, MenuIcon::Failing);
    assert_eq!(
        summary_of(&model),
        (
            "1 failing \u{b7} 1 unhealthy \u{b7} 1 engine unreachable",
            Tone::Danger
        )
    );
    assert_eq!(headers(&model), ["Engines", "Projects"]);
    assert_eq!(
        rows(&model),
        vec![
            (
                "default",
                "Colima \u{b7} Running",
                false,
                vec!["Stop\u{2026}"]
            ),
            ("work", "Colima \u{b7} Stopped", false, vec!["Start"]),
            ("hetzner-1", "SSH \u{b7} Unreachable", false, vec!["Retry"]),
            (
                "shop-api",
                "3 of 5 running",
                false,
                vec!["Stop project", "Open in Berth"]
            ),
            (
                "shop-api-db-1",
                "Exited (1)",
                true,
                vec!["Show logs", "Restart"]
            ),
            (
                "shop-api-worker-1",
                "Restarting",
                true,
                vec!["Show logs", "Restart"]
            ),
            (
                "analytics",
                "3 of 4 running",
                false,
                vec!["Stop project", "Open in Berth"]
            ),
            (
                "berth-site",
                "2 of 2 running",
                false,
                vec!["Stop project", "Open in Berth"]
            ),
        ]
    );
    // The summary leads, Open Berth and Quit close it.
    assert!(matches!(model.items[0], MenuItem::Summary { .. }));
    let tail: Vec<_> = model.items.iter().rev().take(3).collect();
    assert_eq!(
        tail,
        vec![
            &MenuItem::Command {
                label: "Quit Berth",
                command: MenuCommand::Quit
            },
            &MenuItem::Command {
                label: "Open Berth",
                command: MenuCommand::OpenBerth
            },
            &MenuItem::Separator,
        ]
    );
}

#[test]
fn menubar_all_good_shows_a_calm_summary_and_the_success_icon() {
    let model = menu_model(&snapshot(
        vec![colima("default", EngineState::Running)],
        vec![project("site", ProjectStatus::AllRunning, false)],
        vec![running("site-web-1", "site"), running("site-db-1", "site")],
    ));
    assert_eq!(model.icon, MenuIcon::AllGood);
    assert_eq!(summary_of(&model), ("All good", Tone::Success));
    assert_eq!(
        rows(&model)
            .into_iter()
            .map(|r| (r.0, r.1))
            .collect::<Vec<_>>(),
        vec![
            ("default", "Colima \u{b7} Running"),
            ("site", "2 of 2 running")
        ]
    );
}

#[test]
fn menubar_engine_stopped_swaps_the_icon_and_offers_start() {
    let model = menu_model(&snapshot(
        vec![colima("default", EngineState::Stopped)],
        vec![project("site", ProjectStatus::Stopped, true)],
        vec![],
    ));
    assert_eq!(model.icon, MenuIcon::EngineStopped);
    assert_eq!(summary_of(&model), ("1 engine stopped", Tone::Stopped));
    assert_eq!(
        row_command(&model, "default", "Start"),
        MenuCommand::EngineStart(EngineId("test:default".into()))
    );
    // A pinned project stays listed while stopped, with Start rather than Stop.
    assert_eq!(
        row_command(&model, "site", "Start project"),
        MenuCommand::ProjectStart(ProjectId("p:site".into()))
    );
}

#[test]
fn menubar_unreachable_engine_is_a_failure_with_retry() {
    let model = menu_model(&snapshot(
        vec![
            colima("default", EngineState::Running),
            engine(
                "box",
                EngineKind::Remote {
                    transport: RemoteTransport::Ssh {
                        host: "h".into(),
                        user: None,
                        port: None,
                    },
                },
                EngineState::Unreachable {
                    reason: "no route".into(),
                    last_seen: None,
                },
            ),
        ],
        vec![],
        vec![],
    ));
    assert_eq!(model.icon, MenuIcon::Failing);
    assert_eq!(summary_of(&model), ("1 engine unreachable", Tone::Danger));
    assert_eq!(
        row_command(&model, "box", "Retry"),
        MenuCommand::EngineRetry(EngineId("test:box".into()))
    );
    // No Start or Stop for a remote engine Berth does not control.
    let running_row = rows(&model).into_iter().find(|r| r.0 == "default");
    assert_eq!(running_row.map(|r| r.3), Some(vec!["Stop\u{2026}"]));
}

#[test]
fn menubar_failing_containers_nest_under_their_project_failed_before_unwell() {
    let model = menu_model(&snapshot(
        vec![colima("default", EngineState::Running)],
        vec![project("shop", ProjectStatus::Failing, false)],
        vec![
            container(
                "shop-aaa-unhealthy",
                Some("shop"),
                ContainerState::Running,
                Some(Health::Unhealthy),
                None,
            ),
            container(
                "shop-zzz-dead",
                Some("shop"),
                ContainerState::Exited,
                None,
                Some(137),
            ),
            running("shop-ok", "shop"),
        ],
    ));
    let got: Vec<_> = rows(&model)
        .into_iter()
        .map(|(label, detail, nested, _)| (label, detail, nested))
        .collect();
    assert_eq!(
        got,
        vec![
            ("default", "Colima \u{b7} Running", false),
            ("shop", "2 of 3 running", false),
            ("shop-zzz-dead", "Exited (137)", true),
            ("shop-aaa-unhealthy", "Unhealthy", true),
        ]
    );
    assert_eq!(
        row_command(&model, "shop-zzz-dead", "Show logs"),
        MenuCommand::ShowLogs(ContainerId("c:shop-zzz-dead".into()))
    );
    assert_eq!(
        row_command(&model, "shop-zzz-dead", "Restart"),
        MenuCommand::Restart(ContainerId("c:shop-zzz-dead".into()))
    );
    assert_eq!(summary_of(&model).0, "1 failing \u{b7} 1 unhealthy");
}

#[test]
fn menubar_containers_without_a_project_id_nest_by_compose_project_name() {
    // The engines service does not always stamp a ProjectId; Compose's label still places it.
    let p = project("shop", ProjectStatus::Failing, false);
    let mut crash = container("shop-db-1", None, ContainerState::Exited, None, Some(1));
    crash.compose_project = Some("shop".to_string());
    crash.engine = p.engine.clone();
    let mut web = container("shop-web-1", None, ContainerState::Running, None, None);
    web.compose_project = Some("shop".to_string());
    web.engine = p.engine.clone();
    let model = menu_model(&snapshot(
        vec![colima("default", EngineState::Running)],
        vec![p],
        vec![web, crash],
    ));
    let got: Vec<_> = rows(&model)
        .into_iter()
        .map(|(label, detail, nested, _)| (label, detail, nested))
        .collect();
    assert_eq!(
        got,
        vec![
            ("default", "Colima \u{b7} Running", false),
            ("shop", "1 of 2 running", false),
            ("shop-db-1", "Exited (1)", true),
        ]
    );
}

#[test]
fn menubar_two_projects_with_one_name_do_not_both_claim_an_unlinked_container() {
    let mut a = project("same", ProjectStatus::AllRunning, false);
    let mut b = project("same", ProjectStatus::AllRunning, false);
    a.id = ProjectId("p:a".into());
    b.id = ProjectId("p:b".into());
    b.engine = a.engine.clone();
    let mut web = container("same-web-1", None, ContainerState::Running, None, None);
    web.compose_project = Some("same".to_string());
    web.engine = a.engine.clone();
    let model = menu_model(&snapshot(
        vec![colima("default", EngineState::Running)],
        vec![a, b],
        vec![web],
    ));
    let counts: Vec<_> = rows(&model)
        .into_iter()
        .filter(|r| r.0 == "same")
        .map(|r| r.1)
        .collect();
    // Neither counts the container twice: the services' own numbers stand.
    assert_eq!(counts, vec!["0 of 0 running", "0 of 0 running"]);
}

#[test]
fn menubar_the_unassigned_bucket_is_not_a_project_and_its_failures_stay_visible() {
    let mut bucket = project(
        "Unassigned",
        ProjectStatus::Partial {
            running: 1,
            total: 2,
        },
        false,
    );
    bucket.source = ProjectSource::Loose;
    let mut lone = container("lone", None, ContainerState::Exited, None, Some(1));
    lone.project = Some(bucket.id.clone());
    let model = menu_model(&snapshot(
        vec![colima("default", EngineState::Running)],
        vec![bucket],
        vec![lone],
    ));
    let got: Vec<_> = rows(&model).into_iter().map(|r| r.0).collect();
    assert_eq!(got, vec!["default", "lone"]);
}

#[test]
fn menubar_a_failed_container_outside_every_project_is_listed_on_its_own() {
    let model = menu_model(&snapshot(
        vec![colima("default", EngineState::Running)],
        vec![],
        vec![container(
            "lone",
            None,
            ContainerState::Exited,
            None,
            Some(1),
        )],
    ));
    assert_eq!(model.icon, MenuIcon::Failing);
    let got: Vec<_> = rows(&model)
        .into_iter()
        .map(|(label, _, nested, _)| (label, nested))
        .collect();
    assert_eq!(got, vec![("default", false), ("lone", false)]);
}

#[test]
fn menubar_projects_list_failing_first_then_unwell_then_healthy() {
    let model = menu_model(&snapshot(
        vec![colima("default", EngineState::Running)],
        vec![
            project("a-healthy", ProjectStatus::AllRunning, false),
            project(
                "b-partial",
                ProjectStatus::Partial {
                    running: 1,
                    total: 2,
                },
                false,
            ),
            project("c-failing", ProjectStatus::Failing, false),
            project("d-stopped", ProjectStatus::Stopped, true),
            project("e-hidden", ProjectStatus::Stopped, false),
        ],
        vec![],
    ));
    let order: Vec<_> = rows(&model).into_iter().map(|r| r.0).collect();
    assert_eq!(
        order,
        [
            "default",
            "c-failing",
            "b-partial",
            "a-healthy",
            "d-stopped"
        ]
    );
}

#[test]
fn menubar_n_of_m_running_counts_the_projects_containers() {
    let model = menu_model(&snapshot(
        vec![colima("default", EngineState::Running)],
        vec![project(
            "db",
            ProjectStatus::Partial {
                running: 9,
                total: 9,
            },
            false,
        )],
        vec![
            running("db-1", "db"),
            container("db-2", Some("db"), ContainerState::Exited, None, Some(0)),
            container("db-3", Some("db"), ContainerState::Exited, None, Some(0)),
        ],
    ));
    // The project's own containers win over the status counts.
    assert!(
        rows(&model)
            .iter()
            .any(|r| r.0 == "db" && r.1 == "1 of 3 running"),
        "{:?}",
        rows(&model)
    );
}

#[test]
fn menubar_model_changes_only_when_the_menu_content_does() {
    let world = frame_world();
    assert_eq!(menu_model(&world), menu_model(&world.clone()));
    // A container's uptime or stats never reach the menu, so the tray is not rebuilt for them.
    let mut busy = world.clone();
    busy.containers[0].started = None;
    assert_eq!(menu_model(&busy), menu_model(&world));
    // A container going down does.
    let mut down = world.clone();
    down.containers[0].state = ContainerState::Exited;
    down.containers[0].exit_code = Some(2);
    assert_ne!(menu_model(&down), menu_model(&world));
}

// --- command ids ---

fn every_command() -> Vec<MenuCommand> {
    let engine = EngineId("colima:default".to_string());
    let project = ProjectId("proj:with:colons".to_string());
    let container = ContainerId("abc123".to_string());
    vec![
        MenuCommand::EngineStart(engine.clone()),
        MenuCommand::EngineAskStop(engine.clone()),
        MenuCommand::EngineRetry(engine),
        MenuCommand::ProjectStart(project.clone()),
        MenuCommand::ProjectStop(project.clone()),
        MenuCommand::ProjectOpen(project),
        MenuCommand::ShowLogs(container.clone()),
        MenuCommand::Restart(container),
        MenuCommand::OpenBerth,
        MenuCommand::Quit,
    ]
}

#[test]
fn menubar_command_ids_round_trip_and_stay_distinct() {
    let all = every_command();
    let ids: Vec<String> = all.iter().map(command_id).collect();
    for (command, id) in all.iter().zip(&ids) {
        assert!(id.starts_with("tray:"), "{id}");
        assert_eq!(parse_command_id(id).as_ref(), Some(command), "{id}");
    }
    let unique: std::collections::BTreeSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "{ids:?}");
    assert_eq!(
        command_id(&MenuCommand::EngineStart(EngineId("colima:default".into()))),
        "tray:engine.start:colima:default"
    );
}

#[test]
fn menubar_foreign_or_malformed_ids_do_not_parse() {
    for id in [
        "",
        "app.quit",
        "global.palette",
        "tray:",
        "tray:nonsense",
        "tray:engine.start",
        "tray:quit:extra",
        "tray:open:extra",
        "other:quit",
    ] {
        assert_eq!(parse_command_id(id), None, "{id}");
    }
}

// --- running commands ---

#[derive(Default)]
struct FakeHost {
    shown: RefCell<u32>,
    quit: RefCell<u32>,
}

impl MenuHost for FakeHost {
    fn show_window(&self) {
        *self.shown.borrow_mut() += 1;
    }
    fn quit(&self) {
        *self.quit.borrow_mut() += 1;
    }
}

thread_local! {
    static SLOT: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = crate::state::use_app_state();
    use_hook(move || SLOT.with(|slot| *slot.borrow_mut() = Some(state)));
    rsx! { div {} }
}

fn harness() -> (VirtualDom, AppState) {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    (dom, state)
}

fn run(dom: &VirtualDom, host: &FakeHost, state: &mut AppState, command: &MenuCommand) {
    dom.in_scope(ScopeId::ROOT, || {
        run_command(host, state, None, None, command);
    });
}

#[test]
fn menubar_quit_quits_through_the_host_and_never_shows_the_window() {
    let (dom, mut state) = harness();
    let host = FakeHost::default();
    run(&dom, &host, &mut state, &MenuCommand::Quit);
    assert_eq!((*host.quit.borrow(), *host.shown.borrow()), (1, 0));
}

#[test]
fn menubar_open_berth_shows_the_window_and_changes_nothing_else() {
    let (dom, mut state) = harness();
    let host = FakeHost::default();
    run(&dom, &host, &mut state, &MenuCommand::OpenBerth);
    assert_eq!((*host.quit.borrow(), *host.shown.borrow()), (0, 1));
    assert!(state.overlays.peek().is_empty());
}

#[test]
fn menubar_stopping_an_engine_only_asks_and_shows_the_window() {
    let (dom, mut state) = harness();
    let host = FakeHost::default();
    let default = colima("default", EngineState::Running);
    let id = default.id.clone();
    state.engines.set(vec![default]);
    run(
        &dom,
        &host,
        &mut state,
        &MenuCommand::EngineAskStop(id.clone()),
    );
    assert_eq!(*host.shown.borrow(), 1);
    let overlays = state.overlays.peek().clone();
    match overlays.as_slice() {
        [Overlay::Confirm(model)] => {
            assert!(
                matches!(
                    &model.action,
                    EngineAction::Colima { profile, op: ColimaOp::Stop } if profile == "default"
                ),
                "{:?}",
                model.action
            );
            assert_eq!(model.targets, vec![Subject::Engine(id.clone())]);
        }
        other => panic!("one confirm expected, got {other:?}"),
    }
    // Nothing started: the engine is still Running and no action is pending.
    assert_eq!(state.engines.peek()[0].state, EngineState::Running);
    assert!(state.pending.peek().is_empty());
}

#[test]
fn menubar_stopping_a_vanished_engine_says_so_and_opens_nothing() {
    let (dom, mut state) = harness();
    let host = FakeHost::default();
    run(
        &dom,
        &host,
        &mut state,
        &MenuCommand::EngineAskStop(EngineId("gone".into())),
    );
    assert!(state.overlays.peek().is_empty());
    assert_eq!(*host.shown.borrow(), 0);
    assert_eq!(state.toasts.peek().len(), 1);
}

#[test]
fn menubar_show_logs_selects_the_container_and_opens_the_logs_tab() {
    let (dom, mut state) = harness();
    let host = FakeHost::default();
    let id = ContainerId("c:x".into());
    run(&dom, &host, &mut state, &MenuCommand::ShowLogs(id.clone()));
    assert_eq!(*host.shown.borrow(), 1);
    assert_eq!(*state.section.peek(), Section::Containers);
    assert_eq!(*state.selection.peek(), Selection::Containers(vec![id]));
    let bottom = state.bottom.peek();
    assert!(bottom.open);
    assert_eq!(bottom.tab, crate::state::BottomTab::Logs);
}

#[test]
fn menubar_project_and_restart_commands_work_without_a_window_and_refuse_without_actions() {
    let (dom, mut state) = harness();
    let host = FakeHost::default();
    let p = project("site", ProjectStatus::AllRunning, false);
    let c = running("site-web-1", "site");
    state.projects.set(vec![p.clone()]);
    state.containers.set(vec![c.clone()]);
    run(
        &dom,
        &host,
        &mut state,
        &MenuCommand::ProjectStop(p.id.clone()),
    );
    run(&dom, &host, &mut state, &MenuCommand::Restart(c.id.clone()));
    // With no actions service the reason is told, not swallowed, and the window stays hidden.
    assert_eq!(state.toasts.peek().len(), 2);
    assert_eq!(*host.shown.borrow(), 0);
}
