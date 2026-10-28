use super::*;
use berth_core::ContainerAction;
use berth_engine::client::Bs;
use berth_engine::ColimaControl as _;
use berth_engine::EngineClient as _;
use berth_engine::EngineError;
use futures::StreamExt;

// --- VirtualDom harness: real signals, captured from the root. ---

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = crate::state::use_app_state();
    use_hook(move || {
        SLOT.with(|slot| *slot.borrow_mut() = Some(state));
    });
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

// --- Test double: MockEngine with a scripted `act`. ---

enum Mode {
    /// Delegate straight to the mock.
    Normal,
    /// Wait for the gate before acting.
    Hold(Arc<tokio::sync::Semaphore>),
    /// Answer with a Docker error.
    Fail(String),
    /// Answer `Timeout` at once; the mock applies the action `after` later.
    SlowAct(std::time::Duration),
    /// Answer `Timeout` and never apply it.
    TimeoutOnly,
}

struct TestEngine {
    inner: Arc<berth_engine::MockEngine>,
    mode: Mode,
    acts: std::sync::Mutex<Vec<(berth_core::ContainerId, ContainerAction)>>,
}

impl TestEngine {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(berth_engine::MockEngine::bundled(&engine_id())),
            mode,
            acts: std::sync::Mutex::new(Vec::new()),
        })
    }
    fn acts(&self) -> Vec<(berth_core::ContainerId, ContainerAction)> {
        self.acts.lock().expect("acts lock").clone()
    }
}

#[async_trait::async_trait]
impl berth_engine::EngineClient for TestEngine {
    fn id(&self) -> &berth_core::EngineId {
        self.inner.id()
    }
    fn docker_host(&self) -> &str {
        self.inner.docker_host()
    }
    async fn info(&self) -> Result<berth_core::EngineInfo, EngineError> {
        self.inner.info().await
    }
    async fn containers(&self) -> Result<Vec<berth_core::Container>, EngineError> {
        self.inner.containers().await
    }
    async fn container(
        &self,
        id: &berth_core::ContainerId,
    ) -> Result<berth_core::ContainerDetail, EngineError> {
        self.inner.container(id).await
    }
    async fn images(&self) -> Result<Vec<berth_core::Image>, EngineError> {
        self.inner.images().await
    }
    async fn image(&self, id: &str) -> Result<berth_core::ImageDetail, EngineError> {
        self.inner.image(id).await
    }
    async fn volumes(&self) -> Result<Vec<berth_core::Volume>, EngineError> {
        self.inner.volumes().await
    }
    async fn networks(&self) -> Result<Vec<berth_core::Network>, EngineError> {
        self.inner.networks().await
    }
    async fn disk_usage(&self) -> Result<berth_core::DiskUsage, EngineError> {
        self.inner.disk_usage().await
    }
    async fn volume_sizes(
        &self,
    ) -> Result<std::collections::BTreeMap<String, bytesize::ByteSize>, EngineError> {
        self.inner.volume_sizes().await
    }
    fn events(&self, cancel: CancellationToken) -> Bs<berth_core::EngineEvent> {
        self.inner.events(cancel)
    }
    fn logs(
        &self,
        id: &berth_core::ContainerId,
        q: berth_core::LogQuery,
        cancel: CancellationToken,
    ) -> Bs<berth_core::LogLine> {
        self.inner.logs(id, q, cancel)
    }
    fn stats(
        &self,
        id: &berth_core::ContainerId,
        cancel: CancellationToken,
    ) -> Bs<berth_core::LiveStats> {
        self.inner.stats(id, cancel)
    }
    async fn act(
        &self,
        id: &berth_core::ContainerId,
        action: ContainerAction,
    ) -> Result<(), EngineError> {
        self.acts
            .lock()
            .expect("acts lock")
            .push((id.clone(), action.clone()));
        match &self.mode {
            Mode::Normal => self.inner.act(id, action).await,
            Mode::Hold(gate) => {
                if let Ok(permit) = gate.acquire().await {
                    permit.forget();
                }
                self.inner.act(id, action).await
            }
            Mode::Fail(message) => Err(EngineError::Docker(message.clone())),
            Mode::SlowAct(after) => {
                let inner = Arc::clone(&self.inner);
                let id = id.clone();
                let after = *after;
                tokio::spawn(async move {
                    tokio::time::sleep(after).await;
                    let _ = inner.act(&id, action).await;
                });
                Err(EngineError::Timeout(std::time::Duration::from_secs(2)))
            }
            Mode::TimeoutOnly => Err(EngineError::Timeout(std::time::Duration::from_secs(2))),
        }
    }
    async fn update_limits(
        &self,
        id: &berth_core::ContainerId,
        limits: &berth_core::Limits,
        restart: &berth_core::RestartPolicy,
    ) -> Result<(), EngineError> {
        self.inner.update_limits(id, limits, restart).await
    }
    async fn create(
        &self,
        spec: &berth_core::ContainerSpec,
        start: bool,
    ) -> Result<berth_core::ContainerId, EngineError> {
        self.inner.create(spec, start).await
    }
    async fn recreate(
        &self,
        id: &berth_core::ContainerId,
        spec: &berth_core::ContainerSpec,
    ) -> Result<berth_core::ContainerId, EngineError> {
        self.inner.recreate(id, spec).await
    }
    async fn exec(
        &self,
        id: &berth_core::ContainerId,
        req: berth_core::ExecRequest,
        cancel: CancellationToken,
    ) -> Result<berth_engine::ExecHandle, EngineError> {
        self.inner.exec(id, req, cancel).await
    }
    async fn list_files(
        &self,
        target: &berth_core::FileTarget,
        path: &str,
    ) -> Result<Vec<berth_core::FileEntry>, EngineError> {
        self.inner.list_files(target, path).await
    }
    fn download(
        &self,
        target: &berth_core::FileTarget,
        path: &str,
        cancel: CancellationToken,
    ) -> Bs<bytes::Bytes> {
        self.inner.download(target, path, cancel)
    }
    async fn upload(
        &self,
        target: &berth_core::FileTarget,
        dir: &str,
        tar: bytes::Bytes,
    ) -> Result<(), EngineError> {
        self.inner.upload(target, dir, tar).await
    }
    fn pull(
        &self,
        reference: &str,
        platform: Option<&str>,
        credentials: Option<&berth_engine::client::RegistryAuth>,
        cancel: CancellationToken,
    ) -> Bs<berth_core::PullProgress> {
        self.inner.pull(reference, platform, credentials, cancel)
    }
    async fn remove_image(&self, id: &str, force: bool) -> Result<(), EngineError> {
        self.inner.remove_image(id, force).await
    }
    async fn search_images(
        &self,
        term: &str,
        limit: u32,
    ) -> Result<Vec<berth_core::ImageSearchHit>, EngineError> {
        self.inner.search_images(term, limit).await
    }
    async fn create_volume(
        &self,
        spec: &berth_core::VolumeSpec,
    ) -> Result<berth_core::Volume, EngineError> {
        self.inner.create_volume(spec).await
    }
    async fn remove_volume(&self, name: &str) -> Result<(), EngineError> {
        self.inner.remove_volume(name).await
    }
    async fn create_network(
        &self,
        spec: &berth_core::NetworkSpec,
    ) -> Result<berth_core::Network, EngineError> {
        self.inner.create_network(spec).await
    }
    async fn remove_network(&self, id: &str) -> Result<(), EngineError> {
        self.inner.remove_network(id).await
    }
    async fn connect_network(
        &self,
        network: &str,
        id: &berth_core::ContainerId,
    ) -> Result<(), EngineError> {
        self.inner.connect_network(network, id).await
    }
    async fn disconnect_network(
        &self,
        network: &str,
        id: &berth_core::ContainerId,
    ) -> Result<(), EngineError> {
        self.inner.disconnect_network(network, id).await
    }
    async fn prune(
        &self,
        plan: &berth_core::PrunePlan,
        cancel: CancellationToken,
    ) -> Result<berth_core::PruneReport, EngineError> {
        self.inner.prune(plan, cancel).await
    }
}

/// A compose runner that fails with a message on every command.
struct FailingCompose(&'static str);

#[async_trait::async_trait]
impl berth_engine::ComposeRunner for FailingCompose {
    fn run(
        &self,
        _target: &berth_core::ComposeTarget,
        _cmd: berth_core::ComposeCommand,
        _cancel: CancellationToken,
    ) -> Bs<berth_core::ComposeEvent> {
        futures::stream::iter([
            Ok(berth_core::ComposeEvent::Line {
                stream: berth_core::Stream::Stderr,
                text: self.0.to_string(),
            }),
            Ok(berth_core::ComposeEvent::Done { code: 1 }),
        ])
        .boxed()
    }
    async fn config(
        &self,
        _target: &berth_core::ComposeTarget,
    ) -> Result<serde_json::Value, EngineError> {
        Ok(serde_json::json!({}))
    }
    async fn validate(
        &self,
        _folder: &std::path::Path,
        _files: &[std::path::PathBuf],
    ) -> Result<(), EngineError> {
        Ok(())
    }
    async fn validate_text(
        &self,
        _folder: &std::path::Path,
        _compose_text: &str,
        _env: &[(String, String)],
    ) -> Result<(), EngineError> {
        Ok(())
    }
}

fn engine_id() -> berth_core::EngineId {
    berth_core::EngineId("colima:default".to_string())
}

struct Rig {
    dom: VirtualDom,
    state: AppState,
    service: ActionsService,
    rx: tokio::sync::mpsc::UnboundedReceiver<Finished>,
    store: Arc<berth_store::Store>,
    engine: Arc<TestEngine>,
    compose: Arc<berth_engine::MockCompose>,
    colima: Arc<berth_engine::MockColima>,
}

impl Rig {
    fn new(mode: Mode) -> Self {
        let (dom, mut state) = harness();
        let world = berth_engine::MockWorld::bundled();
        let engine = TestEngine::new(mode);
        let clients = crate::services::engines::client_map();
        clients.lock().expect("clients").insert(
            engine_id(),
            Arc::clone(&engine) as Arc<dyn berth_engine::EngineClient>,
        );
        let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
        let compose = Arc::new(berth_engine::MockCompose::bundled());
        let colima = Arc::new(berth_engine::MockColima::bundled());
        let (service, rx) = ActionsService::new(
            clients,
            Arc::clone(&store),
            Arc::clone(&compose) as Arc<dyn berth_engine::ComposeRunner>,
            Arc::clone(&colima) as Arc<dyn berth_engine::ColimaControl>,
            CancellationToken::new(),
        );
        let service = service.with_settle(Settle {
            interval: std::time::Duration::from_millis(10),
            grace: std::time::Duration::from_millis(300),
        });
        dom.in_runtime(|| {
            state.engines.set(world.engines.clone());
            state.containers.set(world.containers_for(&engine_id()));
            state.projects.set(world.projects.clone());
            state
                .images
                .set(world.images_by_engine[&engine_id().0].clone());
            state
                .volumes
                .set(world.volumes_by_engine[&engine_id().0].clone());
            state
                .networks
                .set(world.networks_by_engine[&engine_id().0].clone());
        });
        Self {
            dom,
            state,
            service,
            rx,
            store,
            engine,
            compose,
            colima,
        }
    }

    fn container(&self, name: &str) -> berth_core::Container {
        self.dom.in_runtime(|| {
            self.state
                .containers
                .peek()
                .iter()
                .find(|c| c.name == name)
                .cloned()
                .unwrap_or_else(|| panic!("mock container {name}"))
        })
    }

    fn target(&self, name: &str) -> ContainerTarget {
        ContainerTarget::from(&self.container(name))
    }

    fn project(&self, name: &str) -> berth_core::Project {
        self.dom.in_runtime(|| {
            self.state
                .projects
                .peek()
                .iter()
                .find(|p| p.name == name)
                .cloned()
                .unwrap_or_else(|| panic!("mock project {name}"))
        })
    }

    fn pending(&self) -> Vec<(PendingKey, PendingState)> {
        self.dom.in_runtime(|| {
            self.state
                .pending
                .peek()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
    }

    fn do_container(&mut self, name: &str, action: ContainerAction) -> Result<(), String> {
        let target = self.target(name);
        let service = self.service.clone();
        let mut state = self.state;
        self.dom
            .in_runtime(|| service.container(&mut state, &target, action))
    }

    /// Wait for the next finished action and apply it.
    async fn settle_one(&mut self) -> PendingKey {
        let done = tokio::time::timeout(std::time::Duration::from_secs(5), self.rx.recv())
            .await
            .expect("a Finished within 5 s")
            .expect("channel open");
        let key = done.key.clone();
        let mut state = self.state;
        self.dom.in_runtime(|| apply_finished(&mut state, done));
        key
    }

    fn activity(&self) -> Vec<berth_core::ActivityEntry> {
        self.store
            .activity_query(&berth_store::ActivityFilter::default(), None, 50)
            .expect("activity")
    }
}

fn key(name: &str, rig: &Rig, action: &'static str) -> PendingKey {
    PendingKey {
        subject: Subject::Container(rig.container(name).id),
        action,
    }
}

const STOP: ContainerAction = ContainerAction::Stop { timeout_s: None };
const RESTART: ContainerAction = ContainerAction::Restart { timeout_s: None };

// --- Pending state ---

#[tokio::test]
async fn actions_pending_is_set_before_the_engine_answers() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let mut rig = Rig::new(Mode::Hold(Arc::clone(&gate)));
    rig.do_container("shop-api-api-1", STOP).expect("accepted");
    // Nothing has awaited yet: the engine has not been asked, the row is
    // unchanged, and the pending state is already visible.
    assert_eq!(
        rig.pending(),
        vec![(
            key("shop-api-api-1", &rig, "container.stop"),
            PendingState::Running {
                label: "Stopping…".to_string()
            }
        )]
    );
    assert_eq!(
        rig.container("shop-api-api-1").state,
        berth_core::ContainerState::Running
    );
    gate.add_permits(1);
    rig.settle_one().await;
    assert!(rig.pending().is_empty(), "success clears the pending state");
}

#[tokio::test]
async fn actions_labels_follow_the_action() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let mut rig = Rig::new(Mode::Hold(gate));
    rig.do_container("shop-api-api-1", RESTART)
        .expect("restart");
    rig.do_container("shop-api-redis-1", ContainerAction::Pause)
        .expect("pause");
    rig.do_container("shop-api-db-1", ContainerAction::Start)
        .expect("start");
    let labels: Vec<(&str, String)> = rig
        .pending()
        .into_iter()
        .map(|(k, v)| match v {
            PendingState::Running { label } => (k.action, label),
            PendingState::Failed { message } => (k.action, message),
        })
        .collect();
    assert!(labels.contains(&("container.restart", "Restarting…".to_string())));
    assert!(labels.contains(&("container.pause", "Pausing…".to_string())));
    assert!(labels.contains(&("container.start", "Starting…".to_string())));
    assert_eq!(pending_label("project.start"), "Starting…");
    assert_eq!(pending_label("project.stop"), "Stopping…");
    assert_eq!(pending_label("engine.restart"), "Restarting…");
    assert_eq!(pending_label("project.rebuild"), "Rebuilding…");
}

#[tokio::test]
async fn actions_success_updates_the_engine_and_clears_pending() {
    let mut rig = Rig::new(Mode::Normal);
    rig.do_container("shop-api-api-1", STOP).expect("accepted");
    rig.settle_one().await;
    assert!(rig.pending().is_empty());
    let detail = rig
        .engine
        .container(&rig.container("shop-api-api-1").id)
        .await
        .expect("detail");
    assert_eq!(detail.container.state, berth_core::ContainerState::Exited);
    assert_eq!(
        rig.engine.acts(),
        vec![(rig.container("shop-api-api-1").id, STOP)]
    );
}

#[tokio::test]
async fn actions_failure_restores_the_row_and_sets_failed_with_the_engine_message() {
    let mut rig = Rig::new(Mode::Fail("port is already allocated".to_string()));
    let before = rig.container("shop-api-api-1");
    rig.do_container("shop-api-api-1", STOP).expect("accepted");
    assert!(matches!(rig.pending()[0].1, PendingState::Running { .. }));
    rig.settle_one().await;
    assert_eq!(
        rig.pending(),
        vec![(
            key("shop-api-api-1", &rig, "container.stop"),
            PendingState::Failed {
                message: "port is already allocated".to_string()
            }
        )]
    );
    assert_eq!(rig.container("shop-api-api-1"), before, "row unchanged");
}

#[tokio::test]
async fn actions_a_new_attempt_replaces_the_failure_and_dismiss_clears_it() {
    let mut rig = Rig::new(Mode::Fail("boom".to_string()));
    rig.do_container("shop-api-api-1", STOP).expect("first");
    rig.settle_one().await;
    let failed_key = key("shop-api-api-1", &rig, "container.stop");
    assert!(matches!(rig.pending()[0].1, PendingState::Failed { .. }));
    // A failure does not block retrying.
    rig.do_container("shop-api-api-1", STOP).expect("retry");
    assert!(matches!(rig.pending()[0].1, PendingState::Running { .. }));
    // Dismiss leaves a running action alone...
    let mut state = rig.state;
    rig.dom.in_runtime(|| dismiss(&mut state, &failed_key));
    assert_eq!(rig.pending().len(), 1);
    rig.settle_one().await;
    // ...and removes a failure.
    rig.dom.in_runtime(|| dismiss(&mut state, &failed_key));
    assert!(rig.pending().is_empty());
}

#[tokio::test]
async fn actions_second_press_is_refused_while_the_first_runs() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let mut rig = Rig::new(Mode::Hold(Arc::clone(&gate)));
    rig.do_container("shop-api-api-1", STOP).expect("first");
    let second = rig.do_container("shop-api-api-1", RESTART);
    assert_eq!(second, Err("Stopping… is still in progress".to_string()));
    assert_eq!(rig.pending().len(), 1);
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert_eq!(rig.engine.acts().len(), 1, "the engine was asked once");
    gate.add_permits(1);
    rig.settle_one().await;
}

#[tokio::test]
async fn actions_disabled_action_is_refused_and_sets_nothing() {
    let mut rig = Rig::new(Mode::Normal);
    let result = rig.do_container("shop-api-api-1", ContainerAction::Start);
    assert_eq!(result, Err("Already running".to_string()));
    assert!(rig.pending().is_empty());
    assert!(rig.engine.acts().is_empty());
}

#[tokio::test]
async fn actions_unconnected_engine_is_refused_before_pending() {
    let rig = Rig::new(Mode::Normal);
    let mut target = rig.target("shop-api-api-1");
    target.engine = berth_core::EngineId("socket:/nowhere".to_string());
    let service = rig.service.clone();
    let mut state = rig.state;
    let result = rig
        .dom
        .in_runtime(|| service.container(&mut state, &target, STOP));
    assert_eq!(
        result,
        Err("Engine socket:/nowhere is not connected".to_string())
    );
    assert!(rig.pending().is_empty());
}

// --- Activity ---

#[tokio::test]
async fn actions_every_action_writes_an_activity_entry_with_its_outcome() {
    let mut rig = Rig::new(Mode::Normal);
    let target = rig.target("shop-api-api-1");
    rig.do_container("shop-api-api-1", RESTART).expect("ok");
    rig.settle_one().await;
    let entries = rig.activity();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, "container.restart");
    assert_eq!(entries[0].origin, berth_core::ActivityOrigin::Berth);
    assert_eq!(entries[0].subject, Subject::Container(target.id.clone()));
    assert_eq!(entries[0].subject_name, "shop-api-api-1");
    assert_eq!(entries[0].engine, engine_id());
    assert_eq!(entries[0].project, target.project);
    assert_eq!(entries[0].outcome, berth_core::Outcome::Ok);
}

#[tokio::test]
async fn actions_failed_action_writes_a_failed_activity_entry() {
    let mut rig = Rig::new(Mode::Fail("no such container".to_string()));
    rig.do_container("shop-api-api-1", STOP).expect("accepted");
    rig.settle_one().await;
    let entries = rig.activity();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].outcome,
        berth_core::Outcome::Failed {
            reason: "no such container".to_string()
        }
    );
}

// --- Several containers in one call ---

#[tokio::test]
async fn actions_restart_of_three_containers_in_one_call() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let mut rig = Rig::new(Mode::Hold(Arc::clone(&gate)));
    let names = ["shop-api-api-1", "shop-api-redis-1", "shop-api-mailpit-1"];
    let targets: Vec<ContainerTarget> = names.iter().map(|n| rig.target(n)).collect();
    let service = rig.service.clone();
    let mut state = rig.state;
    let results = rig
        .dom
        .in_runtime(|| service.containers(&mut state, &targets, &RESTART));
    assert!(results.iter().all(Result::is_ok), "{results:?}");
    assert_eq!(
        rig.pending().len(),
        3,
        "all three pending before any answer"
    );
    for _ in 0..3 {
        gate.add_permits(1);
    }
    for _ in 0..3 {
        rig.settle_one().await;
    }
    assert!(rig.pending().is_empty());
    assert_eq!(rig.engine.acts().len(), 3);
    assert_eq!(rig.activity().len(), 3);
    for name in names {
        assert!(rig.container(name).restart_count < 1000);
    }
}

#[tokio::test]
async fn actions_bulk_reports_each_refusal_and_runs_the_rest() {
    let mut rig = Rig::new(Mode::Normal);
    let targets = vec![rig.target("shop-api-api-1"), rig.target("shop-api-db-1")];
    let service = rig.service.clone();
    let mut state = rig.state;
    let results = rig
        .dom
        .in_runtime(|| service.containers(&mut state, &targets, &STOP));
    assert_eq!(results[0], Ok(()));
    assert_eq!(results[1], Err("Not running".to_string()));
    rig.settle_one().await;
    assert!(rig.pending().is_empty());
}

// --- bollard's 2 s timeout ---

#[tokio::test]
async fn actions_stop_that_times_out_is_polled_until_the_container_exits() {
    let mut rig = Rig::new(Mode::SlowAct(std::time::Duration::from_millis(80)));
    let started = std::time::Instant::now();
    rig.do_container(
        "shop-api-api-1",
        ContainerAction::Stop { timeout_s: Some(0) },
    )
    .expect("accepted");
    rig.settle_one().await;
    assert!(rig.pending().is_empty(), "polled to success, not Timeout");
    assert!(started.elapsed() >= std::time::Duration::from_millis(80));
    assert_eq!(rig.activity()[0].outcome, berth_core::Outcome::Ok);
}

/// The live container list carries no start times, so a target built
/// from it has `started: None`. A restart that times out must still be
/// judged by inspecting the container, against the start time read
/// before the action: the old run is not "restarted".
#[tokio::test]
async fn actions_restart_that_times_out_waits_for_a_new_run_without_list_start_times() {
    let mut rig = Rig::new(Mode::SlowAct(std::time::Duration::from_millis(80)));
    let mut target = rig.target("shop-api-api-1");
    let old_started = rig.container("shop-api-api-1").started;
    assert!(old_started.is_some(), "the mock container has run before");
    target.started = None;
    let service = rig.service.clone();
    let mut state = rig.state;
    let began = std::time::Instant::now();
    rig.dom
        .in_runtime(|| {
            service.container(
                &mut state,
                &target,
                ContainerAction::Restart { timeout_s: Some(0) },
            )
        })
        .expect("accepted");
    rig.settle_one().await;
    assert!(
        began.elapsed() >= std::time::Duration::from_millis(80),
        "settled on the old run after {:?}",
        began.elapsed()
    );
    assert!(rig.pending().is_empty(), "{:?}", rig.pending());
    assert_eq!(rig.activity()[0].outcome, berth_core::Outcome::Ok);
}

#[tokio::test]
async fn actions_stop_that_never_settles_fails_with_a_message_not_a_timeout_error() {
    let mut rig = Rig::new(Mode::TimeoutOnly);
    rig.do_container(
        "shop-api-api-1",
        ContainerAction::Stop { timeout_s: Some(0) },
    )
    .expect("accepted");
    rig.settle_one().await;
    match &rig.pending()[0].1 {
        PendingState::Failed { message } => {
            assert!(
                message.starts_with("The engine did not finish within"),
                "{message}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn actions_settled_reads_the_outcome_of_each_action() {
    use berth_core::ContainerState as S;
    let mut running = berth_engine::MockWorld::bundled()
        .containers
        .into_iter()
        .find(|c| c.state == S::Running)
        .expect("running container");
    let t0 = chrono::DateTime::from_timestamp(1_000, 0);
    let t1 = chrono::DateTime::from_timestamp(2_000, 0);
    running.started = t0;
    let mut exited = running.clone();
    exited.state = S::Exited;
    assert!(!settled(&STOP, Some(&running), t0));
    assert!(settled(&STOP, Some(&exited), t0));
    assert!(settled(&STOP, None, t0), "gone counts as stopped");
    assert!(!settled(&ContainerAction::Start, Some(&exited), t0));
    assert!(settled(&ContainerAction::Start, Some(&running), t0));
    // Restart: still the old run, then a new one.
    assert!(!settled(&RESTART, Some(&running), t0));
    running.started = t1;
    assert!(settled(&RESTART, Some(&running), t0));
    assert!(!settled(&RESTART, Some(&exited), t0));
    assert!(!settled(
        &ContainerAction::Remove {
            force: false,
            volumes: false
        },
        Some(&running),
        t0
    ));
    assert!(settled(
        &ContainerAction::Remove {
            force: false,
            volumes: false
        },
        None,
        t0
    ));
}

// --- Project and engine actions ---

#[tokio::test]
async fn actions_stop_project_then_start_project_run_compose_stop_then_up() {
    let mut rig = Rig::new(Mode::Normal);
    let project = rig.project("shop-api");
    let service = rig.service.clone();
    let mut state = rig.state;
    rig.dom
        .in_runtime(|| service.project(&mut state, &project, ProjectOp::Stop))
        .expect("stop accepted");
    assert_eq!(
        rig.pending(),
        vec![(
            PendingKey {
                subject: Subject::Project(project.id.clone()),
                action: "project.stop"
            },
            PendingState::Running {
                label: "Stopping…".to_string()
            }
        )]
    );
    rig.settle_one().await;
    assert!(rig.pending().is_empty());
    // shop-api is Failing (not stopped), so Start is allowed too.
    rig.dom
        .in_runtime(|| service.project(&mut state, &project, ProjectOp::Start))
        .expect("start accepted");
    rig.settle_one().await;
    let recorded = rig.compose.recorded();
    assert_eq!(recorded.len(), 2);
    assert_eq!(
        recorded[0].1,
        berth_core::ComposeCommand::Stop { services: vec![] }
    );
    assert_eq!(
        recorded[1].1,
        berth_core::ComposeCommand::Up {
            services: vec![],
            build: false
        }
    );
    assert_eq!(recorded[0].0.project_name, "shop-api");
    assert_eq!(
        recorded[0].0.folder,
        project.folder.clone().expect("folder")
    );
    assert_eq!(recorded[0].0.docker_host, rig.engine.docker_host());
    let actions: Vec<String> = rig.activity().into_iter().map(|e| e.action).collect();
    assert!(actions.contains(&"project.stop".to_string()));
    assert!(actions.contains(&"project.start".to_string()));
    assert!(rig
        .activity()
        .iter()
        .all(|e| e.project == Some(project.id.clone())));
}

#[tokio::test]
async fn actions_delete_project_runs_down_with_the_sheets_options() {
    let mut rig = Rig::new(Mode::Normal);
    let project = rig.project("shop-api");
    let service = rig.service.clone();
    let mut state = rig.state;
    rig.dom
        .in_runtime(|| {
            service.project(
                &mut state,
                &project,
                ProjectOp::Delete {
                    volumes: true,
                    remove_images: false,
                },
            )
        })
        .expect("accepted");
    rig.settle_one().await;
    assert_eq!(
        rig.compose.recorded()[0].1,
        berth_core::ComposeCommand::Down {
            volumes: true,
            remove_images: false
        }
    );
}

#[tokio::test]
async fn actions_project_failure_shows_the_compose_error() {
    let mut rig = Rig::new(Mode::Normal);
    let (service, rx) = ActionsService::new(
        Arc::clone(&rig.service.inner.clients),
        Arc::clone(&rig.store),
        Arc::new(FailingCompose("port 5432 is already allocated")),
        Arc::clone(&rig.colima) as Arc<dyn berth_engine::ColimaControl>,
        CancellationToken::new(),
    );
    rig.rx = rx;
    let project = rig.project("shop-api");
    let mut state = rig.state;
    rig.dom
        .in_runtime(|| service.project(&mut state, &project, ProjectOp::Restart))
        .expect("accepted");
    rig.service = service;
    rig.settle_one().await;
    assert_eq!(
        rig.pending()[0].1,
        PendingState::Failed {
            message: "port 5432 is already allocated".to_string()
        }
    );
    assert_eq!(
        rig.activity()[0].outcome,
        berth_core::Outcome::Failed {
            reason: "port 5432 is already allocated".to_string()
        }
    );
}

/// A compose runner that fails as long as `failing` is set and then succeeds.
struct FlakyCompose {
    failing: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl berth_engine::ComposeRunner for FlakyCompose {
    fn run(
        &self,
        _target: &berth_core::ComposeTarget,
        _cmd: berth_core::ComposeCommand,
        _cancel: CancellationToken,
    ) -> Bs<berth_core::ComposeEvent> {
        if self.failing.load(std::sync::atomic::Ordering::SeqCst) {
            futures::stream::iter([
                Ok(berth_core::ComposeEvent::Line {
                    stream: berth_core::Stream::Stderr,
                    text: "port 5432 is already allocated".to_string(),
                }),
                Ok(berth_core::ComposeEvent::Progress {
                    id: "Container shop-api-db-1".to_string(),
                    status: "Error".to_string(),
                    text: Some("Ports are not available".to_string()),
                }),
                Ok(berth_core::ComposeEvent::Done { code: 1 }),
            ])
            .boxed()
        } else {
            futures::stream::iter([Ok(berth_core::ComposeEvent::Done { code: 0 })]).boxed()
        }
    }
    async fn config(
        &self,
        _target: &berth_core::ComposeTarget,
    ) -> Result<serde_json::Value, EngineError> {
        Ok(serde_json::json!({}))
    }
    async fn validate(
        &self,
        _folder: &std::path::Path,
        _files: &[std::path::PathBuf],
    ) -> Result<(), EngineError> {
        Ok(())
    }
    async fn validate_text(
        &self,
        _folder: &std::path::Path,
        _compose_text: &str,
        _env: &[(String, String)],
    ) -> Result<(), EngineError> {
        Ok(())
    }
}

#[tokio::test]
async fn actions_compose_failure_lines_are_kept_per_project_until_the_next_success() {
    let mut rig = Rig::new(Mode::Normal);
    let flaky = Arc::new(FlakyCompose {
        failing: std::sync::atomic::AtomicBool::new(true),
    });
    let (service, rx) = ActionsService::new(
        Arc::clone(&rig.service.inner.clients),
        Arc::clone(&rig.store),
        Arc::clone(&flaky) as Arc<dyn berth_engine::ComposeRunner>,
        Arc::clone(&rig.colima) as Arc<dyn berth_engine::ColimaControl>,
        CancellationToken::new(),
    );
    rig.rx = rx;
    rig.service = service.clone();
    let shop = rig.project("shop-api");
    let other = rig.project("analytics");
    let mut state = rig.state;
    assert!(service.compose_failure_lines(&shop.id).is_empty());

    rig.dom
        .in_runtime(|| service.project(&mut state, &shop, ProjectOp::Start))
        .expect("accepted");
    rig.settle_one().await;
    assert_eq!(
        service.compose_failure_lines(&shop.id),
        [
            "port 5432 is already allocated",
            "Container shop-api-db-1: Ports are not available"
        ]
    );
    assert!(
        service.compose_failure_lines(&other.id).is_empty(),
        "another project has no failure"
    );

    // A failed second attempt replaces the lines rather than adding to them.
    rig.dom
        .in_runtime(|| service.project(&mut state, &shop, ProjectOp::Start))
        .expect("accepted after the failure was shown");
    rig.settle_one().await;
    assert_eq!(service.compose_failure_lines(&shop.id).len(), 2);

    // A success clears them.
    flaky
        .failing
        .store(false, std::sync::atomic::Ordering::SeqCst);
    rig.dom
        .in_runtime(|| service.project(&mut state, &shop, ProjectOp::Start))
        .expect("accepted");
    rig.settle_one().await;
    assert!(service.compose_failure_lines(&shop.id).is_empty());
}

#[tokio::test]
async fn actions_engine_stop_runs_colima_and_clears_pending() {
    let mut rig = Rig::new(Mode::Normal);
    let engine = rig
        .dom
        .in_runtime(|| {
            rig.state
                .engines
                .peek()
                .iter()
                .find(|e| e.id == engine_id())
                .cloned()
        })
        .expect("engine");
    let service = rig.service.clone();
    let mut state = rig.state;
    rig.dom
        .in_runtime(|| service.engine(&mut state, &engine, berth_core::ColimaOp::Stop))
        .expect("accepted");
    assert_eq!(
        rig.pending()[0].1,
        PendingState::Running {
            label: "Stopping…".to_string()
        }
    );
    rig.settle_one().await;
    assert!(rig.pending().is_empty());
    assert!(!rig.colima.status("default").await.expect("profile").running);
    assert_eq!(rig.activity()[0].action, "engine.stop");
    assert_eq!(rig.activity()[0].subject, Subject::Engine(engine_id()));
}

// --- Disabled reasons ---

fn reason(rig: &Rig, subject: &Subject, action: &str) -> Option<String> {
    rig.dom
        .in_runtime(|| with_view(&rig.state, |view| disabled_reason(subject, action, view)))
}

fn container_subject(rig: &Rig, name: &str) -> Subject {
    Subject::Container(rig.container(name).id)
}

/// One place decides it: `disabled_reason` answers for every action the
/// mock engine cannot do, under mock data only, ahead of any other reason.
#[tokio::test]
async fn actions_mock_data_disables_terminal_create_and_files_actions_and_a_live_engine_does_not() {
    use crate::services::backing::Backing;
    let rig = Rig::new(Mode::Normal);
    let api = container_subject(&rig, "shop-api-api-1");
    let set = |backing| {
        let mut slot = rig.state.backing;
        rig.dom.in_runtime(|| slot.set(backing));
    };
    let actions = [
        "container.terminal",
        "container.edit",
        "container.duplicate",
        "container.recreate_latest",
    ];
    set(Backing::Live);
    for action in actions {
        assert_eq!(reason(&rig, &api, action), None, "{action} live");
    }
    set(Backing::Mock);
    for action in actions {
        let why = reason(&rig, &api, action).unwrap_or_default();
        assert!(
            why.ends_with("need a real Docker engine. You're viewing mock data."),
            "{action}: {why}"
        );
    }
    assert_eq!(
        reason(&rig, &api, "container.terminal").as_deref(),
        Some("Terminals need a real Docker engine. You're viewing mock data.")
    );
    // What the mock engine can do stays enabled.
    assert_eq!(reason(&rig, &api, "container.restart"), None);
}

#[tokio::test]
async fn actions_disabled_reasons_for_containers_follow_their_state() {
    let rig = Rig::new(Mode::Normal);
    let running = container_subject(&rig, "shop-api-api-1");
    let exited = container_subject(&rig, "shop-api-db-1");
    let paused = container_subject(&rig, "api-worker-7f9c4d8b6-xk2lp");
    assert_eq!(
        reason(&rig, &running, "container.start").as_deref(),
        Some("Already running")
    );
    assert_eq!(reason(&rig, &running, "container.stop"), None);
    assert_eq!(reason(&rig, &running, "container.pause"), None);
    assert_eq!(
        reason(&rig, &running, "container.resume").as_deref(),
        Some("Not paused")
    );
    assert_eq!(reason(&rig, &exited, "container.start"), None);
    assert_eq!(
        reason(&rig, &exited, "container.stop").as_deref(),
        Some("Not running")
    );
    assert_eq!(
        reason(&rig, &exited, "container.pause").as_deref(),
        Some("Only a running container can be paused")
    );
    assert_eq!(
        reason(&rig, &exited, "container.kill").as_deref(),
        Some("Not running")
    );
    assert_eq!(reason(&rig, &exited, "container.delete"), None);
    assert_eq!(reason(&rig, &exited, "container.restart"), None);
    assert_eq!(reason(&rig, &paused, "container.resume"), None);
    assert_eq!(reason(&rig, &paused, "container.stop"), None);
    assert_eq!(
        reason(&rig, &paused, "container.start").as_deref(),
        Some("Paused. Resume it instead")
    );
    assert_eq!(
        reason(
            &rig,
            &Subject::Container(berth_core::ContainerId("gone".to_string())),
            "container.stop"
        )
        .as_deref(),
        Some("This container no longer exists")
    );
}

#[tokio::test]
async fn actions_disabled_reason_names_the_stopped_engine() {
    let rig = Rig::new(Mode::Normal);
    let mut container = rig.container("shop-api-api-1");
    container.engine = berth_core::EngineId("colima:work".to_string());
    let mut state = rig.state;
    rig.dom
        .in_runtime(|| state.containers.set(vec![container.clone()]));
    assert_eq!(
        reason(&rig, &Subject::Container(container.id), "container.stop").as_deref(),
        Some("Engine work is stopped")
    );
}

#[tokio::test]
async fn actions_delete_image_in_use_is_disabled_with_the_count() {
    let rig = Rig::new(Mode::Normal);
    let (used, unused) = rig.dom.in_runtime(|| {
        let images = rig.state.images.peek();
        (
            images
                .iter()
                .find(|i| i.used_by.len() == 2)
                .cloned()
                .expect("used by two"),
            images
                .iter()
                .find(|i| i.used_by.is_empty())
                .cloned()
                .expect("unused"),
        )
    });
    assert_eq!(
        reason(&rig, &Subject::Image(used.id.clone()), "image.delete").as_deref(),
        Some("Used by 2 containers")
    );
    assert_eq!(
        reason(&rig, &Subject::Image(unused.id), "image.delete"),
        None
    );
    assert_eq!(reason(&rig, &Subject::Image(used.id), "image.pull"), None);
    let one = rig.dom.in_runtime(|| {
        rig.state
            .images
            .peek()
            .iter()
            .find(|i| i.used_by.len() == 1)
            .cloned()
            .expect("one")
    });
    assert_eq!(
        reason(&rig, &Subject::Image(one.id), "image.delete").as_deref(),
        Some("Used by 1 container")
    );
}

#[tokio::test]
async fn actions_delete_volume_and_network_in_use_are_disabled() {
    let rig = Rig::new(Mode::Normal);
    assert_eq!(
        reason(
            &rig,
            &Subject::Volume("shop-api_pgdata".to_string()),
            "volume.delete"
        )
        .as_deref(),
        Some("Used by 1 container")
    );
    assert_eq!(
        reason(
            &rig,
            &Subject::Volume("stale-cache".to_string()),
            "volume.delete"
        ),
        None
    );
    assert_eq!(
        reason(
            &rig,
            &Subject::Network("bridge".to_string()),
            "network.delete"
        )
        .as_deref(),
        Some("Built-in networks cannot be removed")
    );
    assert_eq!(
        reason(
            &rig,
            &Subject::Network("shop-api_default".to_string()),
            "network.delete"
        )
        .as_deref(),
        Some("5 containers are connected")
    );
}

#[tokio::test]
async fn actions_disabled_reasons_for_projects_follow_status_and_files() {
    let rig = Rig::new(Mode::Normal);
    let id = |name: &str| Subject::Project(rig.project(name).id);
    // berth-site is AllRunning, ml-notebooks Stopped, docker-manger NotCreated
    // with no Compose file, landing is remote-only with no folder.
    assert_eq!(
        reason(&rig, &id("berth-site"), "project.start").as_deref(),
        Some("All services are already running")
    );
    assert_eq!(reason(&rig, &id("berth-site"), "project.stop"), None);
    assert_eq!(reason(&rig, &id("ml-notebooks"), "project.start"), None);
    assert_eq!(
        reason(&rig, &id("ml-notebooks"), "project.stop").as_deref(),
        Some("Nothing is running")
    );
    assert_eq!(
        reason(&rig, &id("ml-notebooks"), "project.restart").as_deref(),
        Some("Nothing is running")
    );
    assert_eq!(
        reason(&rig, &id("docker-manger"), "project.start").as_deref(),
        Some("No Compose file is known for this project")
    );
    assert_eq!(reason(&rig, &id("ml-notebooks"), "project.delete"), None);
    assert_eq!(
        reason(&rig, &id("landing"), "project.stop").as_deref(),
        Some("Engine hetzner-1 is unreachable")
    );
}

#[tokio::test]
async fn actions_disabled_reasons_for_engines() {
    let rig = Rig::new(Mode::Normal);
    let default = Subject::Engine(engine_id());
    let work = Subject::Engine(berth_core::EngineId("colima:work".to_string()));
    let ssh = Subject::Engine(berth_core::EngineId("ssh:hetzner-1".to_string()));
    assert_eq!(
        reason(&rig, &default, "engine.start").as_deref(),
        Some("Already running")
    );
    assert_eq!(reason(&rig, &default, "engine.stop"), None);
    assert_eq!(reason(&rig, &work, "engine.start"), None);
    assert_eq!(
        reason(&rig, &work, "engine.stop").as_deref(),
        Some("Already stopped")
    );
    assert_eq!(
        reason(&rig, &ssh, "engine.start").as_deref(),
        Some("Only Colima engines can be started or stopped here")
    );
}
