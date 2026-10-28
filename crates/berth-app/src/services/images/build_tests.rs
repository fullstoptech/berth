//! P8-T3 tests for the build, push, export and import service: how events
//! fold into states, what a build is refused for, and the store's paths
//! over the mock engine (states, pending keys, credentials, cancel).

use std::sync::{Arc, Mutex};

use berth_core::{BuildContext, BuildEvent, Image, PullProgress};
use berth_engine::client::{Bs, RegistryAuth};
use berth_engine::{EngineClient, EngineError};
use berth_store::{InMemorySecretStore, SecretStore};
use dioxus::core::current_scope_id;
use secrecy::ExposeSecret as _;

use super::*;
use crate::services::registries::{secret_key, RegistriesService};
use crate::state::use_app_state;

fn default_engine() -> EngineId {
    EngineId("colima:default".to_string())
}

fn inline(text: &str, tags: &[&str]) -> BuildSpec {
    BuildSpec {
        context: BuildContext::Inline(text.to_string()),
        dockerfile: None,
        tags: tags.iter().map(ToString::to_string).collect(),
        build_args: Vec::new(),
        platform: None,
    }
}

// ---------------------------------------------------------------------
// Pure
// ---------------------------------------------------------------------

fn step(index: u32, total: u32, text: &str) -> BuildEvent {
    BuildEvent::Step {
        index,
        total,
        text: text.to_string(),
    }
}

#[test]
fn build_state_follows_the_steps_and_names_the_step_that_failed() {
    let mut state = BuildState::new(default_engine(), &inline("FROM alpine", &["a:1"]));
    assert_eq!(state.fraction(), None, "no step yet");
    state.apply(&step(1, 4, "Step 1/4 : FROM alpine"));
    state.apply(&BuildEvent::Log(" ---> abc".to_string()));
    state.apply(&step(2, 4, "Step 2/4 : RUN false"));
    assert_eq!(state.step, Some((2, 4)));
    assert_eq!(state.fraction(), Some(0.25), "step 2 is under way");
    state.apply(&BuildEvent::Failed {
        text: "The command returned a non-zero code: 1".to_string(),
    });
    assert_eq!(state.failed_step, Some(2));
    assert_eq!(
        state.failure.as_deref(),
        Some("The command returned a non-zero code: 1")
    );
    let kinds: Vec<_> = state.lines.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        [
            BuildLineKind::Step,
            BuildLineKind::Log,
            BuildLineKind::Step,
            BuildLineKind::Failed
        ]
    );
    assert_eq!(state.context, "Dockerfile");
    assert_eq!(state.key(), (default_engine(), "a:1".to_string()));
}

#[test]
fn build_state_is_whole_when_done_and_keeps_only_the_newest_lines() {
    let mut state = BuildState::new(default_engine(), &inline("FROM alpine", &["a:1"]));
    state.apply(&step(3, 3, "Step 3/3 : CMD x"));
    state.phase = BuildPhase::Done;
    assert_eq!(state.fraction(), Some(1.0));
    for n in 0..(LOG_CAP + 10) {
        state.apply(&BuildEvent::Log(format!("line {n}")));
    }
    assert_eq!(state.lines.len(), LOG_CAP);
    assert_eq!(state.lines[0].text, "line 10", "the oldest went first");
    assert_eq!(
        state.lines.last().map(|l| l.text.as_str()),
        Some(format!("line {}", LOG_CAP + 9).as_str())
    );
}

#[test]
fn push_state_keeps_the_latest_line_and_the_digest() {
    let mut state = PushState::new(default_engine(), "reg.example/app:1");
    let line = |status: &str| PullProgress {
        layer: None,
        status: status.to_string(),
        current: None,
        total: None,
    };
    state.apply(&line("The push refers to repository [reg.example/app]"));
    state.apply(&line(""));
    assert_eq!(
        state.note,
        "The push refers to repository [reg.example/app]"
    );
    assert_eq!(state.digest, None);
    state.apply(&line("1: digest: sha256:0123abcd size: 527"));
    assert_eq!(state.digest.as_deref(), Some("sha256:0123abcd"));
    state.apply(&line("Pushing"));
    assert_eq!(state.note, "Pushing");
    assert_eq!(
        state.digest.as_deref(),
        Some("sha256:0123abcd"),
        "a later line keeps it"
    );
}

#[test]
fn build_check_spec_refuses_what_cannot_build_and_tidies_the_tags() {
    let err = |spec| check_build_spec(spec).unwrap_err();
    assert!(err(inline("FROM alpine", &[])).contains("name"));
    assert!(err(inline("FROM alpine", &["Bad Name"])).contains("no spaces"));
    assert!(err(inline("FROM alpine", &["a@sha256:0123456789abcdef"])).contains("digest"));
    assert!(err(inline("  \n", &["a:1"])).contains("empty"));
    let mut blank_arg = inline("FROM alpine", &["a:1"]);
    blank_arg
        .build_args
        .push((" ".to_string(), "x".to_string()));
    assert!(err(blank_arg).contains("argument"));

    let ok = check_build_spec(inline("FROM alpine", &[" a:1 ", "a:1", "b"])).expect("ok");
    assert_eq!(ok.tags, ["a:1", "b"]);

    let dir = tempfile::tempdir().expect("dir");
    let folder = |dockerfile: Option<&str>| BuildSpec {
        context: BuildContext::Folder(dir.path().to_path_buf()),
        dockerfile: dockerfile.map(str::to_string),
        tags: vec!["a:1".to_string()],
        build_args: Vec::new(),
        platform: None,
    };
    assert!(err(folder(None)).contains("There is no Dockerfile"));
    std::fs::write(dir.path().join("Dockerfile"), "FROM alpine\n").expect("write");
    assert!(check_build_spec(folder(None)).is_ok());
    assert!(err(folder(Some("Other.dockerfile"))).contains("Other.dockerfile"));
    let missing = BuildSpec {
        context: BuildContext::Folder(dir.path().join("nope")),
        ..folder(None)
    };
    assert!(err(missing).contains("not a folder"));
}

// ---------------------------------------------------------------------
// The store over a recording mock engine
// ---------------------------------------------------------------------

/// One call and the (username, password, server) it carried, if any.
type Seen = (&'static str, Option<(String, String, String)>);

/// The mock engine, remembering which credentials each build and push
/// was given.
struct Recorder {
    inner: berth_engine::MockEngine,
    seen: Mutex<Vec<Seen>>,
}

impl Recorder {
    fn note(&self, call: &'static str, auth: Option<&RegistryAuth>) {
        let auth = auth.map(|a| {
            (
                a.username.clone(),
                a.password.expose_secret().to_string(),
                a.server.clone(),
            )
        });
        self.seen.lock().expect("seen").push((call, auth));
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("seen").clone()
    }
}

#[async_trait::async_trait]
impl EngineClient for Recorder {
    fn id(&self) -> &EngineId {
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
    async fn images(&self) -> Result<Vec<Image>, EngineError> {
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
        action: berth_core::ContainerAction,
    ) -> Result<(), EngineError> {
        self.inner.act(id, action).await
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
        credentials: Option<&RegistryAuth>,
        cancel: CancellationToken,
    ) -> Bs<PullProgress> {
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
    async fn tag_image(&self, id: &str, repo: &str, tag: &str) -> Result<(), EngineError> {
        self.inner.tag_image(id, repo, tag).await
    }
    fn push(
        &self,
        reference: &str,
        auth: Option<&RegistryAuth>,
        cancel: CancellationToken,
    ) -> Bs<PullProgress> {
        self.note("push", auth);
        self.inner.push(reference, auth, cancel)
    }
    fn build_image(
        &self,
        spec: &BuildSpec,
        credentials: Option<&RegistryAuth>,
        cancel: CancellationToken,
    ) -> Bs<BuildEvent> {
        self.note("build", credentials);
        self.inner.build_image(spec, credentials, cancel)
    }
    fn export_image(&self, id: &str, cancel: CancellationToken) -> Bs<bytes::Bytes> {
        self.inner.export_image(id, cancel)
    }
    async fn import_image(&self, tar: bytes::Bytes) -> Result<Vec<String>, EngineError> {
        self.inner.import_image(tar).await
    }
}

const FIXTURE_REGISTRY: &str = "127.0.0.1:56500";
/// Not a real password: only what the test hands the secret store.
const TEST_PASSWORD: &str = "pw-not-real";

thread_local! {
    static SLOT: std::cell::RefCell<Option<(AppState, ImageStore)>> =
        const { std::cell::RefCell::new(None) };
}

/// The App root's part: `AppState` and the registries service in context
/// (as `pull` finds them), one logged-in registry with a stored password.
#[component]
fn Harness() -> Element {
    let state = use_app_state();
    let store = ImageStore::new();
    use_hook(move || {
        let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::default());
        secrets
            .set(
                &secret_key(FIXTURE_REGISTRY),
                &secrecy::SecretString::from(TEST_PASSWORD.to_string()),
            )
            .expect("set");
        let registries = RegistriesService::new(
            Arc::new(berth_store::Store::open_in_memory().expect("store")),
            secrets,
        );
        let mut rows = state;
        rows.registries.set(vec![berth_core::Registry {
            id: FIXTURE_REGISTRY.to_string(),
            name: "berthfx".to_string(),
            url: url::Url::parse(&format!("http://{FIXTURE_REGISTRY}")).expect("url"),
            username: Some("berthfx".to_string()),
            status: berth_core::RegistryStatus::LoggedIn,
        }]);
        provide_context(state);
        provide_context(Some(registries));
        SLOT.with(|slot| *slot.borrow_mut() = Some((state, store)));
    });
    let shown = use_signal(|| true);
    use_hook(|| CALLER_SHOWN.with(|slot| *slot.borrow_mut() = Some(shown)));
    rsx! {
        div {}
        if shown() {
            Caller {}
        }
    }
}

thread_local! {
    static CALLER_SHOWN: std::cell::RefCell<Option<Signal<bool>>> =
        const { std::cell::RefCell::new(None) };
    static CALLER_SCOPE: std::cell::Cell<Option<ScopeId>> = const { std::cell::Cell::new(None) };
}

/// A child that starts work and is then removed, as the Build image
/// sheet's form page is the moment its build starts.
#[component]
fn Caller() -> Element {
    use_hook(|| CALLER_SCOPE.with(|slot| slot.set(Some(current_scope_id()))));
    rsx! { div {} }
}

struct Rig {
    dom: VirtualDom,
    state: AppState,
    store: ImageStore,
    services: Services,
    engine: Arc<Recorder>,
}

impl Rig {
    fn new() -> Self {
        let mut dom = VirtualDom::new(Harness);
        let _ = dom.rebuild_to_vec();
        let (state, store) = SLOT
            .with(|slot| slot.borrow_mut().take())
            .expect("harness captured the store");
        let engine = Arc::new(Recorder {
            inner: berth_engine::MockEngine::from_world(
                berth_engine::MockWorld::bundled(),
                &default_engine(),
            )
            .expect("engine"),
            seen: Mutex::new(Vec::new()),
        });
        let clients = crate::services::engines::client_map();
        if let Ok(mut map) = clients.lock() {
            map.insert(default_engine(), engine.clone() as Arc<dyn EngineClient>);
        }
        let services = Services {
            store: Arc::new(berth_store::Store::open_in_memory().expect("store")),
            clients,
            cancel: CancellationToken::new(),
        };
        Self {
            dom,
            state,
            store,
            services,
            engine,
        }
    }

    /// Run `f` inside the app scope, where `try_consume_context` finds the
    /// harness's contexts.
    fn scoped<R>(&self, f: impl FnOnce() -> R) -> R {
        self.dom.in_scope(ScopeId::APP, f)
    }

    fn read<R>(&self, f: impl FnOnce() -> R) -> R {
        self.dom.in_runtime(f)
    }

    /// Run `f` inside the `Caller` child, as a sheet's click handler runs.
    fn in_caller<R>(&self, f: impl FnOnce() -> R) -> R {
        let scope = CALLER_SCOPE
            .with(std::cell::Cell::get)
            .expect("caller mounted");
        self.dom.in_scope(scope, f)
    }

    /// Unmount the `Caller` child, as the sheet swaps its form page out.
    fn remove_caller(&mut self) {
        let mut shown = CALLER_SHOWN
            .with(|slot| *slot.borrow())
            .expect("harness shown signal");
        self.dom.in_runtime(|| shown.set(false));
        self.dom.render_immediate(&mut dioxus::core::NoOpMutations);
    }

    /// Drive the dom until `done` says so (5 s at most).
    async fn until(&mut self, what: &str, mut done: impl FnMut(&VirtualDom) -> bool) {
        let waited = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if done(&self.dom) {
                    return;
                }
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(50),
                    self.dom.wait_for_work(),
                )
                .await;
                self.dom.render_immediate(&mut dioxus::core::NoOpMutations);
            }
        })
        .await;
        assert!(waited.is_ok(), "{what} did not happen");
    }

    async fn build_phase_is_over(&mut self) {
        let store = self.store;
        self.until("the build to end", |dom| {
            dom.in_runtime(|| store.builds.peek().iter().all(|b| !b.is_running()))
        })
        .await;
    }

    fn pending(&self, subject: &str, action: &'static str) -> Option<PendingState> {
        let state = self.state;
        self.read(|| {
            state
                .pending
                .peek()
                .get(&PendingKey {
                    subject: Subject::Image(subject.to_string()),
                    action,
                })
                .cloned()
        })
    }

    fn activity(&self) -> Vec<berth_core::ActivityEntry> {
        self.services
            .store
            .activity_query(&berth_store::ActivityFilter::default(), None, 20)
            .expect("activity")
    }

    async fn image_ids(&self, tag: &str) -> Vec<String> {
        self.engine
            .images()
            .await
            .expect("images")
            .into_iter()
            .filter(|i| i.repo_tags.iter().any(|t| t == tag))
            .map(|i| i.id)
            .collect()
    }
}

const DOCKERFILE: &str = "FROM alpine:3\nRUN echo hi\n";

/// P8-HC: the Build image sheet starts a build from its form page and
/// unmounts that page at once; the build must still reach Done.
#[tokio::test]
async fn build_started_from_a_child_that_unmounts_still_reaches_done() {
    let mut rig = Rig::new();
    let store = rig.store;
    let services = rig.services.clone();
    let result = rig.in_caller(|| {
        store.build(
            &services,
            &default_engine(),
            inline(DOCKERFILE, &["berthfx/unmount:1"]),
        )
    });
    assert_eq!(result, Ok(()));
    rig.remove_caller();
    rig.build_phase_is_over().await;
    let done = rig.read(|| store.builds.peek()[0].clone());
    assert_eq!(done.phase, BuildPhase::Done);
    assert!(done
        .lines
        .iter()
        .any(|l| l.text.contains("Successfully built")));
    assert_eq!(rig.pending("berthfx/unmount:1", BUILD_ACTION), None);
}

#[tokio::test]
async fn build_store_runs_to_done_with_its_pending_entry_activity_and_the_new_image() {
    let mut rig = Rig::new();
    let store = rig.store;
    let result = rig.scoped(|| {
        store.build(
            &rig.services,
            &default_engine(),
            inline(DOCKERFILE, &["berthfx/app:1", "berthfx/app:dev"]),
        )
    });
    assert_eq!(result, Ok(()));
    // At once: a running card and a pending entry on the image.
    let started = rig.read(|| store.builds.peek().clone());
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].tags, ["berthfx/app:1", "berthfx/app:dev"]);
    assert_eq!(
        rig.pending("berthfx/app:1", BUILD_ACTION),
        Some(PendingState::Running {
            label: "Building berthfx/app:1".to_string()
        })
    );
    rig.build_phase_is_over().await;
    let done = rig.read(|| store.builds.peek()[0].clone());
    assert_eq!(done.phase, BuildPhase::Done);
    assert_eq!(done.step, Some((2, 2)));
    assert!(done
        .lines
        .iter()
        .any(|l| l.text.contains("Successfully built")));
    assert_eq!(rig.pending("berthfx/app:1", BUILD_ACTION), None);
    assert_eq!(rig.image_ids("berthfx/app:dev").await.len(), 1);
    // The list the screen reads is refreshed.
    rig.until("the refresh", |dom| {
        dom.in_runtime(|| {
            store
                .by_engine
                .peek()
                .get(&default_engine())
                .is_some_and(|images| {
                    images
                        .iter()
                        .any(|i| i.repo_tags.contains(&"berthfx/app:1".to_string()))
                })
        })
    })
    .await;
    let entries = rig.activity();
    assert!(
        entries.iter().any(|e| e.action == BUILD_ACTION
            && e.subject_name == "berthfx/app:1"
            && e.outcome == Outcome::Ok),
        "{entries:?}"
    );
}

#[tokio::test]
async fn build_store_a_failing_step_ends_failed_with_its_step_and_adds_nothing() {
    let mut rig = Rig::new();
    let store = rig.store;
    let before = rig.engine.images().await.expect("images").len();
    rig.scoped(|| {
        store
            .build(
                &rig.services,
                &default_engine(),
                inline(
                    "FROM alpine\nRUN sh -c 'exit 1'\nCMD true\n",
                    &["berthfx/broken:1"],
                ),
            )
            .expect("started");
    });
    rig.build_phase_is_over().await;
    let state = rig.read(|| store.builds.peek()[0].clone());
    assert!(matches!(&state.phase, BuildPhase::Failed(r) if r.contains("non-zero code")));
    assert_eq!(state.failed_step, Some(2));
    assert!(state.failure.is_some());
    assert_eq!(rig.engine.images().await.expect("images").len(), before);
    assert_eq!(rig.pending("berthfx/broken:1", BUILD_ACTION), None);
    let entries = rig.activity();
    assert!(
        entries.iter().any(|e| e.action == BUILD_ACTION
            && matches!(&e.outcome, Outcome::Failed { reason } if reason.contains("non-zero"))),
        "{entries:?}"
    );
}

#[tokio::test]
async fn build_store_refuses_a_bad_spec_and_an_unconnected_engine_and_starts_nothing() {
    let rig = Rig::new();
    let store = rig.store;
    let bad = rig.scoped(|| store.build(&rig.services, &default_engine(), inline(DOCKERFILE, &[])));
    assert!(bad.unwrap_err().contains("name"));
    let missing = rig.scoped(|| {
        store.build(
            &rig.services,
            &EngineId("nowhere".to_string()),
            inline(DOCKERFILE, &["a:1"]),
        )
    });
    assert!(missing.unwrap_err().contains("not connected"));
    assert!(rig.read(|| store.builds.peek().is_empty()));
    assert_eq!(rig.pending("a:1", BUILD_ACTION), None);
}

#[tokio::test]
async fn build_store_a_second_build_of_the_same_tag_does_not_start_another() {
    let mut rig = Rig::new();
    let store = rig.store;
    rig.scoped(|| {
        let spec = || inline(DOCKERFILE, &["berthfx/app:1"]);
        assert_eq!(
            store.build(&rig.services, &default_engine(), spec()),
            Ok(())
        );
        assert_eq!(
            store.build(&rig.services, &default_engine(), spec()),
            Ok(())
        );
    });
    assert_eq!(rig.read(|| store.builds.peek().len()), 1);
    rig.build_phase_is_over().await;
    assert_eq!(rig.engine.seen().len(), 1, "the engine was asked once");
}

#[tokio::test]
async fn build_store_cancel_ends_it_as_cancelled_with_no_image_and_no_activity() {
    let mut rig = Rig::new();
    let store = rig.store;
    let before = rig.engine.images().await.expect("images").len();
    rig.scoped(|| {
        store
            .build(
                &rig.services,
                &default_engine(),
                inline(DOCKERFILE, &["berthfx/never:1"]),
            )
            .expect("started");
        store.cancel_build(&(default_engine(), "berthfx/never:1".to_string()));
    });
    rig.build_phase_is_over().await;
    assert_eq!(
        rig.read(|| store.builds.peek()[0].phase.clone()),
        BuildPhase::Cancelled
    );
    assert_eq!(rig.engine.images().await.expect("images").len(), before);
    assert_eq!(rig.pending("berthfx/never:1", BUILD_ACTION), None);
    assert!(rig.activity().iter().all(|e| e.action != BUILD_ACTION));
}

#[tokio::test]
async fn build_store_dismiss_removes_a_finished_build_but_not_a_running_one() {
    let mut rig = Rig::new();
    let store = rig.store;
    let key = (default_engine(), "berthfx/app:1".to_string());
    rig.scoped(|| {
        store
            .build(
                &rig.services,
                &default_engine(),
                inline(DOCKERFILE, &["berthfx/app:1"]),
            )
            .expect("started");
        store.dismiss_build(&key);
    });
    assert_eq!(rig.read(|| store.builds.peek().len()), 1, "running stays");
    rig.build_phase_is_over().await;
    rig.scoped(|| store.dismiss_build(&key));
    assert!(rig.read(|| store.builds.peek().is_empty()));
}

#[tokio::test]
async fn build_store_hands_the_engine_the_credentials_of_the_tags_registry_only() {
    let mut rig = Rig::new();
    let store = rig.store;
    rig.scoped(|| {
        store
            .build(
                &rig.services,
                &default_engine(),
                inline(DOCKERFILE, &["127.0.0.1:56500/berthfx/app:1"]),
            )
            .expect("started");
    });
    rig.build_phase_is_over().await;
    rig.scoped(|| {
        store
            .build(
                &rig.services,
                &default_engine(),
                inline(DOCKERFILE, &["berthfx/plain:1"]),
            )
            .expect("started");
    });
    rig.until("the second build", |dom| {
        dom.in_runtime(|| store.builds.peek().iter().all(|b| !b.is_running()))
    })
    .await;
    assert_eq!(
        rig.engine.seen(),
        [
            (
                "build",
                Some((
                    "berthfx".to_string(),
                    TEST_PASSWORD.to_string(),
                    FIXTURE_REGISTRY.to_string()
                ))
            ),
            ("build", None)
        ]
    );
}

/// Tags the first listed image with `reference`, returning its id.
async fn tagged(rig: &Rig, repo: &str, tag: &str) -> String {
    let id = rig.engine.images().await.expect("images")[0].id.clone();
    rig.engine.tag_image(&id, repo, tag).await.expect("tag");
    id
}

#[tokio::test]
async fn push_store_runs_to_done_with_credentials_digest_pending_and_activity() {
    let mut rig = Rig::new();
    let store = rig.store;
    tagged(&rig, "127.0.0.1:56500/berthfx/app", "1").await;
    // No tag given: the engine's `latest` rule applies to the key too.
    tagged(&rig, "127.0.0.1:56500/berthfx/app", "latest").await;
    rig.scoped(|| {
        store
            .push(
                &rig.services,
                &default_engine(),
                "127.0.0.1:56500/berthfx/app",
            )
            .expect("started");
    });
    let key = (
        default_engine(),
        "127.0.0.1:56500/berthfx/app:latest".to_string(),
    );
    assert_eq!(rig.read(|| store.pushes.peek()[0].key()), key);
    assert_eq!(
        rig.pending(&key.1, PUSH_ACTION),
        Some(PendingState::Running {
            label: format!("Pushing {}", key.1)
        })
    );
    rig.until("the push", |dom| {
        dom.in_runtime(|| store.pushes.peek().iter().all(|p| !p.is_running()))
    })
    .await;
    let state = rig.read(|| store.pushes.peek()[0].clone());
    assert_eq!(state.phase, PushPhase::Done);
    assert!(state
        .digest
        .as_deref()
        .is_some_and(|d| d.starts_with("sha256:")));
    assert_eq!(rig.pending(&key.1, PUSH_ACTION), None);
    assert_eq!(
        rig.engine.seen(),
        [(
            "push",
            Some((
                "berthfx".to_string(),
                TEST_PASSWORD.to_string(),
                FIXTURE_REGISTRY.to_string()
            ))
        )]
    );
    assert!(
        rig.activity().iter().any(|e| e.action == PUSH_ACTION
            && e.subject_name == key.1
            && e.outcome == Outcome::Ok),
        "{:?}",
        rig.activity()
    );
}

/// P8-HC: a push started from a child that unmounts at once still ends.
#[tokio::test]
async fn push_started_from_a_child_that_unmounts_still_reaches_done() {
    let mut rig = Rig::new();
    let store = rig.store;
    tagged(&rig, "127.0.0.1:56500/berthfx/unmount", "1").await;
    let services = rig.services.clone();
    rig.in_caller(|| {
        store
            .push(
                &services,
                &default_engine(),
                "127.0.0.1:56500/berthfx/unmount:1",
            )
            .expect("started");
    });
    rig.remove_caller();
    rig.until("the push", |dom| {
        dom.in_runtime(|| store.pushes.peek().iter().all(|p| !p.is_running()))
    })
    .await;
    assert_eq!(
        rig.read(|| store.pushes.peek()[0].phase.clone()),
        PushPhase::Done
    );
    assert_eq!(
        rig.pending("127.0.0.1:56500/berthfx/unmount:1", PUSH_ACTION),
        None
    );
}

#[tokio::test]
async fn push_store_an_unknown_image_fails_with_the_engines_words() {
    let mut rig = Rig::new();
    let store = rig.store;
    rig.scoped(|| {
        store
            .push(&rig.services, &default_engine(), "berthfx/ghost:1")
            .expect("started");
    });
    rig.until("the push", |dom| {
        dom.in_runtime(|| store.pushes.peek().iter().all(|p| !p.is_running()))
    })
    .await;
    let phase = rig.read(|| store.pushes.peek()[0].phase.clone());
    assert!(
        matches!(&phase, PushPhase::Failed(r) if r.contains("does not exist")),
        "{phase:?}"
    );
    assert!(rig
        .activity()
        .iter()
        .any(|e| e.action == PUSH_ACTION && matches!(e.outcome, Outcome::Failed { .. })));
    assert_eq!(rig.pending("berthfx/ghost:1", PUSH_ACTION), None);
}

#[tokio::test]
async fn push_store_refuses_a_bad_reference_cancels_and_dismisses() {
    let mut rig = Rig::new();
    let store = rig.store;
    let bad = rig.scoped(|| store.push(&rig.services, &default_engine(), "not valid"));
    assert!(bad.unwrap_err().contains("no spaces"));
    tagged(&rig, "berthfx/p", "1").await;
    let key = (default_engine(), "berthfx/p:1".to_string());
    rig.scoped(|| {
        store
            .push(&rig.services, &default_engine(), "berthfx/p:1")
            .expect("started");
        store.dismiss_push(&key);
        store.cancel_push(&key);
    });
    assert_eq!(rig.read(|| store.pushes.peek().len()), 1, "running stays");
    rig.until("the end", |dom| {
        dom.in_runtime(|| store.pushes.peek().iter().all(|p| !p.is_running()))
    })
    .await;
    assert_eq!(
        rig.read(|| store.pushes.peek()[0].phase.clone()),
        PushPhase::Cancelled
    );
    assert!(rig.activity().iter().all(|e| e.action != PUSH_ACTION));
    rig.scoped(|| store.dismiss_push(&key));
    assert!(rig.read(|| store.pushes.peek().is_empty()));
}

/// Wait for a callback-style result.
async fn result_of<T: Send + 'static>(rig: &mut Rig, start: impl FnOnce(Box<dyn FnOnce(T)>)) -> T {
    let slot: Rc<RefCell<Option<T>>> = Rc::new(RefCell::new(None));
    let sink = slot.clone();
    rig.scoped(|| start(Box::new(move |value| *sink.borrow_mut() = Some(value))));
    rig.until("the result", |_| slot.borrow().is_some()).await;
    let value = slot.borrow_mut().take();
    value.expect("a result")
}

use std::cell::RefCell;
use std::rc::Rc;

#[tokio::test]
async fn export_then_import_round_trips_an_image_through_a_tar_file() {
    let mut rig = Rig::new();
    let store = rig.store;
    rig.scoped(|| {
        store
            .build(
                &rig.services,
                &default_engine(),
                inline(DOCKERFILE, &["berthfx/app:1"]),
            )
            .expect("started");
    });
    rig.build_phase_is_over().await;
    let id = rig.image_ids("berthfx/app:1").await.remove(0);
    let dir = tempfile::tempdir().expect("dir");
    let tar = dir.path().join("app.tar");

    let (services, target) = (rig.services.clone(), tar.clone());
    let size = result_of(&mut rig, |done| {
        store.export(&services, &default_engine(), "berthfx/app:1", target, done);
    })
    .await
    .expect("exported");
    assert!(size > 0);
    assert_eq!(std::fs::metadata(&tar).expect("file").len(), size);
    assert!(!dir.path().join("app.tar.part").exists(), "no partial left");
    assert_eq!(rig.pending("berthfx/app:1", EXPORT_ACTION), None);

    rig.engine
        .remove_image(&id, true)
        .await
        .expect("remove the local copy");
    assert!(rig.image_ids("berthfx/app:1").await.is_empty());
    // The screen's list learns it is gone, so the import has to bring it back.
    rig.scoped(|| store.refresh(&rig.services));
    rig.until("the list to lose the image", |dom| {
        dom.in_runtime(|| {
            store
                .by_engine
                .peek()
                .get(&default_engine())
                .is_some_and(|images| {
                    images
                        .iter()
                        .all(|i| !i.repo_tags.contains(&"berthfx/app:1".to_string()))
                })
        })
    })
    .await;

    let (services, source) = (rig.services.clone(), tar.clone());
    let names = result_of(&mut rig, |done| {
        store.import(&services, &default_engine(), source, done);
    })
    .await
    .expect("imported");
    assert_eq!(names, ["berthfx/app:1"]);
    assert_eq!(rig.image_ids("berthfx/app:1").await, [id]);
    assert_eq!(rig.pending("app.tar", IMPORT_ACTION), None);
    let entries = rig.activity();
    assert!(entries
        .iter()
        .any(|e| e.action == EXPORT_ACTION && e.outcome == Outcome::Ok));
    assert!(entries.iter().any(|e| e.action == IMPORT_ACTION
        && e.subject_name == "berthfx/app:1"
        && e.outcome == Outcome::Ok));
    // The screen's list shows the imported image.
    rig.until("the refresh", |dom| {
        dom.in_runtime(|| {
            store
                .by_engine
                .peek()
                .get(&default_engine())
                .is_some_and(|images| {
                    images
                        .iter()
                        .any(|i| i.repo_tags.contains(&"berthfx/app:1".to_string()))
                })
        })
    })
    .await;
}

#[tokio::test]
async fn export_of_an_unknown_image_fails_and_leaves_no_file() {
    let mut rig = Rig::new();
    let store = rig.store;
    let dir = tempfile::tempdir().expect("dir");
    let tar = dir.path().join("ghost.tar");
    let (services, target) = (rig.services.clone(), tar.clone());
    let result = result_of(&mut rig, |done| {
        store.export(
            &services,
            &default_engine(),
            "berthfx/ghost:1",
            target,
            done,
        );
    })
    .await;
    assert!(result.unwrap_err().contains("ghost"));
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);
    assert!(rig
        .activity()
        .iter()
        .any(|e| e.action == EXPORT_ACTION && matches!(e.outcome, Outcome::Failed { .. })));
}

#[tokio::test]
async fn export_cancelled_returns_cancelled_leaves_no_file_and_records_nothing() {
    let mut rig = Rig::new();
    let store = rig.store;
    let tag_id = tagged(&rig, "berthfx/c", "1").await;
    assert!(!tag_id.is_empty());
    let dir = tempfile::tempdir().expect("dir");
    let tar = dir.path().join("c.tar");
    let (services, target) = (rig.services.clone(), tar.clone());
    let result = result_of(&mut rig, |done| {
        store.export(&services, &default_engine(), "berthfx/c:1", target, done);
        // Cancelled before the stream's first poll.
        store.cancel_transfer(&default_engine(), EXPORT_ACTION, "berthfx/c:1");
    })
    .await;
    assert_eq!(result, Err("Cancelled".to_string()));
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);
    assert!(rig.activity().iter().all(|e| e.action != EXPORT_ACTION));
}

#[tokio::test]
async fn import_of_a_file_that_is_no_image_tar_fails_with_a_reason() {
    let mut rig = Rig::new();
    let store = rig.store;
    let before = rig.engine.images().await.expect("images").len();
    let dir = tempfile::tempdir().expect("dir");
    let junk = dir.path().join("junk.tar");
    std::fs::write(&junk, b"not a tar").expect("write");
    let (services, source) = (rig.services.clone(), junk);
    let result = result_of(&mut rig, |done| {
        store.import(&services, &default_engine(), source, done);
    })
    .await;
    assert!(result.is_err());
    assert_eq!(rig.engine.images().await.expect("images").len(), before);
    assert!(rig.activity().iter().any(|e| e.action == IMPORT_ACTION
        && e.subject_name == "junk.tar"
        && matches!(e.outcome, Outcome::Failed { .. })));

    let (services, gone) = (rig.services.clone(), dir.path().join("missing.tar"));
    let missing = result_of(&mut rig, |done| {
        store.import(&services, &default_engine(), gone, done);
    })
    .await;
    assert!(missing.unwrap_err().contains("Cannot read"));
}
