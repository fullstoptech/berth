use super::*;
use berth_core::StatusKind as S;

struct World {
    engines: Vec<berth_core::Engine>,
    containers: Vec<berth_core::Container>,
    projects: Vec<berth_core::Project>,
    pending: std::collections::BTreeMap<PendingKey, PendingState>,
}

fn world() -> World {
    let w = berth_engine::MockWorld::bundled();
    World {
        engines: w.engines.clone(),
        containers: w.containers.clone(),
        projects: w.projects.clone(),
        pending: std::collections::BTreeMap::new(),
    }
}

fn model_of(w: &World, name: &str, problems: &[berth_core::Problem]) -> ProjectRowModel {
    let project = w
        .projects
        .iter()
        .find(|p| p.name == name)
        .expect("project in the mock world");
    let view = StateView {
        engines: &w.engines,
        containers: &w.containers,
        projects: &w.projects,
        images: &[],
        volumes: &[],
        networks: &[],
        pending: &w.pending,
        backing: crate::services::backing::Backing::default(),
    };
    row_model(
        project,
        &view,
        problems,
        std::path::Path::new("/Users/berth"),
    )
}

fn error_on(container: &str, title: &str, w: &World) -> berth_core::Problem {
    let id = w
        .containers
        .iter()
        .find(|c| c.name == container)
        .expect("container")
        .id
        .clone();
    let kind = berth_core::ProblemKind::PortAllocated { port: 5432 };
    let subject = berth_core::Subject::Container(id);
    berth_core::Problem {
        id: berth_core::ProblemId::new(&kind, &subject),
        kind,
        severity: berth_core::Severity::Error,
        subject,
        title: title.to_string(),
        explanation: String::new(),
        evidence: Vec::new(),
        fixes: Vec::new(),
        first_seen: chrono::DateTime::default(),
    }
}

#[test]
fn projects_row_collapse_path_keeps_first_two_and_last() {
    assert_eq!(
        collapse_path("~/Dev/clients/acme/shop-api"),
        "~/Dev/…/shop-api"
    );
    assert_eq!(
        collapse_path("~/Dev/app-ideas/docker-manger"),
        "~/Dev/…/docker-manger"
    );
    assert_eq!(collapse_path("~/Dev/analytics"), "~/Dev/analytics");
    assert_eq!(collapse_path("/srv/landing"), "/srv/landing");
}

#[test]
fn projects_row_ports_are_running_services_sorted_once() {
    let w = world();
    let shop = w
        .projects
        .iter()
        .find(|p| p.name == "shop-api")
        .expect("shop");
    // db (5432) is exited, so it is not listed; the rest ascend.
    assert_eq!(running_ports(shop), vec![3000, 6379, 8025]);
    let analytics = w
        .projects
        .iter()
        .find(|p| p.name == "analytics")
        .expect("analytics");
    assert_eq!(running_ports(analytics), vec![8123, 9104]);
    let stopped = w
        .projects
        .iter()
        .find(|p| p.name == "ml-notebooks")
        .expect("ml");
    assert!(running_ports(stopped).is_empty());
}

#[test]
fn projects_row_service_note_names_the_worst_service() {
    let n = |items: &[(&str, S)]| {
        service_note(
            &items
                .iter()
                .map(|(name, s)| ((*name).to_string(), *s))
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        n(&[("a", S::Running), ("b", S::Healthy)]),
        Some(("all running".to_string(), NoteTone::Muted))
    );
    assert_eq!(
        n(&[("a", S::Running)]),
        Some(("running".to_string(), NoteTone::Muted))
    );
    assert_eq!(
        n(&[("a", S::Exited(0)), ("b", S::Exited(0))]),
        Some(("stopped".to_string(), NoteTone::Muted))
    );
    assert_eq!(
        n(&[
            ("api", S::Running),
            ("db", S::Exited(1)),
            ("w", S::Restarting)
        ]),
        Some(("db failing".to_string(), NoteTone::Danger))
    );
    assert_eq!(
        n(&[("ch", S::Running), ("grafana", S::Exited(0))]),
        Some(("grafana exited".to_string(), NoteTone::Warning))
    );
    // A service that is down explains the project better than one that is
    // merely unhealthy (frame: analytics reads "grafana exited").
    assert_eq!(
        n(&[
            ("ingest", S::Unhealthy),
            ("grafana", S::Exited(0)),
            ("ch", S::Running)
        ]),
        Some(("grafana exited".to_string(), NoteTone::Warning))
    );
    assert_eq!(
        n(&[("ingest", S::Unhealthy), ("ch", S::Running)]),
        Some(("ingest unhealthy".to_string(), NoteTone::Warning))
    );
    assert_eq!(n(&[]), None);
}

#[test]
fn projects_row_model_of_the_failing_project_matches_the_frame() {
    let w = world();
    let problems = vec![error_on("shop-api-db-1", "Port 5432 is already in use", &w)];
    let m = model_of(&w, "shop-api", &problems);
    assert_eq!(m.kind, RowKind::Normal);
    assert_eq!(m.path_short, "~/Dev/…/shop-api");
    assert_eq!(m.engine_name, "default");
    assert_eq!(m.branch.as_deref(), Some("feat/checkout-v2"));
    assert_eq!(m.services_label, "5 services");
    assert_eq!(m.note, Some(("db failing".to_string(), NoteTone::Danger)));
    assert_eq!(m.ports, vec![3000, 6379, 8025]);
    assert_eq!(m.cpu.as_deref(), Some("3.7%"));
    assert_eq!(m.problem_count, 1);
    assert_eq!(
        m.problem_summary.as_deref(),
        Some("1 problem · Port 5432 is already in use")
    );
    assert_eq!(m.primary, PrimaryOp::Stop);
    assert_eq!(m.restart_reason, None);
    assert_eq!(m.primary_reason, None);
}

#[test]
fn projects_row_only_error_problems_count_on_the_row() {
    let w = world();
    let mut warning = error_on("analytics-ingest-1", "ingest health check is failing", &w);
    warning.severity = berth_core::Severity::Warning;
    let m = model_of(&w, "analytics", &[warning]);
    assert_eq!(m.problem_count, 0);
    assert_eq!(m.problem_summary, None);
}

#[test]
fn projects_row_stopped_project_offers_start_and_disables_restart_with_a_reason() {
    let w = world();
    let m = model_of(&w, "ml-notebooks", &[]);
    assert_eq!(m.primary, PrimaryOp::Start);
    assert_eq!(m.restart_reason.as_deref(), Some("Nothing is running"));
    assert_eq!(m.primary_reason, None);
    assert_eq!(m.cpu, None);
    assert_eq!(m.mem, None);
    assert_eq!(m.note, Some(("stopped".to_string(), NoteTone::Muted)));
}

#[test]
fn projects_row_unreachable_engine_disables_everything_with_the_engine_reason() {
    let w = world();
    let m = model_of(&w, "landing", &[]);
    assert_eq!(m.kind, RowKind::Unreachable);
    assert_eq!(m.primary, PrimaryOp::Start);
    let reason = m.primary_reason.expect("start is disabled");
    assert!(reason.contains("unreachable"), "{reason}");
    assert!(m.restart_reason.is_some());
    assert!(m.dots.iter().all(|d| d.tone == berth_core::Tone::Danger));
    assert!(m.note.is_none());
}

#[test]
fn projects_row_pinned_folder_without_compose_is_not_set_up() {
    let w = world();
    let m = model_of(&w, "docker-manger", &[]);
    assert_eq!(m.kind, RowKind::NotSetUp);
    assert_eq!(m.services_label, "No compose file yet");
    assert_eq!(m.dots.len(), 1);
    assert_eq!(m.dots[0].shape, DotShape::Hollow);
}

#[test]
fn projects_row_pending_and_failed_actions_show_in_the_model() {
    let mut w = world();
    let id = w
        .projects
        .iter()
        .find(|p| p.name == "berth-site")
        .expect("p")
        .id
        .clone();
    w.pending.insert(
        PendingKey {
            subject: berth_core::Subject::Project(id.clone()),
            action: "project.stop",
        },
        PendingState::Running {
            label: "Stopping…".to_string(),
        },
    );
    let m = model_of(&w, "berth-site", &[]);
    assert_eq!(
        m.pending,
        Some(RowPending {
            action: "project.stop",
            label: "Stopping…".to_string()
        })
    );
    // Other buttons are refused while it runs (one pure rule, D34).
    assert_eq!(
        m.restart_reason.as_deref(),
        Some("Stopping… is still in progress")
    );

    w.pending.clear();
    w.pending.insert(
        PendingKey {
            subject: berth_core::Subject::Project(id.clone()),
            action: "project.stop",
        },
        PendingState::Failed {
            message: "Compose failed: no such service".to_string(),
        },
    );
    let m = model_of(&w, "berth-site", &[]);
    assert_eq!(m.pending, None);
    assert_eq!(
        m.failure,
        Some(RowFailure {
            subject: berth_core::Subject::Project(id.clone()),
            action: "project.stop",
            message: "Compose failed: no such service".to_string()
        })
    );
    // A failure does not block a retry.
    assert_eq!(m.restart_reason, None);
}

#[test]
fn projects_row_path_shows_home_as_tilde_for_any_home() {
    let home = std::path::Path::new("/Users/someone");
    let mock = crate::services::mock_world_in(chrono::Utc::now(), home);
    let project = mock
        .projects
        .iter()
        .find(|p| p.name == "shop-api")
        .expect("shop-api");
    let view = StateView {
        engines: &mock.engines,
        containers: &mock.containers,
        projects: &mock.projects,
        images: &[],
        volumes: &[],
        networks: &[],
        pending: &std::collections::BTreeMap::new(),
        backing: crate::services::backing::Backing::default(),
    };
    let m = row_model(project, &view, &[], home);
    assert_eq!(m.path_short, "~/Dev/…/shop-api");
    assert_eq!(m.path_full, "~/Dev/clients/acme/shop-api");
}

// --- The Unassigned row acts on its own containers (P2-T6) ---

fn default_engine() -> berth_core::EngineId {
    berth_core::EngineId("colima:default".to_string())
}

/// The mock world with the Unassigned project the live path groups.
fn world_with_unassigned() -> World {
    let mut w = world();
    let mock = berth_engine::MockWorld::bundled();
    w.projects.extend(crate::scenes::loose_projects(&mock));
    w
}

fn unassigned(w: &World) -> &berth_core::Project {
    w.projects
        .iter()
        .find(|p| p.source == berth_core::ProjectSource::Loose && p.engine == default_engine())
        .expect("the default engine has loose containers")
}

fn unassigned_model(w: &World) -> ProjectRowModel {
    let view = StateView {
        engines: &w.engines,
        containers: &w.containers,
        projects: &w.projects,
        images: &[],
        volumes: &[],
        networks: &[],
        pending: &w.pending,
        backing: crate::services::backing::Backing::default(),
    };
    row_model(
        unassigned(w),
        &view,
        &[],
        std::path::Path::new("/Users/berth"),
    )
}

fn member_ids(w: &World, state: berth_core::ContainerState) -> Vec<berth_core::ContainerId> {
    unassigned(w)
        .services
        .iter()
        .flat_map(|s| s.containers.iter())
        .filter(|id| {
            w.containers
                .iter()
                .any(|c| c.id == **id && c.state == state)
        })
        .cloned()
        .collect()
}

#[test]
fn projects_row_unassigned_has_no_folder_and_its_buttons_run_on_its_containers() {
    let w = world_with_unassigned();
    let m = unassigned_model(&w);
    assert_eq!(m.name, "Unassigned");
    assert_eq!(m.path_short, "—");
    // Six running, one paused, one exited: Stop all and Restart qualify.
    assert_eq!(m.primary, PrimaryOp::Stop);
    assert_eq!(m.primary_label, "Stop all");
    assert_eq!(m.primary_reason, None);
    assert_eq!(m.restart_reason, None);
    let ops = m.loose.expect("loose ops");
    assert_eq!(ops.ids.stop_ids.len(), 7, "{:?}", ops.ids);
    assert_eq!(ops.ids.start_ids.len(), 1, "{:?}", ops.ids);
    // Compose-only actions say why they are off.
    assert_eq!(m.rebuild_reason.as_deref(), Some("Not a Compose project"));
    assert_eq!(m.pull_reason.as_deref(), Some("Not a Compose project"));
}

#[test]
fn projects_row_unassigned_offers_start_all_when_nothing_runs() {
    let mut w = world_with_unassigned();
    let running: Vec<_> = member_ids(&w, berth_core::ContainerState::Running);
    for c in &mut w.containers {
        if running.contains(&c.id) || c.state == berth_core::ContainerState::Paused {
            c.state = berth_core::ContainerState::Exited;
        }
    }
    let m = unassigned_model(&w);
    assert_eq!(m.primary, PrimaryOp::Start);
    assert_eq!(m.primary_label, "Start all");
    assert_eq!(m.primary_reason, None);
    assert_eq!(m.restart_reason.as_deref(), Some("Nothing is running"));
}

#[test]
fn projects_row_unassigned_disables_a_button_with_a_reason_when_nothing_qualifies() {
    // Everything running: Start all has nobody to start.
    let mut w = world_with_unassigned();
    let members: Vec<_> = unassigned(&w)
        .services
        .iter()
        .flat_map(|s| s.containers.iter().cloned())
        .collect();
    for c in w.containers.iter_mut().filter(|c| members.contains(&c.id)) {
        c.state = berth_core::ContainerState::Running;
    }
    let m = unassigned_model(&w);
    let ops = m.loose.as_ref().expect("loose ops");
    assert_eq!(ops.start_reason.as_deref(), Some("Nothing is stopped"));
    assert_eq!(ops.stop_reason, None);
    let entries = menu_entries(&m);
    let start_all = entries.iter().find(|e| e.id == "start-all").expect("entry");
    assert_eq!(
        start_all.disabled_reason.as_deref(),
        Some("Nothing is stopped")
    );

    // The engine is stopped: every button gives the engine's reason.
    let mut w = world_with_unassigned();
    for e in &mut w.engines {
        if e.id == default_engine() {
            e.state = berth_core::EngineState::Stopped;
        }
    }
    let m = unassigned_model(&w);
    assert_eq!(
        m.primary_reason.as_deref(),
        Some("Engine default is stopped")
    );
    assert_eq!(
        m.restart_reason.as_deref(),
        Some("Engine default is stopped")
    );
}

#[test]
fn projects_row_unassigned_menu_has_the_same_actions_and_no_compose_ones() {
    let w = world_with_unassigned();
    let entries = menu_entries(&unassigned_model(&w));
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["open", "start-all", "stop-all", "restart"]);
    // A Compose project keeps Rebuild, Pull and Delete.
    let compose = menu_entries(&model_of(&w, "berth-site", &[]));
    let ids: Vec<&str> = compose.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["open", "rebuild", "pull", "terminals", "delete"]);
}

#[test]
fn projects_row_unassigned_pending_member_action_reads_as_the_rows_own_button() {
    let mut w = world_with_unassigned();
    let id = member_ids(&w, berth_core::ContainerState::Running)
        .into_iter()
        .next()
        .expect("a running member");
    w.pending.insert(
        PendingKey {
            subject: berth_core::Subject::Container(id.clone()),
            action: "container.restart",
        },
        PendingState::Running {
            label: "Restarting…".to_string(),
        },
    );
    let m = unassigned_model(&w);
    assert_eq!(
        m.pending,
        Some(RowPending {
            action: "project.restart",
            label: "Restarting…".to_string()
        })
    );
    // A container outside the project does not.
    w.pending.clear();
    let outsider = w
        .containers
        .iter()
        .find(|c| c.name == "shop-api-api-1")
        .expect("compose container")
        .id
        .clone();
    w.pending.insert(
        PendingKey {
            subject: berth_core::Subject::Container(outsider),
            action: "container.restart",
        },
        PendingState::Running {
            label: "Restarting…".to_string(),
        },
    );
    assert_eq!(unassigned_model(&w).pending, None);
}

#[test]
fn projects_row_compose_only_operations_have_no_container_plan() {
    let w = world_with_unassigned();
    let ops = unassigned_model(&w).loose.expect("loose ops");
    assert!(ops.plan(ProjectOp::Rebuild).is_none());
    assert!(ops.plan(ProjectOp::PullImages).is_none());
    let (ids, action) = ops.plan(ProjectOp::Stop).expect("stop plans");
    assert_eq!(ids, ops.ids.stop_ids.as_slice());
    assert_eq!(
        action,
        berth_core::ContainerAction::Stop { timeout_s: None }
    );
}

// --- Running them: one container action per qualifying member ---

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = crate::state::use_app_state();
    use_hook(move || SLOT.with(|slot| *slot.borrow_mut() = Some(state)));
    rsx! { div {} }
}

struct Rig {
    dom: VirtualDom,
    state: AppState,
    service: ActionsService,
    engine: std::sync::Arc<berth_engine::MockEngine>,
    finished: tokio::sync::mpsc::UnboundedReceiver<crate::services::actions::Finished>,
    row: ProjectRowModel,
}

fn rig() -> Rig {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    let w = world_with_unassigned();
    let clients = crate::services::engines::client_map();
    let engine = std::sync::Arc::new(berth_engine::MockEngine::bundled(&default_engine()));
    clients.lock().expect("clients").insert(
        default_engine(),
        std::sync::Arc::clone(&engine) as std::sync::Arc<dyn berth_engine::EngineClient>,
    );
    let (service, finished) = ActionsService::new(
        clients,
        std::sync::Arc::new(berth_store::Store::open_in_memory().expect("store")),
        std::sync::Arc::new(berth_engine::MockCompose::bundled()),
        std::sync::Arc::new(berth_engine::MockColima::bundled()),
        tokio_util::sync::CancellationToken::new(),
    );
    let row = unassigned_model(&w);
    dom.in_runtime(|| {
        let mut state = state;
        state.engines.set(w.engines.clone());
        state.containers.set(
            w.containers
                .iter()
                .filter(|c| c.engine == default_engine())
                .cloned()
                .collect(),
        );
        state.projects.set(w.projects.clone());
    });
    Rig {
        dom,
        state,
        service,
        engine,
        finished,
        row,
    }
}

impl Rig {
    /// Click a row button: `run_row_op` as the component calls it.
    fn press(&self, op: ProjectOp) {
        self.dom.in_runtime(|| {
            let mut state = self.state;
            run_row_op(
                &mut state,
                Some(&self.service),
                &self.row.id,
                self.row.loose.as_ref(),
                op,
            );
        });
    }

    fn pending_keys(&self) -> Vec<(berth_core::Subject, &'static str)> {
        self.dom.in_runtime(|| {
            self.state
                .pending
                .peek()
                .iter()
                .filter(|(_, v)| matches!(v, PendingState::Running { .. }))
                .map(|(k, _)| (k.subject.clone(), k.action))
                .collect()
        })
    }

    fn container_ids(&self, state: berth_core::ContainerState) -> Vec<berth_core::ContainerId> {
        self.dom.in_runtime(|| {
            self.state
                .containers
                .peek()
                .iter()
                .filter(|c| c.state == state)
                .map(|c| c.id.clone())
                .collect()
        })
    }
}

#[tokio::test]
async fn projects_row_unassigned_stop_all_stops_each_running_member_and_nothing_else() {
    use berth_engine::EngineClient as _;
    let mut rig = rig();
    let ops = rig.row.loose.clone().expect("loose ops");
    assert_eq!(ops.ids.stop_ids.len(), 7);
    let members: Vec<_> = rig
        .row
        .loose
        .iter()
        .flat_map(|o| o.ids.start_ids.iter().chain(o.ids.stop_ids.iter()))
        .cloned()
        .collect();
    let outside_running: Vec<_> = rig
        .container_ids(berth_core::ContainerState::Running)
        .into_iter()
        .filter(|id| !members.contains(id))
        .collect();
    assert!(!outside_running.is_empty(), "compose containers run too");

    rig.press(ProjectOp::Stop);

    // One container.stop key per active member: no project key, none for
    // the exited member, none for a Compose project's containers.
    let keys = rig.pending_keys();
    assert_eq!(keys.len(), 7, "{keys:?}");
    assert!(keys.iter().all(|(_, action)| *action == "container.stop"));
    for id in &ops.ids.stop_ids {
        assert!(
            keys.contains(&(berth_core::Subject::Container(id.clone()), "container.stop")),
            "{id:?} has its own pending key"
        );
    }
    // The button reads as running (Stop all, loading) while they run.
    let view_pending = rig.dom.in_runtime(|| rig.state.pending.peek().clone());
    let w = world_with_unassigned();
    let model = rig.dom.in_runtime(|| {
        let containers = rig.state.containers.peek();
        let engines = rig.state.engines.peek();
        let projects = rig.state.projects.peek();
        row_model(
            unassigned(&w),
            &StateView {
                engines: &engines,
                containers: &containers,
                projects: &projects,
                images: &[],
                volumes: &[],
                networks: &[],
                pending: &view_pending,
                backing: crate::services::backing::Backing::default(),
            },
            &[],
            std::path::Path::new("/Users/berth"),
        )
    });
    assert_eq!(
        model.pending.as_ref().map(|p| p.action),
        Some("project.stop")
    );

    for _ in 0..7 {
        let done = tokio::time::timeout(std::time::Duration::from_secs(5), rig.finished.recv())
            .await
            .expect("each stop finishes")
            .expect("channel open");
        assert_eq!(done.result, Ok(()));
        let mut state = rig.state;
        rig.dom
            .in_runtime(|| crate::services::actions::apply_finished(&mut state, done));
    }
    // The engine stopped those seven and left the others running.
    let now = rig.engine.containers().await.expect("containers");
    for id in &ops.ids.stop_ids {
        let c = now.iter().find(|c| c.id == *id).expect("member");
        assert_eq!(c.state, berth_core::ContainerState::Exited, "{}", c.name);
    }
    for id in &outside_running {
        let c = now.iter().find(|c| c.id == *id).expect("outsider");
        assert_eq!(c.state, berth_core::ContainerState::Running, "{}", c.name);
    }
}

#[tokio::test]
async fn projects_row_unassigned_restart_and_start_all_run_per_container_too() {
    let rig = rig();
    let ops = rig.row.loose.clone().expect("loose ops");
    rig.press(ProjectOp::Restart);
    let keys = rig.pending_keys();
    assert_eq!(keys.len(), ops.ids.stop_ids.len(), "{keys:?}");
    assert!(keys
        .iter()
        .all(|(_, action)| *action == "container.restart"));

    let rig = self::rig();
    rig.press(ProjectOp::Start);
    let keys = rig.pending_keys();
    assert_eq!(keys.len(), ops.ids.start_ids.len(), "{keys:?}");
    assert!(keys.iter().all(|(_, action)| *action == "container.start"));
}

#[tokio::test]
async fn projects_row_unassigned_button_with_nothing_to_act_on_starts_nothing() {
    let rig = rig();
    // Stop the world first: nothing runs, so Stop does nothing.
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        state.containers.write().iter_mut().for_each(|c| {
            c.state = berth_core::ContainerState::Exited;
        });
    });
    let w = world_with_unassigned();
    let containers = rig.dom.in_runtime(|| rig.state.containers.peek().clone());
    let row = row_model(
        unassigned(&w),
        &StateView {
            engines: &w.engines,
            containers: &containers,
            projects: &w.projects,
            images: &[],
            volumes: &[],
            networks: &[],
            pending: &std::collections::BTreeMap::new(),
            backing: crate::services::backing::Backing::default(),
        },
        &[],
        std::path::Path::new("/Users/berth"),
    );
    assert_eq!(row.restart_reason.as_deref(), Some("Nothing is running"));
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        run_row_op(
            &mut state,
            Some(&rig.service),
            &row.id,
            row.loose.as_ref(),
            ProjectOp::Stop,
        );
    });
    assert_eq!(rig.pending_keys(), vec![]);
}
