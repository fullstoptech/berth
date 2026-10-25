//! P5-T8 tests for the problems service: the list keeps its order and `first_seen` and clears
//! what is resolved, the rules read what the engine says about a container, error lines from log
//! batches feed them, and the service updates `AppState::problems` as the engines change.

use berth_core::{ContainerState, ProblemKind, Severity};

use super::*;
use crate::state::use_app_state;

fn engine_id() -> EngineId {
    EngineId("colima:default".to_string())
}

fn other_engine() -> EngineId {
    EngineId("ssh:hetzner-1".to_string())
}

fn problem(name: &str, severity: Severity, at: i64) -> Problem {
    let kind = ProblemKind::Unhealthy;
    let subject = Subject::Container(ContainerId(name.to_string()));
    Problem {
        id: ProblemId::new(&kind, &subject),
        kind,
        severity,
        subject,
        title: name.to_string(),
        explanation: String::new(),
        evidence: Vec::new(),
        fixes: Vec::new(),
        first_seen: DateTime::from_timestamp(at, 0).unwrap_or_default(),
    }
}

fn titles(list: &[Problem]) -> Vec<&str> {
    list.iter().map(|p| p.title.as_str()).collect()
}

// ---------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------

#[test]
fn problems_tracker_orders_a_first_pass_by_severity_then_title() {
    let mut tracker = Tracker::default();
    tracker.update(
        &engine_id(),
        vec![
            problem("zeta", Severity::Warning, 10),
            problem("beta", Severity::Error, 10),
            problem("alpha", Severity::Error, 10),
        ],
    );
    assert_eq!(titles(&tracker.list()), ["alpha", "beta", "zeta"]);
}

#[test]
fn problems_tracker_keeps_first_seen_and_never_reorders_what_is_listed() {
    let mut tracker = Tracker::default();
    tracker.update(
        &engine_id(),
        vec![
            problem("beta", Severity::Warning, 100),
            problem("alpha", Severity::Warning, 100),
        ],
    );
    // A later pass: the same two (their rules stamped a newer time), and an Error that would sort
    // first by severity. It joins at the end; the first two keep their place and their time.
    tracker.update(
        &engine_id(),
        vec![
            problem("gamma", Severity::Error, 200),
            problem("beta", Severity::Warning, 200),
            problem("alpha", Severity::Warning, 200),
        ],
    );
    let list = tracker.list();
    assert_eq!(titles(&list), ["alpha", "beta", "gamma"]);
    let seen: Vec<i64> = list.iter().map(|p| p.first_seen.timestamp()).collect();
    assert_eq!(seen, [100, 100, 200]);
}

#[test]
fn problems_tracker_clears_a_resolved_problem_and_a_returning_one_is_new() {
    let mut tracker = Tracker::default();
    tracker.update(
        &engine_id(),
        vec![
            problem("alpha", Severity::Error, 100),
            problem("beta", Severity::Error, 100),
        ],
    );
    tracker.update(&engine_id(), vec![problem("beta", Severity::Error, 150)]);
    assert_eq!(titles(&tracker.list()), ["beta"]);
    // alpha comes back: a new problem, after beta, with the time it came back.
    tracker.update(
        &engine_id(),
        vec![
            problem("alpha", Severity::Error, 300),
            problem("beta", Severity::Error, 300),
        ],
    );
    let list = tracker.list();
    assert_eq!(titles(&list), ["beta", "alpha"]);
    assert_eq!(list[0].first_seen.timestamp(), 100);
    assert_eq!(list[1].first_seen.timestamp(), 300);
}

#[test]
fn problems_tracker_engines_are_independent_and_a_gone_engine_is_dropped() {
    let mut tracker = Tracker::default();
    tracker.update(&engine_id(), vec![problem("local", Severity::Error, 1)]);
    tracker.update(&other_engine(), vec![problem("remote", Severity::Error, 2)]);
    // The remote engine answering again with nothing does not touch the local one's problem.
    tracker.update(&other_engine(), Vec::new());
    assert_eq!(titles(&tracker.list()), ["local"]);
    tracker.update(&other_engine(), vec![problem("remote", Severity::Error, 3)]);
    assert_eq!(titles(&tracker.list()), ["local", "remote"]);
    tracker.retain_engines(&[other_engine()]);
    assert_eq!(titles(&tracker.list()), ["remote"]);
}

// ---------------------------------------------------------------------
// One engine's pass
// ---------------------------------------------------------------------

fn world() -> berth_engine::MockWorld {
    crate::services::mock_world_at(Utc::now())
}

fn snapshot(world: &berth_engine::MockWorld) -> EngineSnapshot {
    EngineSnapshot {
        engine: world
            .engines
            .iter()
            .find(|e| e.id == engine_id())
            .cloned()
            .expect("the default engine"),
        containers: world.containers_for(&engine_id()),
        projects: world
            .projects
            .iter()
            .filter(|p| p.engine == engine_id())
            .cloned()
            .collect(),
        failures: HashMap::new(),
        log_errors: HashMap::new(),
        disk: None,
    }
}

fn mock_client(world: &berth_engine::MockWorld) -> berth_engine::MockEngine {
    berth_engine::MockEngine::from_world(world.clone(), &engine_id()).expect("mock engine")
}

fn empty_cache() -> Cache {
    Mutex::new(HashMap::new())
}

#[tokio::test]
async fn problems_a_pass_over_the_mock_engine_finds_the_port_clash_and_the_unhealthy_container() {
    let world = world();
    let client = mock_client(&world);
    let found = check_engine(Some(&client), &empty_cache(), snapshot(&world), Utc::now()).await;
    let kinds: Vec<&ProblemKind> = found.iter().map(|p| &p.kind).collect();
    assert!(
        kinds.contains(&&ProblemKind::PortAllocated { port: 5432 }),
        "{:?}",
        titles(&found)
    );
    assert!(kinds.contains(&&ProblemKind::Unhealthy));
    // One engine's pass names only that engine's containers.
    for p in &found {
        let Subject::Container(id) = &p.subject else {
            continue;
        };
        assert!(
            world
                .containers_for(&engine_id())
                .iter()
                .any(|c| c.id == *id),
            "{}",
            p.title
        );
    }
}

#[test]
fn problems_a_container_gets_the_project_that_lists_it() {
    let world = world();
    let mut containers = world.containers_for(&engine_id());
    for c in &mut containers {
        c.project = None;
    }
    let projects: Vec<Project> = world
        .projects
        .iter()
        .filter(|p| p.engine == engine_id())
        .cloned()
        .collect();
    link_projects(&mut containers, &projects);
    let mut linked = 0;
    for project in &projects {
        for id in project.services.iter().flat_map(|s| s.containers.iter()) {
            let c = containers.iter().find(|c| c.id == *id);
            if let Some(c) = c {
                assert_eq!(c.project.as_ref(), Some(&project.id), "{}", c.name);
                linked += 1;
            }
        }
    }
    assert!(
        linked > 5,
        "the mock world's projects list their containers"
    );
    // A container no project lists stays unlinked, and a link that was there is kept.
    let loose = containers
        .iter()
        .find(|c| c.name == "pg-scratch")
        .expect("pg-scratch");
    assert_eq!(loose.project, None);
    let mut kept = containers[0].clone();
    kept.project = Some(ProjectId("elsewhere".to_string()));
    let mut one = [kept];
    link_projects(&mut one, &projects);
    assert_eq!(one[0].project, Some(ProjectId("elsewhere".to_string())));
}

#[tokio::test]
async fn problems_a_container_that_is_running_and_healthy_is_never_read() {
    let world = world();
    let client = mock_client(&world);
    let cache = empty_cache();
    let snap = snapshot(&world);
    let _ = check_engine(Some(&client), &cache, snap.clone(), Utc::now()).await;
    let read: HashSet<ContainerId> = lock(&cache).keys().cloned().collect();
    for c in &snap.containers {
        assert_eq!(read.contains(&c.id), worth_reading(c), "{}", c.name);
    }
    assert!(
        snap.containers.iter().any(|c| !worth_reading(c)),
        "the mock world has containers that are in trouble"
    );
    assert!(
        snap.containers.iter().any(worth_reading_not),
        "and containers that are fine"
    );
}

fn worth_reading_not(c: &Container) -> bool {
    !worth_reading(c)
}

#[tokio::test]
async fn problems_the_cache_forgets_containers_that_are_gone() {
    let world = world();
    let client = mock_client(&world);
    let cache = empty_cache();
    let mut snap = snapshot(&world);
    let _ = check_engine(Some(&client), &cache, snap.clone(), Utc::now()).await;
    let gone = snap
        .containers
        .iter()
        .find(|c| worth_reading(c))
        .map(|c| c.id.clone())
        .expect("a container that was read");
    assert!(lock(&cache).contains_key(&gone));
    snap.containers.retain(|c| c.id != gone);
    let _ = check_engine(Some(&client), &cache, snap, Utc::now()).await;
    assert!(!lock(&cache).contains_key(&gone));
}

#[tokio::test]
async fn problems_error_lines_from_a_log_batch_reach_the_rules() {
    let world = world();
    let client = mock_client(&world);
    let mut snap = snapshot(&world);
    // A running, healthy container: nothing is read for it, so only what a batch showed counts.
    let api = snap
        .containers
        .iter()
        .find(|c| c.name == "shop-api-api-1")
        .cloned()
        .expect("shop-api-api-1");
    assert!(!worth_reading(&api));
    let before = check_engine(Some(&client), &empty_cache(), snap.clone(), Utc::now()).await;
    assert!(!before
        .iter()
        .any(|p| p.subject == Subject::Container(api.id.clone())));
    snap.log_errors.insert(
        api.id.clone(),
        vec![
            "Error response from daemon: Bind for 0.0.0.0:3000 failed: port is already allocated"
                .to_string(),
        ],
    );
    let after = check_engine(Some(&client), &empty_cache(), snap, Utc::now()).await;
    let found = after
        .iter()
        .find(|p| p.subject == Subject::Container(api.id.clone()))
        .unwrap_or_else(|| panic!("no problem for the api: {:?}", titles(&after)));
    assert_eq!(found.kind, ProblemKind::PortAllocated { port: 3000 });
}

#[tokio::test]
async fn problems_a_compose_failure_becomes_a_project_problem() {
    let world = world();
    let client = mock_client(&world);
    let mut snap = snapshot(&world);
    let project = snap
        .projects
        .iter()
        .find(|p| p.name == "shop-api")
        .cloned()
        .expect("shop-api");
    snap.failures.insert(
        project.id.clone(),
        vec![
            "Error response from daemon: pull access denied for berthfx/does-not-exist, repository does not exist or may require 'docker login'"
                .to_string(),
        ],
    );
    let found = check_engine(Some(&client), &empty_cache(), snap, Utc::now()).await;
    assert!(
        found.iter().any(|p| matches!(
            &p.kind,
            ProblemKind::ImageNotFound { reference } if reference == "berthfx/does-not-exist"
        )),
        "{:?}",
        found
            .iter()
            .map(|p| (&p.kind, &p.title))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------
// Log batches
// ---------------------------------------------------------------------

fn log_line(container: &ContainerId, text: &str) -> LogLine {
    LogLine {
        container: container.clone(),
        ts: Utc::now(),
        stream: Stream::Stderr,
        text: text.to_string(),
    }
}

#[test]
fn problems_error_lines_are_kept_per_container_without_repeats_and_newest_last() {
    let service = ProblemsService::inert();
    let id = ContainerId("c1".to_string());
    let lines: Vec<LogLine> = (0..30)
        .map(|n| log_line(&id, &format!("Error: attempt {n}")))
        .chain([log_line(&id, "Error: attempt 29")])
        .collect();
    service.note_errors(lines.iter());
    let kept = lock(&service.inner.log_errors)
        .get(&id)
        .map(|e| e.lines.clone())
        .unwrap_or_default();
    assert_eq!(kept.len(), KEEP_ERRORS);
    assert_eq!(kept.last().map(String::as_str), Some("Error: attempt 29"));
    assert_eq!(kept.first().map(String::as_str), Some("Error: attempt 10"));
}

#[tokio::test]
async fn problems_a_log_session_hands_its_error_lines_and_only_those_to_the_service() {
    let service = ProblemsService::inert();
    *lock(&TAP) = Some(service.clone());
    let id = ContainerId("tapped".to_string());
    let lines = vec![
        log_line(&id, "INFO listening on 3000"),
        log_line(&id, "Error: connect ECONNREFUSED 10.0.0.1:5432"),
        log_line(&id, "GET /health 200"),
    ];
    let cancel = CancellationToken::new();
    let core = crate::services::logs::LogCore::start_merging(
        std::collections::BTreeMap::new(),
        move |_cancel, _core| vec![futures::StreamExt::boxed(futures::stream::iter(lines))],
        &cancel,
        Duration::ZERO,
    );
    let kept = async {
        loop {
            let found = lock(&service.inner.log_errors)
                .get(&id)
                .map(|e| e.lines.clone());
            if let Some(found) = found {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let kept = tokio::time::timeout(Duration::from_secs(5), kept)
        .await
        .expect("the session's error line reached the service");
    assert_eq!(kept, ["Error: connect ECONNREFUSED 10.0.0.1:5432"]);
    core.close();
    cancel.cancel();
}

#[test]
fn problems_a_container_that_changed_starts_its_error_lines_again() {
    let world = world();
    let service = ProblemsService::inert();
    let mut worker = world
        .containers
        .iter()
        .find(|c| c.name == "shop-api-worker-1")
        .cloned()
        .expect("worker");
    service.note_errors([log_line(&worker.id, "Error: connect ECONNREFUSED")].iter());
    assert_eq!(service.errors_for(&worker), ["Error: connect ECONNREFUSED"]);
    // Seen again with the same row: still there.
    assert_eq!(service.errors_for(&worker).len(), 1);
    // The container restarted (its row changed): what its old run printed no longer counts.
    worker.restart_count += 1;
    worker.state = ContainerState::Running;
    assert!(service.errors_for(&worker).is_empty());
}

#[test]
fn problems_a_log_batch_with_error_lines_wakes_the_service_and_a_clean_one_does_not() {
    let service = ProblemsService::inert();
    let version = || service.inner.version.load(Ordering::Relaxed);
    let id = ContainerId("c2".to_string());
    let before = version();
    service.note_errors(std::iter::empty());
    assert_eq!(version(), before);
    service.note_errors([log_line(&id, "Error: boom")].iter());
    assert_eq!(version(), before + 1);
    // The same line again changes nothing.
    service.note_errors([log_line(&id, "Error: boom")].iter());
    assert_eq!(version(), before + 1);
}

// ---------------------------------------------------------------------
// Explain
// ---------------------------------------------------------------------

fn world_of(world: &berth_engine::MockWorld) -> World {
    World {
        engines: world.engines.clone(),
        containers: world.containers.clone(),
        projects: world.projects.clone(),
    }
}

#[tokio::test]
async fn problems_explain_runs_the_rules_over_the_selected_lines() {
    let world = world();
    let service = ProblemsService::inert();
    let found = service
        .explain(
            world_of(&world),
            None,
            vec![ExplainLine {
                container: "shop-api-api-1".to_string(),
                text:
                    "Error response from daemon: Bind for 0.0.0.0:3000 failed: port is already allocated"
                        .to_string(),
            }],
        )
        .await;
    assert_eq!(found.len(), 1, "{:?}", titles(&found));
    assert_eq!(found[0].kind, ProblemKind::PortAllocated { port: 3000 });
    assert!(!found[0].fixes.is_empty());
}

#[tokio::test]
async fn problems_explain_says_nothing_for_lines_no_rule_knows_or_no_container_owns() {
    let world = world();
    let service = ProblemsService::inert();
    let plain = service
        .explain(
            world_of(&world),
            None,
            vec![ExplainLine {
                container: "shop-api-api-1".to_string(),
                text: "GET /health 200 3ms".to_string(),
            }],
        )
        .await;
    assert!(plain.is_empty(), "{:?}", titles(&plain));
    let stranger = service
        .explain(
            world_of(&world),
            None,
            vec![ExplainLine {
                container: "no-such-container".to_string(),
                text: "Error response from daemon: Bind for 0.0.0.0:3000 failed: port is already allocated"
                    .to_string(),
            }],
        )
        .await;
    assert!(stranger.is_empty());
}

#[tokio::test]
async fn problems_explain_does_not_touch_the_list_or_the_tap_buffers() {
    let world = world();
    let service = ProblemsService::inert();
    let _ = service
        .explain(
            world_of(&world),
            None,
            vec![ExplainLine {
                container: "shop-api-api-1".to_string(),
                text: "Error response from daemon: Bind for 0.0.0.0:3000 failed: port is already allocated"
                    .to_string(),
            }],
        )
        .await;
    assert!(lock(&service.inner.log_errors).is_empty());
    assert!(lock(&service.inner.cache).is_empty());
}

// ---------------------------------------------------------------------
// The service, running
// ---------------------------------------------------------------------

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some(state));
    });
    rsx! { div {} }
}

struct Rig {
    dom: VirtualDom,
    state: AppState,
    service: ProblemsService,
    cancel: CancellationToken,
}

fn rig() -> Rig {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT
        .with(|slot| slot.borrow_mut().take())
        .expect("harness captured AppState");
    let world = world();
    let clients = crate::services::engines::client_map();
    let engine =
        Arc::new(berth_engine::MockEngine::from_world(world.clone(), &engine_id()).expect("mock"));
    clients
        .lock()
        .expect("clients")
        .insert(engine_id(), engine as Arc<dyn EngineClient>);
    let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
    let cancel = CancellationToken::new();
    let services = Services {
        store: Arc::clone(&store),
        clients: clients.clone(),
        cancel: cancel.clone(),
    };
    let (actions, _finished) = ActionsService::new(
        clients,
        store,
        Arc::new(berth_engine::MockCompose::bundled()),
        Arc::new(berth_engine::MockColima::bundled()),
        cancel.clone(),
    );
    let mut running = state;
    let service = dom.in_scope(ScopeId::APP, || {
        running.engines.set(
            world
                .engines
                .iter()
                .filter(|e| e.id == engine_id())
                .cloned()
                .collect(),
        );
        running.containers.set(world.containers_for(&engine_id()));
        running.projects.set(
            world
                .projects
                .iter()
                .filter(|p| p.engine == engine_id())
                .cloned()
                .collect(),
        );
        ProblemsService::start(state, services, actions)
    });
    Rig {
        dom,
        state,
        service,
        cancel,
    }
}

impl Rig {
    fn problems(&self) -> Vec<Problem> {
        self.dom.in_runtime(|| self.state.problems.peek().clone())
    }

    /// Nudge, then let the service's task run until the list satisfies `done` or a few seconds pass.
    async fn settle(&mut self, done: impl Fn(&[Problem]) -> bool) -> Vec<Problem> {
        self.settle_for(Duration::from_secs(6), done).await
    }

    async fn settle_for(
        &mut self,
        wait: Duration,
        done: impl Fn(&[Problem]) -> bool,
    ) -> Vec<Problem> {
        self.dom.in_runtime(|| self.service.nudge());
        let deadline = std::time::Instant::now() + wait;
        loop {
            let _ = tokio::time::timeout(Duration::from_millis(50), self.dom.wait_for_work()).await;
            self.dom.render_immediate(&mut dioxus::core::NoOpMutations);
            let list = self.problems();
            if done(&list) || std::time::Instant::now() > deadline {
                return list;
            }
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[tokio::test]
async fn problems_the_service_fills_the_list_and_clears_a_problem_when_it_is_resolved() {
    let mut rig = rig();
    let first = rig
        .settle(|list| list.iter().any(|p| p.kind == ProblemKind::Unhealthy))
        .await;
    let kinds: Vec<&ProblemKind> = first.iter().map(|p| &p.kind).collect();
    assert!(
        kinds.contains(&&ProblemKind::PortAllocated { port: 5432 }),
        "{:?}",
        titles(&first)
    );
    assert!(kinds.contains(&&ProblemKind::Unhealthy));
    let port_before = first
        .iter()
        .find(|p| matches!(p.kind, ProblemKind::PortAllocated { .. }))
        .cloned()
        .expect("port problem");

    // The unhealthy container turns healthy: its problem goes, the other stays where it was.
    let unhealthy = first
        .iter()
        .find(|p| p.kind == ProblemKind::Unhealthy)
        .and_then(|p| match &p.subject {
            Subject::Container(id) => Some(id.clone()),
            _ => None,
        })
        .expect("unhealthy container");
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        for c in state.containers.write().iter_mut() {
            if c.id == unhealthy {
                c.health = Some(Health::Healthy);
                c.restart_count = 0;
            }
        }
    });
    let second = rig
        .settle(|list| !list.iter().any(|p| p.kind == ProblemKind::Unhealthy))
        .await;
    assert!(!second.iter().any(|p| p.kind == ProblemKind::Unhealthy));
    let port_after = second
        .iter()
        .find(|p| matches!(p.kind, ProblemKind::PortAllocated { .. }))
        .expect("port problem stays");
    assert_eq!(port_after.first_seen, port_before.first_seen);
    assert_eq!(port_after.id, port_before.id);
}

#[tokio::test]
async fn problems_the_service_keeps_the_list_while_an_engine_is_starting() {
    let mut rig = rig();
    let before = rig
        .settle(|list| list.iter().any(|p| p.kind == ProblemKind::Unhealthy))
        .await;
    assert!(!before.is_empty());
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        for engine in state.engines.write().iter_mut() {
            engine.state = EngineState::Starting;
        }
    });
    // A starting engine is not judged: what was listed stays, it is not read as "no problems".
    let after = rig.settle_for(Duration::from_millis(1500), |_| false).await;
    assert_eq!(after, before);
}

#[tokio::test]
async fn problems_a_stopped_engine_is_reported_by_its_socket_rule() {
    let mut rig = rig();
    let _ = rig
        .settle(|list| list.iter().any(|p| p.kind == ProblemKind::Unhealthy))
        .await;
    rig.dom.in_runtime(|| {
        let mut state = rig.state;
        for engine in state.engines.write().iter_mut() {
            engine.state = EngineState::Stopped;
        }
    });
    let list = rig
        .settle(|list| {
            list.iter()
                .any(|p| p.kind == ProblemKind::EngineSocketMissing)
        })
        .await;
    assert_eq!(list.len(), 1, "{:?}", titles(&list));
    assert_eq!(list[0].kind, ProblemKind::EngineSocketMissing);
}
