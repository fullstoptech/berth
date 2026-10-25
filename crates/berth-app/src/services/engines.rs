//! Engines service (D23): discovery, one client per running engine,
//! snapshot-first startup, initial lists, and one events loop per engine
//! that refetches the single container an event names. A full refresh
//! happens only on (re)connect: when an events stream ends or errors, the
//! loop backs off, refetches everything, and resubscribes.
//!
//! Threading: engine I/O runs in tokio tasks that never touch a signal.
//! They send [`EngineUpdate`]s over a channel; a Dioxus-spawned receiver
//! applies them with [`Applier`] on the UI thread. Tests drive the same
//! channel and applier by hand.

use std::sync::Arc;

use dioxus::prelude::*;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::services::activity;
use crate::state::{AppState, EngineScope};

/// Live clients by engine, filled by discovery/connect. Shared through
/// the `Services` context so later services act on the same clients.
pub type ClientMap = Arc<
    std::sync::Mutex<
        std::collections::BTreeMap<berth_core::EngineId, Arc<dyn berth_engine::EngineClient>>,
    >,
>;

pub fn client_map() -> ClientMap {
    Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new()))
}

pub const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
pub const SNAPSHOT_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

/// One fact from an engine task, applied to the signals in order.
#[derive(Debug)]
pub enum EngineUpdate {
    /// Discovery finished; the engine list is known before any answer.
    EnginesDiscovered(Vec<berth_core::Engine>),
    /// Snapshot rows loaded from the store, before the first engine answer.
    Snapshots(Vec<berth_store::EngineSnapshot>),
    /// A full (re)connect refresh of one engine.
    EngineData {
        engine: berth_core::EngineId,
        info: Option<berth_core::EngineInfo>,
        containers: Vec<berth_core::Container>,
        projects: Vec<berth_core::Project>,
        images: Vec<berth_core::Image>,
        volumes: Vec<berth_core::Volume>,
        networks: Vec<berth_core::Network>,
    },
    /// One container changed (an event named it); `None` means it is gone.
    ContainerChanged {
        engine: berth_core::EngineId,
        id: berth_core::ContainerId,
        container: Option<berth_core::Container>,
    },
    ImagesChanged {
        engine: berth_core::EngineId,
        images: Vec<berth_core::Image>,
    },
    VolumesChanged {
        engine: berth_core::EngineId,
        volumes: Vec<berth_core::Volume>,
    },
    NetworksChanged {
        engine: berth_core::EngineId,
        networks: Vec<berth_core::Network>,
    },
    /// The events stream ended; a reconnect (full refresh) follows.
    Disconnected { engine: berth_core::EngineId },
    /// A refresh failed; the engine is unreachable for now.
    EngineFailed {
        engine: berth_core::EngineId,
        reason: String,
    },
    /// Colima started a profile (possibly with new resources): what it
    /// reports now (P4-T6).
    ProfileChanged {
        engine: berth_core::EngineId,
        profile: berth_core::ColimaProfile,
    },
    /// The set of pinned folders changed (a folder was opened or generated): every engine's
    /// projects are grouped again with these pins. Sent by [`EngineLink::pins_changed`].
    PinsChanged(Vec<berth_core::PinnedFolder>),
}

fn is_remote(kind: &berth_core::EngineKind) -> bool {
    matches!(kind, berth_core::EngineKind::Remote { .. })
}

async fn full_refresh(
    client: &dyn berth_engine::EngineClient,
    pins: &[berth_core::PinnedFolder],
    remote: bool,
) -> Result<EngineUpdate, berth_engine::EngineError> {
    let containers = client.containers().await?;
    let projects = berth_engine::projects::group(client.id(), &containers, pins, remote);
    let images = client.images().await.unwrap_or_else(|err| {
        tracing::warn!(%err, engine = %client.id().0, "images list failed");
        Vec::new()
    });
    let mut volumes = client.volumes().await.unwrap_or_else(|err| {
        tracing::warn!(%err, engine = %client.id().0, "volumes list failed");
        Vec::new()
    });
    if let Ok(sizes) = client.volume_sizes().await {
        for volume in &mut volumes {
            if volume.size.is_none() {
                volume.size = sizes.get(&volume.name).copied();
            }
        }
    }
    let networks = client.networks().await.unwrap_or_else(|err| {
        tracing::warn!(%err, engine = %client.id().0, "networks list failed");
        Vec::new()
    });
    let info = client.info().await.ok();
    Ok(EngineUpdate::EngineData {
        engine: client.id().clone(),
        info,
        containers,
        projects,
        images,
        volumes,
        networks,
    })
}

async fn sleep_or_cancel(cancel: &CancellationToken, delay: std::time::Duration) {
    tokio::select! {
        () = cancel.cancelled() => {}
        () = tokio::time::sleep(delay) => {}
    }
}

/// The per-engine loop: subscribe to events first (so nothing changes
/// unseen between the refresh and the subscription), full refresh, then
/// events until the stream ends, then reconnect from the top. Runs until
/// `cancel` fires.
pub async fn run_engine(
    client: Arc<dyn berth_engine::EngineClient>,
    store: Arc<berth_store::Store>,
    pins: Vec<berth_core::PinnedFolder>,
    remote: bool,
    tx: tokio::sync::mpsc::Sender<EngineUpdate>,
    cancel: CancellationToken,
    reconnect_delay: std::time::Duration,
) {
    let engine = client.id().clone();
    'reconnect: loop {
        if cancel.is_cancelled() {
            return;
        }
        let stream_cancel = cancel.child_token();
        let mut events = client.events(stream_cancel.clone());
        match full_refresh(client.as_ref(), &pins, remote).await {
            Ok(update) => {
                if tx.send(update).await.is_err() {
                    return;
                }
            }
            Err(err) => {
                stream_cancel.cancel();
                tracing::warn!(%err, engine = %engine.0, "engine refresh failed");
                if tx
                    .send(EngineUpdate::EngineFailed {
                        engine: engine.clone(),
                        reason: err.to_string(),
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                sleep_or_cancel(&cancel, reconnect_delay).await;
                continue 'reconnect;
            }
        }

        while let Some(item) = events.next().await {
            if cancel.is_cancelled() {
                return;
            }
            let event = match item {
                Ok(event) => event,
                Err(err) => {
                    tracing::warn!(%err, engine = %engine.0, "engine events stream failed");
                    break;
                }
            };
            let subject_name = event
                .attributes
                .get("name")
                .cloned()
                .unwrap_or_else(|| subject_name_for(&event.kind));
            if let Some(entry) = activity::entry_for_event(&event, subject_name, None) {
                activity::record(&store, &entry);
            }
            let update = match &event.kind {
                berth_core::EventKind::Container { id, action } => {
                    container_update(client.as_ref(), id, action).await
                }
                berth_core::EventKind::Image { .. } => match client.images().await {
                    Ok(images) => Some(EngineUpdate::ImagesChanged {
                        engine: engine.clone(),
                        images,
                    }),
                    Err(err) => {
                        tracing::warn!(%err, engine = %engine.0, "images refetch failed");
                        None
                    }
                },
                berth_core::EventKind::Volume { .. } => match client.volumes().await {
                    Ok(volumes) => Some(EngineUpdate::VolumesChanged {
                        engine: engine.clone(),
                        volumes,
                    }),
                    Err(err) => {
                        tracing::warn!(%err, engine = %engine.0, "volumes refetch failed");
                        None
                    }
                },
                berth_core::EventKind::Network { .. } => match client.networks().await {
                    Ok(networks) => Some(EngineUpdate::NetworksChanged {
                        engine: engine.clone(),
                        networks,
                    }),
                    Err(err) => {
                        tracing::warn!(%err, engine = %engine.0, "networks refetch failed");
                        None
                    }
                },
                berth_core::EventKind::Other { .. } => None,
            };
            if let Some(update) = update {
                if tx.send(update).await.is_err() {
                    return;
                }
            }
        }

        stream_cancel.cancel();
        if cancel.is_cancelled() {
            return;
        }
        // Events between now and the reconnect are lost: mark the gap (P2-T10).
        activity::record(
            &store,
            &activity::entry_for_gap(engine.clone(), chrono::Utc::now()),
        );
        if tx
            .send(EngineUpdate::Disconnected {
                engine: engine.clone(),
            })
            .await
            .is_err()
        {
            return;
        }
        sleep_or_cancel(&cancel, reconnect_delay).await;
    }
}

fn subject_name_for(kind: &berth_core::EventKind) -> String {
    match kind {
        berth_core::EventKind::Container { id, .. } => id.short().to_string(),
        berth_core::EventKind::Image { id, .. } => id.clone(),
        berth_core::EventKind::Volume { name, .. } => name.clone(),
        berth_core::EventKind::Network { id, .. } => id.clone(),
        berth_core::EventKind::Other { ty, .. } => ty.clone(),
    }
}

/// Refetch the one container an event names. `destroy` needs no refetch;
/// a refetch that 404s also reads as "gone". A transient error yields no
/// update (the row stays; the next event or reconnect corrects it).
async fn container_update(
    client: &dyn berth_engine::EngineClient,
    id: &berth_core::ContainerId,
    action: &str,
) -> Option<EngineUpdate> {
    let engine = client.id().clone();
    if action == "destroy" {
        return Some(EngineUpdate::ContainerChanged {
            engine,
            id: id.clone(),
            container: None,
        });
    }
    match client.container(id).await {
        Ok(detail) => Some(EngineUpdate::ContainerChanged {
            engine,
            id: detail.container.id.clone(),
            container: Some(detail.container),
        }),
        Err(berth_engine::EngineError::NotFound(_)) => Some(EngineUpdate::ContainerChanged {
            engine,
            id: id.clone(),
            container: None,
        }),
        Err(err) => {
            tracing::warn!(%err, container = %id.0, "container refetch failed");
            None
        }
    }
}

fn spawn_io(future: impl std::future::Future<Output = ()> + Send + 'static) {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(future);
    } else {
        // No tokio runtime on this thread (not expected on desktop):
        // fall back to the Dioxus executor so the future still runs.
        spawn(future);
    }
}

/// Snapshot-first startup over already-connected clients: engines, then
/// snapshot rows, then the per-engine loops (so the first engine answer
/// can only land after snapshot data is in the signals).
pub async fn run_with_clients(
    engines: Vec<berth_core::Engine>,
    clients: Vec<Arc<dyn berth_engine::EngineClient>>,
    store: Arc<berth_store::Store>,
    pins: Vec<berth_core::PinnedFolder>,
    tx: tokio::sync::mpsc::Sender<EngineUpdate>,
    cancel: CancellationToken,
    reconnect_delay: std::time::Duration,
) {
    let remote_by_engine: std::collections::BTreeMap<berth_core::EngineId, bool> = engines
        .iter()
        .map(|e| (e.id.clone(), is_remote(&e.kind)))
        .collect();
    if tx
        .send(EngineUpdate::EnginesDiscovered(engines.clone()))
        .await
        .is_err()
    {
        return;
    }
    let mut snapshots = Vec::new();
    for engine in &engines {
        match store.snapshot(&engine.id) {
            Ok(Some(snapshot)) => snapshots.push(snapshot),
            Ok(None) => {}
            Err(err) => tracing::warn!(%err, engine = %engine.id.0, "snapshot load failed"),
        }
    }
    if tx.send(EngineUpdate::Snapshots(snapshots)).await.is_err() {
        return;
    }
    for client in clients {
        let remote = remote_by_engine.get(client.id()).copied().unwrap_or(false);
        spawn_io(run_engine(
            client,
            Arc::clone(&store),
            pins.clone(),
            remote,
            tx.clone(),
            cancel.clone(),
            reconnect_delay,
        ));
    }
    cancel.cancelled().await;
}

/// Real discovery: Colima profiles + docker contexts + user engines from
/// the store (`berth_engine::discovery` is the only socket lookup, D29),
/// one `BollardEngine` per running engine.
async fn discover_and_connect(
    store: &berth_store::Store,
    clients: &ClientMap,
) -> Vec<berth_core::Engine> {
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima = berth_engine::ColimaCli::new(search);
    let user: Vec<berth_engine::discovery::UserEngineRef> = match store.user_engines() {
        Ok(rows) => rows
            .into_iter()
            .map(|row| berth_engine::discovery::UserEngineRef {
                id: row.id,
                name: row.name,
                kind: row.kind,
            })
            .collect(),
        Err(err) => {
            tracing::warn!(%err, "user engines load failed");
            Vec::new()
        }
    };
    let engines = berth_engine::discovery::discover(&colima, &user).await;
    let mut map = match clients.lock() {
        Ok(map) => map,
        Err(poisoned) => poisoned.into_inner(),
    };
    for engine in &engines {
        if engine.state != berth_core::EngineState::Running {
            continue;
        }
        match berth_engine::BollardEngine::connect(engine) {
            Ok(client) => {
                map.insert(engine.id.clone(), Arc::new(client));
            }
            Err(err) => {
                tracing::warn!(%err, engine = %engine.id.0, "engine connect failed");
            }
        }
    }
    engines
}

/// Mocks for `BERTH_MOCK=1`: the bundled world, one `MockEngine` per
/// running mock engine, the world's pins.
fn mock_world_parts(
    clients: &ClientMap,
) -> (Vec<berth_core::Engine>, Vec<berth_core::PinnedFolder>) {
    let world = super::mock_world_at(chrono::Utc::now());
    let mut map = match clients.lock() {
        Ok(map) => map,
        Err(poisoned) => poisoned.into_inner(),
    };
    for engine in &world.engines {
        if engine.state != berth_core::EngineState::Running {
            continue;
        }
        match berth_engine::MockEngine::from_world(world.clone(), &engine.id) {
            Ok(client) => {
                map.insert(engine.id.clone(), Arc::new(client));
            }
            Err(err) => tracing::warn!(%err, engine = %engine.id.0, "mock engine failed"),
        }
    }
    (world.engines.clone(), world.pins.clone())
}

fn connected_clients(clients: &ClientMap) -> Vec<Arc<dyn berth_engine::EngineClient>> {
    match clients.lock() {
        Ok(map) => map.values().cloned().collect(),
        Err(poisoned) => poisoned.into_inner().values().cloned().collect(),
    }
}

fn store_pins(store: &berth_store::Store) -> Vec<berth_core::PinnedFolder> {
    match store.pins() {
        Ok(pins) => pins,
        Err(err) => {
            tracing::warn!(%err, "pins load failed");
            Vec::new()
        }
    }
}

/// Owns the per-engine resource slices, so one engine's update replaces
/// only its own rows in the flat signals, and applies updates to the
/// signals. One applier lives in the receiver task.
#[derive(Default)]
pub struct Applier {
    pins: Vec<berth_core::PinnedFolder>,
    /// A [`EngineUpdate::PinsChanged`] has arrived: the pins here are newer than the ones the
    /// engine loops hold, so a full refresh regroups with these.
    pins_updated: bool,
    images: std::collections::BTreeMap<berth_core::EngineId, Vec<berth_core::Image>>,
    volumes: std::collections::BTreeMap<berth_core::EngineId, Vec<berth_core::Volume>>,
    networks: std::collections::BTreeMap<berth_core::EngineId, Vec<berth_core::Network>>,
}

impl Applier {
    /// Group `engine`'s containers into projects with the pins known now and put them in the
    /// signal in place of that engine's old projects. Returns the engine's containers.
    fn regroup(
        &self,
        state: &mut AppState,
        engine: &berth_core::EngineId,
    ) -> Vec<berth_core::Container> {
        let mine: Vec<berth_core::Container> = state
            .containers
            .peek()
            .iter()
            .filter(|c| c.engine == *engine)
            .cloned()
            .collect();
        let remote = state
            .engines
            .peek()
            .iter()
            .find(|e| e.id == *engine)
            .is_some_and(|e| is_remote(&e.kind));
        let projects = berth_engine::projects::group(engine, &mine, &self.pins, remote);
        replace_engine_slice(&mut state.projects, engine, projects);
        mine
    }

    pub fn new(pins: Vec<berth_core::PinnedFolder>) -> Self {
        Self {
            pins,
            pins_updated: false,
            images: std::collections::BTreeMap::new(),
            volumes: std::collections::BTreeMap::new(),
            networks: std::collections::BTreeMap::new(),
        }
    }

    pub fn apply(&mut self, state: &mut AppState, update: EngineUpdate) {
        match update {
            EngineUpdate::EnginesDiscovered(engines) => {
                // D81: default scope is the first running Colima profile.
                let scope = engines
                    .iter()
                    .find(|e| {
                        e.state == berth_core::EngineState::Running
                            && matches!(e.kind, berth_core::EngineKind::Colima { .. })
                    })
                    .map(|e| EngineScope::One(e.id.clone()))
                    .unwrap_or(EngineScope::All);
                state.engines.set(engines);
                state.engine_scope.set(scope);
            }
            EngineUpdate::Snapshots(snapshots) => {
                if snapshots.is_empty() {
                    return;
                }
                let mut containers = Vec::new();
                let mut projects = Vec::new();
                for snapshot in snapshots {
                    containers.extend(snapshot.containers);
                    projects.extend(snapshot.projects);
                }
                state.containers.set(containers);
                state.projects.set(projects);
                state.snapshot_stale.set(true);
            }
            EngineUpdate::EngineData {
                engine,
                info,
                containers,
                projects,
                images,
                volumes,
                networks,
            } => {
                let counts = berth_core::ResourceCounts {
                    containers_running: u32::try_from(
                        containers
                            .iter()
                            .filter(|c| c.state == berth_core::ContainerState::Running)
                            .count(),
                    )
                    .unwrap_or(u32::MAX),
                    containers_total: u32::try_from(containers.len()).unwrap_or(u32::MAX),
                    images: u32::try_from(images.len()).unwrap_or(u32::MAX),
                    volumes: u32::try_from(volumes.len()).unwrap_or(u32::MAX),
                    networks: u32::try_from(networks.len()).unwrap_or(u32::MAX),
                };
                self.images.insert(engine.clone(), images);
                self.volumes.insert(engine.clone(), volumes);
                self.networks.insert(engine.clone(), networks);
                replace_engine_slice(&mut state.containers, &engine, containers);
                if self.pins_updated {
                    // The loop that built `projects` may hold pins from before one was added.
                    self.regroup(state, &engine);
                } else {
                    replace_engine_slice(&mut state.projects, &engine, projects);
                }
                self.write_flat_lists(state);
                let mut engines = state.engines.write();
                if let Some(found) = engines.iter_mut().find(|e| e.id == engine) {
                    found.state = berth_core::EngineState::Running;
                    found.counts = counts;
                    if let Some(info) = &info {
                        found.docker_version = Some(info.docker_version.clone());
                    }
                }
                drop(engines);
                clear_retry(state, &engine);
                state.snapshot_stale.set(false);
            }
            EngineUpdate::ContainerChanged {
                engine,
                id,
                container,
            } => {
                {
                    let mut containers = state.containers.write();
                    containers.retain(|c| !(c.engine == engine && c.id == id));
                    if let Some(container) = container {
                        containers.push(container);
                    }
                }
                let mine = self.regroup(state, &engine);
                let running = mine
                    .iter()
                    .filter(|c| c.state == berth_core::ContainerState::Running)
                    .count();
                let mut engines = state.engines.write();
                if let Some(found) = engines.iter_mut().find(|e| e.id == engine) {
                    found.counts.containers_running = u32::try_from(running).unwrap_or(u32::MAX);
                    found.counts.containers_total = u32::try_from(mine.len()).unwrap_or(u32::MAX);
                }
            }
            EngineUpdate::PinsChanged(pins) => {
                self.pins = pins;
                self.pins_updated = true;
                let engines: Vec<berth_core::EngineId> =
                    state.engines.peek().iter().map(|e| e.id.clone()).collect();
                for engine in engines {
                    self.regroup(state, &engine);
                }
            }
            EngineUpdate::ImagesChanged { engine, images } => {
                self.images.insert(engine, images);
                self.write_flat_lists(state);
            }
            EngineUpdate::VolumesChanged { engine, volumes } => {
                self.volumes.insert(engine, volumes);
                self.write_flat_lists(state);
            }
            EngineUpdate::NetworksChanged { engine, networks } => {
                self.networks.insert(engine, networks);
                self.write_flat_lists(state);
            }
            EngineUpdate::Disconnected { .. } => {
                state.snapshot_stale.set(true);
            }
            EngineUpdate::EngineFailed { engine, reason } => {
                let mut engines = state.engines.write();
                if let Some(found) = engines.iter_mut().find(|e| e.id == engine) {
                    use berth_core::EngineState as S;
                    // A profile Colima stopped (or is starting) answers
                    // nothing, and that is not a failure to report.
                    let last_seen = match &found.state {
                        S::Stopped | S::Stopping | S::Starting => None,
                        S::Running => Some(chrono::Utc::now()),
                        S::Unreachable { last_seen, .. } => *last_seen,
                        S::Unsupported { .. } => None,
                    };
                    if matches!(found.state, S::Running | S::Unreachable { .. }) {
                        found.state = S::Unreachable { reason, last_seen };
                    }
                }
                drop(engines);
                clear_retry(state, &engine);
                state.snapshot_stale.set(true);
            }
            EngineUpdate::ProfileChanged { engine, profile } => {
                let mut engines = state.engines.write();
                if let Some(found) = engines.iter_mut().find(|e| e.id == engine) {
                    found.resources = Some(resources_of(&profile));
                    if let Some(socket) = &profile.docker_socket {
                        found.docker_host = Some(format!("unix://{}", socket.display()));
                    }
                }
            }
        }
    }

    fn write_flat_lists(&self, state: &mut AppState) {
        state.images.set(
            self.images
                .values()
                .flat_map(|rows| rows.iter().cloned())
                .collect(),
        );
        state.volumes.set(
            self.volumes
                .values()
                .flat_map(|rows| rows.iter().cloned())
                .collect(),
        );
        state.networks.set(
            self.networks
                .values()
                .flat_map(|rows| rows.iter().cloned())
                .collect(),
        );
    }
}

/// The engine answered (or failed again): its "Retrying…" is over.
fn clear_retry(state: &mut AppState, engine: &berth_core::EngineId) {
    let key = crate::state::PendingKey {
        subject: berth_core::Subject::Engine(engine.clone()),
        action: "engine.retry",
    };
    if state.pending.peek().contains_key(&key) {
        state.pending.write().remove(&key);
    }
}

/// The VM sizes a Colima profile reports, as an engine shows them.
fn resources_of(profile: &berth_core::ColimaProfile) -> berth_core::VmResources {
    berth_core::VmResources {
        cpus: profile.cpus,
        memory: profile.memory,
        disk: profile.disk,
        arch: profile.arch.clone(),
        vm_type: profile.vm_type.clone().unwrap_or(berth_core::VmType::Vz),
        rosetta: profile.rosetta.unwrap_or(false),
        runtime: profile.runtime,
        mount_type: profile.mount_type.clone(),
    }
}

fn replace_engine_slice<T>(
    signal: &mut Signal<Vec<T>>,
    engine: &berth_core::EngineId,
    fresh: Vec<T>,
) where
    T: HasEngine + 'static,
{
    let mut rows = signal.write();
    rows.retain(|row| row.engine() != engine);
    rows.extend(fresh);
}

trait HasEngine {
    fn engine(&self) -> &berth_core::EngineId;
}
impl HasEngine for berth_core::Container {
    fn engine(&self) -> &berth_core::EngineId {
        &self.engine
    }
}
impl HasEngine for berth_core::Project {
    fn engine(&self) -> &berth_core::EngineId {
        &self.engine
    }
}

/// Load every stored snapshot straight into the signals (D23 cold start).
/// Returns how many engines had a snapshot. The channel path uses
/// `EngineUpdate::Snapshots`; this is the same rule for direct callers.
pub fn load_snapshots(
    state: &mut AppState,
    store: &berth_store::Store,
    engines: &[berth_core::Engine],
) -> usize {
    let mut applier = Applier::default();
    let mut snapshots = Vec::new();
    for engine in engines {
        match store.snapshot(&engine.id) {
            Ok(Some(snapshot)) => snapshots.push(snapshot),
            Ok(None) => {}
            Err(err) => tracing::warn!(%err, engine = %engine.id.0, "snapshot load failed"),
        }
    }
    let count = snapshots.len();
    applier.apply(state, EngineUpdate::Snapshots(snapshots));
    count
}

/// Persist one snapshot per engine from the current signals.
/// Returns how many snapshots were saved.
pub fn save_snapshots(state: &AppState, store: &berth_store::Store) -> usize {
    let engines = match state.engines.try_peek() {
        Ok(engines) => engines.clone(),
        Err(err) => {
            tracing::warn!(%err, "snapshot save: engines borrowed");
            return 0;
        }
    };
    let containers = match state.containers.try_peek() {
        Ok(rows) => rows.clone(),
        Err(_) => return 0,
    };
    let projects = match state.projects.try_peek() {
        Ok(rows) => rows.clone(),
        Err(_) => return 0,
    };
    let mut saved = 0;
    for engine in &engines {
        let snapshot = berth_store::EngineSnapshot {
            engine: engine.id.clone(),
            taken: chrono::Utc::now(),
            containers: containers
                .iter()
                .filter(|c| c.engine == engine.id)
                .cloned()
                .collect(),
            projects: projects
                .iter()
                .filter(|p| p.engine == engine.id)
                .cloned()
                .collect(),
        };
        match store.save_snapshot(&snapshot) {
            Ok(()) => saved += 1,
            Err(err) => tracing::warn!(%err, engine = %engine.id.0, "snapshot save failed"),
        }
    }
    saved
}

/// Keeps the service alive with the App root; dropping it cancels every
/// task and makes a best-effort final snapshot save (quit path, D23).
pub struct EnginesHandle {
    cancel: CancellationToken,
    store: Arc<berth_store::Store>,
    state: AppState,
    link: EngineLink,
}

impl EnginesHandle {
    /// The link other services use to bring an engine back online.
    #[must_use]
    pub fn link(&self) -> EngineLink {
        self.link.clone()
    }
}

/// Brings one engine online after discovery has run (P4-T6): a Colima
/// profile that was stopped at launch and has just been started, or an
/// unreachable engine the user pressed Retry on. An engine that already has
/// a client keeps its loop, which reconnects by itself; one without gets a
/// client and a loop here. Cheap to clone.
#[derive(Clone)]
pub struct EngineLink {
    inner: Arc<LinkInner>,
}

struct LinkInner {
    clients: ClientMap,
    store: Arc<berth_store::Store>,
    pins: Vec<berth_core::PinnedFolder>,
    tx: tokio::sync::mpsc::Sender<EngineUpdate>,
    cancel: CancellationToken,
    mock: bool,
}

impl EngineLink {
    #[must_use]
    pub fn new(
        clients: ClientMap,
        store: Arc<berth_store::Store>,
        pins: Vec<berth_core::PinnedFolder>,
        tx: tokio::sync::mpsc::Sender<EngineUpdate>,
        cancel: CancellationToken,
        mock: bool,
    ) -> Self {
        Self {
            inner: Arc::new(LinkInner {
                clients,
                store,
                pins,
                tx,
                cancel,
                mock,
            }),
        }
    }

    /// Connect `engine` unless it has a client. `profile` is what Colima
    /// reports for it now (its socket exists only once it runs); it is also
    /// sent on so the sizes on screen follow an Apply.
    pub fn connect(&self, engine: berth_core::Engine, profile: Option<berth_core::ColimaProfile>) {
        let inner = Arc::clone(&self.inner);
        spawn_io(async move { inner.connect(engine, profile).await });
    }

    /// Tell the engines service the set of pinned folders changed (Open folder, a generated
    /// project): it groups every engine's projects again, so the folder shows up without a
    /// restart. Call after `Store::pin`; the pins are read from the store here.
    pub fn pins_changed(&self) {
        let inner = Arc::clone(&self.inner);
        spawn_io(async move {
            let pins = inner.current_pins();
            let _ = inner.tx.send(EngineUpdate::PinsChanged(pins)).await;
        });
    }
}

impl LinkInner {
    /// The pins now: the store's, plus the mock world's under `BERTH_MOCK=1` (they are not in the
    /// store).
    fn current_pins(&self) -> Vec<berth_core::PinnedFolder> {
        let mut pins = if self.mock {
            self.pins.clone()
        } else {
            Vec::new()
        };
        for pin in store_pins(&self.store) {
            if !pins.iter().any(|p| p.folder == pin.folder) {
                pins.push(pin);
            }
        }
        pins
    }

    fn has_client(&self, id: &berth_core::EngineId) -> bool {
        match self.clients.lock() {
            Ok(map) => map.contains_key(id),
            Err(poisoned) => poisoned.into_inner().contains_key(id),
        }
    }

    async fn connect(
        &self,
        mut engine: berth_core::Engine,
        profile: Option<berth_core::ColimaProfile>,
    ) {
        if let Some(profile) = profile {
            if let Some(socket) = &profile.docker_socket {
                engine.docker_host = Some(format!("unix://{}", socket.display()));
            }
            let update = EngineUpdate::ProfileChanged {
                engine: engine.id.clone(),
                profile,
            };
            if self.tx.send(update).await.is_err() {
                return;
            }
        }
        if self.has_client(&engine.id) {
            return;
        }
        let client: Result<Arc<dyn berth_engine::EngineClient>, String> = if self.mock {
            let world = super::mock_world_at(chrono::Utc::now());
            berth_engine::MockEngine::from_world(world, &engine.id)
                .map(|c| Arc::new(c) as Arc<dyn berth_engine::EngineClient>)
                .map_err(|err| err.to_string())
        } else {
            berth_engine::BollardEngine::connect(&engine)
                .map(|c| Arc::new(c) as Arc<dyn berth_engine::EngineClient>)
                .map_err(|err| err.to_string())
        };
        match client {
            Ok(client) => {
                match self.clients.lock() {
                    Ok(mut map) => map.insert(engine.id.clone(), Arc::clone(&client)),
                    Err(poisoned) => poisoned
                        .into_inner()
                        .insert(engine.id.clone(), Arc::clone(&client)),
                };
                spawn_io(run_engine(
                    client,
                    Arc::clone(&self.store),
                    self.current_pins(),
                    is_remote(&engine.kind),
                    self.tx.clone(),
                    self.cancel.child_token(),
                    RECONNECT_DELAY,
                ));
            }
            Err(reason) => {
                tracing::warn!(%reason, engine = %engine.id.0, "engine connect failed");
                let _ = self
                    .tx
                    .send(EngineUpdate::EngineFailed {
                        engine: engine.id.clone(),
                        reason,
                    })
                    .await;
            }
        }
    }
}

impl Drop for EnginesHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
        save_snapshots(&self.state, &self.store);
    }
}

/// Start everything from the App root (call inside `use_hook`): spawns
/// the receiver/applier and snapshot saver as Dioxus tasks and discovery
/// + engine loops as tokio tasks. `mock` is `BERTH_MOCK=1`.
pub fn start(
    mut state: AppState,
    store: Arc<berth_store::Store>,
    clients: ClientMap,
    cancel: CancellationToken,
    mock: bool,
) -> EnginesHandle {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<EngineUpdate>(256);
    let run_store = Arc::clone(&store);
    let run_clients = Arc::clone(&clients);
    let run_cancel = cancel.clone();
    let link_tx = tx.clone();
    let link_store = Arc::clone(&store);
    let link_clients = Arc::clone(&clients);
    let link_cancel = cancel.clone();
    spawn_io(async move {
        let (engines, pins) = if mock {
            mock_world_parts(&run_clients)
        } else {
            let engines = discover_and_connect(&run_store, &run_clients).await;
            let pins = store_pins(&run_store);
            (engines, pins)
        };
        let connected = connected_clients(&run_clients);
        run_with_clients(
            engines,
            connected,
            run_store,
            pins,
            tx,
            run_cancel,
            RECONNECT_DELAY,
        )
        .await;
    });

    let pins = if mock {
        berth_engine::MockWorld::bundled().pins
    } else {
        store_pins(&store)
    };
    spawn(async move {
        let mut applier = Applier::new(pins);
        while let Some(update) = rx.recv().await {
            applier.apply(&mut state, update);
        }
    });

    let saver_store = Arc::clone(&store);
    let saver_cancel = cancel.clone();
    spawn(async move {
        loop {
            tokio::select! {
                () = saver_cancel.cancelled() => {
                    save_snapshots(&state, &saver_store);
                    return;
                }
                () = tokio::time::sleep(SNAPSHOT_EVERY) => {
                    save_snapshots(&state, &saver_store);
                }
            }
        }
    });

    let link_pins = if mock {
        berth_engine::MockWorld::bundled().pins
    } else {
        store_pins(&link_store)
    };
    let link = EngineLink::new(
        link_clients,
        link_store,
        link_pins,
        link_tx,
        link_cancel,
        mock,
    );
    EnginesHandle {
        cancel,
        store,
        state,
        link,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use berth_engine::client::Bs;
    use berth_engine::EngineClient as _;
    use berth_engine::EngineError;

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

    fn apply(dom: &VirtualDom, applier: &mut Applier, state: &mut AppState, update: EngineUpdate) {
        dom.in_runtime(|| applier.apply(state, update));
    }

    fn read_containers(dom: &VirtualDom, state: &AppState) -> Vec<berth_core::Container> {
        dom.in_runtime(|| state.containers.peek().clone())
    }

    // --- Test double: MockEngine with delay / a stream that ends once. ---

    struct TestEngine {
        inner: berth_engine::MockEngine,
        delay: std::time::Duration,
        containers_calls: Arc<std::sync::atomic::AtomicUsize>,
        end_first_events: std::sync::atomic::AtomicBool,
    }

    impl TestEngine {
        fn new(engine: &berth_core::EngineId) -> Self {
            Self {
                inner: berth_engine::MockEngine::bundled(engine),
                delay: std::time::Duration::ZERO,
                containers_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                end_first_events: std::sync::atomic::AtomicBool::new(false),
            }
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
            self.containers_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
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
            if self
                .end_first_events
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return futures::stream::empty().boxed();
            }
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
            cancel: CancellationToken,
        ) -> Bs<berth_core::PullProgress> {
            self.inner.pull(reference, platform, cancel)
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

    fn default_engine_id() -> berth_core::EngineId {
        berth_core::EngineId("colima:default".to_string())
    }

    async fn recv(rx: &mut tokio::sync::mpsc::Receiver<EngineUpdate>) -> EngineUpdate {
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("update within 5s")
            .expect("channel open")
    }

    #[tokio::test]
    async fn services_event_updates_exactly_one_container_in_the_signal() {
        let (dom, mut state) = harness();
        let world = berth_engine::MockWorld::bundled();
        let mut applier = Applier::new(world.pins.clone());
        apply(
            &dom,
            &mut applier,
            &mut state,
            EngineUpdate::EnginesDiscovered(world.engines.clone()),
        );
        let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
        let client = Arc::new(TestEngine::new(&default_engine_id()));
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run_engine(
            Arc::clone(&client) as Arc<dyn berth_engine::EngineClient>,
            Arc::clone(&store),
            world.pins.clone(),
            false,
            tx,
            cancel.clone(),
            std::time::Duration::from_millis(10),
        ));
        let first = recv(&mut rx).await;
        assert!(
            matches!(first, EngineUpdate::EngineData { .. }),
            "{first:?}"
        );
        apply(&dom, &mut applier, &mut state, first);
        let before = read_containers(&dom, &state);
        assert!(before.len() > 5, "mock containers loaded: {}", before.len());
        let api = before
            .iter()
            .find(|c| c.name == "shop-api-api-1")
            .expect("api container")
            .clone();
        assert_eq!(api.state, berth_core::ContainerState::Running);

        client
            .act(
                &api.id,
                berth_core::ContainerAction::Stop { timeout_s: None },
            )
            .await
            .expect("stop");
        let update = recv(&mut rx).await;
        match &update {
            EngineUpdate::ContainerChanged { id, container, .. } => {
                assert_eq!(*id, api.id);
                assert_eq!(
                    container.as_ref().expect("refetched").state,
                    berth_core::ContainerState::Exited
                );
            }
            other => panic!("expected ContainerChanged, got {other:?}"),
        }
        apply(&dom, &mut applier, &mut state, update);
        let after = read_containers(&dom, &state);
        assert_eq!(after.len(), before.len(), "no row added or removed");
        let changed: Vec<&berth_core::Container> = after
            .iter()
            .filter(|c| {
                before
                    .iter()
                    .find(|b| b.id == c.id)
                    .is_some_and(|b| b != *c)
            })
            .collect();
        assert_eq!(changed.len(), 1, "exactly one row changed: {changed:?}");
        assert_eq!(changed[0].id, api.id);
        assert_eq!(changed[0].state, berth_core::ContainerState::Exited);
        // The event also landed in Activity (origin Engine, container.die).
        let entries = store
            .activity_query(&berth_store::ActivityFilter::default(), None, 10)
            .expect("activity");
        assert!(
            entries
                .iter()
                .any(|e| e.action == "container.die"
                    && e.origin == berth_core::ActivityOrigin::Engine),
            "{entries:?}"
        );
        cancel.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task).await;
    }

    #[tokio::test]
    async fn services_snapshot_is_in_signals_before_a_delayed_engine_answers() {
        let (dom, mut state) = harness();
        let world = berth_engine::MockWorld::bundled();
        let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
        let mut marker = world
            .containers_for(&default_engine_id())
            .into_iter()
            .next()
            .expect("a mock container");
        marker.name = "snapshot-marker".to_string();
        store
            .save_snapshot(&berth_store::EngineSnapshot {
                engine: default_engine_id(),
                taken: chrono::Utc::now(),
                containers: vec![marker],
                projects: Vec::new(),
            })
            .expect("seed snapshot");

        let mut test_engine = TestEngine::new(&default_engine_id());
        test_engine.delay = std::time::Duration::from_millis(500);
        let client: Arc<dyn berth_engine::EngineClient> = Arc::new(test_engine);
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run_with_clients(
            world.engines.clone(),
            vec![client],
            Arc::clone(&store),
            world.pins.clone(),
            tx,
            cancel.clone(),
            std::time::Duration::from_millis(10),
        ));

        let mut applier = Applier::new(world.pins.clone());
        let discovered = recv(&mut rx).await;
        assert!(matches!(discovered, EngineUpdate::EnginesDiscovered(_)));
        apply(&dom, &mut applier, &mut state, discovered);
        let snapshots = recv(&mut rx).await;
        assert!(
            matches!(snapshots, EngineUpdate::Snapshots(_)),
            "{snapshots:?}"
        );
        apply(&dom, &mut applier, &mut state, snapshots);

        // The delayed engine has not answered yet, but the snapshot row
        // is already what the UI would render.
        let shown = read_containers(&dom, &state);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].name, "snapshot-marker");
        assert!(dom.in_runtime(|| *state.snapshot_stale.peek()));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
                .await
                .is_err(),
            "live data arrived before the delay elapsed"
        );

        let live = recv(&mut rx).await;
        assert!(matches!(live, EngineUpdate::EngineData { .. }), "{live:?}");
        apply(&dom, &mut applier, &mut state, live);
        let shown = read_containers(&dom, &state);
        assert!(shown.len() > 5, "live list replaced the snapshot");
        assert!(!shown.iter().any(|c| c.name == "snapshot-marker"));
        assert!(!dom.in_runtime(|| *state.snapshot_stale.peek()));
        cancel.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task).await;
    }

    #[tokio::test]
    async fn services_reconnect_after_stream_end_triggers_a_full_refresh() {
        let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
        let test_engine = TestEngine::new(&default_engine_id());
        test_engine
            .end_first_events
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let calls = Arc::clone(&test_engine.containers_calls);
        let client: Arc<dyn berth_engine::EngineClient> = Arc::new(test_engine);
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run_engine(
            client,
            store,
            Vec::new(),
            false,
            tx,
            cancel.clone(),
            std::time::Duration::from_millis(10),
        ));
        let mut data_updates = 0;
        let mut saw_disconnect = false;
        while data_updates < 2 {
            match recv(&mut rx).await {
                EngineUpdate::EngineData { .. } => data_updates += 1,
                EngineUpdate::Disconnected { .. } => saw_disconnect = true,
                other => panic!("unexpected update {other:?}"),
            }
        }
        assert!(saw_disconnect, "stream end reported before reconnect");
        assert!(
            calls.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "containers() refetched on reconnect"
        );
        cancel.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task).await;
    }

    #[tokio::test]
    async fn services_activity_stream_end_records_a_gap_marker_for_that_engine() {
        let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
        let test_engine = TestEngine::new(&default_engine_id());
        test_engine
            .end_first_events
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let client: Arc<dyn berth_engine::EngineClient> = Arc::new(test_engine);
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run_engine(
            client,
            Arc::clone(&store),
            Vec::new(),
            false,
            tx,
            cancel.clone(),
            std::time::Duration::from_millis(10),
        ));
        loop {
            if matches!(recv(&mut rx).await, EngineUpdate::Disconnected { .. }) {
                break;
            }
        }
        cancel.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), task).await;
        let entries = store
            .activity_query(&berth_store::ActivityFilter::default(), None, 10)
            .expect("activity");
        let gaps: Vec<_> = entries
            .iter()
            .filter(|e| e.action == activity::GAP_ACTION)
            .collect();
        assert_eq!(gaps.len(), 1, "{entries:?}");
        assert_eq!(gaps[0].engine, default_engine_id());
        assert_eq!(
            gaps[0].subject,
            berth_core::Subject::Engine(default_engine_id())
        );
    }

    #[tokio::test]
    async fn services_full_refresh_groups_projects_and_fills_engine_counts() {
        let (dom, mut state) = harness();
        let world = berth_engine::MockWorld::bundled();
        let mock = berth_engine::MockEngine::bundled(&default_engine_id());
        let update = full_refresh(&mock, &world.pins, false)
            .await
            .expect("full refresh");
        let mut applier = Applier::new(world.pins.clone());
        apply(
            &dom,
            &mut applier,
            &mut state,
            EngineUpdate::EnginesDiscovered(world.engines.clone()),
        );
        // D81 default scope: first running Colima profile.
        let scope_is_default = dom.in_runtime(|| {
            matches!(
                &*state.engine_scope.peek(),
                EngineScope::One(id) if id.0 == "colima:default"
            )
        });
        assert!(scope_is_default);
        apply(&dom, &mut applier, &mut state, update);

        let projects = dom.in_runtime(|| state.projects.peek().clone());
        let shop = projects
            .iter()
            .find(|p| p.name == "shop-api")
            .expect("shop-api grouped");
        // worker is Restarting -> Failing (Q9 keeps mere Unhealthy out).
        assert_eq!(shop.status, berth_core::ProjectStatus::Failing);
        assert!(
            projects
                .iter()
                .any(|p| p.pinned && p.status == berth_core::ProjectStatus::NotCreated),
            "pinned not-created project present: {projects:?}"
        );
        let engines = dom.in_runtime(|| state.engines.peek().clone());
        let default = engines
            .iter()
            .find(|e| e.id == default_engine_id())
            .expect("default engine");
        assert_eq!(default.docker_version.as_deref(), Some("28.4.0"));
        assert!(default.counts.containers_total > 5);
        assert!(default.counts.containers_running > 0);
        assert!(default.counts.images > 0);
        // Volume sizes filled from volume_sizes().
        let volumes = dom.in_runtime(|| state.volumes.peek().clone());
        let pgdata = volumes
            .iter()
            .find(|v| v.name == "shop-api_pgdata")
            .expect("pgdata volume");
        assert_eq!(
            berth_core::fmt::bytes(pgdata.size.expect("size filled")),
            "1.2 GB"
        );
    }

    fn empty_engine_data(engine: berth_core::EngineId) -> EngineUpdate {
        EngineUpdate::EngineData {
            engine,
            info: None,
            containers: Vec::new(),
            projects: Vec::new(),
            images: Vec::new(),
            volumes: Vec::new(),
            networks: Vec::new(),
        }
    }

    fn projects_at(
        dom: &VirtualDom,
        state: &AppState,
        folder: &std::path::Path,
    ) -> Vec<berth_core::Project> {
        dom.in_runtime(|| {
            state
                .projects
                .peek()
                .iter()
                .filter(|p| p.folder.as_deref() == Some(folder))
                .cloned()
                .collect()
        })
    }

    #[tokio::test]
    async fn pins_a_folder_pinned_at_runtime_makes_its_project_appear_without_a_restart() {
        let (dom, mut state) = harness();
        let engine = default_engine_id();
        let folder = std::path::PathBuf::from("/Users/me/Dev/side/new-app");
        let store = Arc::new(berth_store::Store::open_in_memory().expect("store"));
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let link = EngineLink::new(
            client_map(),
            Arc::clone(&store),
            Vec::new(),
            tx,
            CancellationToken::new(),
            false,
        );
        // The service started with no pins and the engine answered.
        let mut applier = Applier::new(Vec::new());
        let world = berth_engine::MockWorld::bundled();
        apply(
            &dom,
            &mut applier,
            &mut state,
            EngineUpdate::EnginesDiscovered(world.engines),
        );
        apply(
            &dom,
            &mut applier,
            &mut state,
            empty_engine_data(engine.clone()),
        );
        assert!(projects_at(&dom, &state, &folder).is_empty());

        // The folder is pinned through the store, as Open folder and Review changes do.
        store
            .pin(&berth_core::PinnedFolder {
                folder: folder.clone(),
                engine: engine.clone(),
                added: chrono::Utc::now(),
            })
            .expect("pin");
        link.pins_changed();
        let update = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("the service was told")
            .expect("an update");
        assert!(
            matches!(&update, EngineUpdate::PinsChanged(pins) if pins.len() == 1),
            "{update:?}"
        );
        apply(&dom, &mut applier, &mut state, update);

        let found = projects_at(&dom, &state, &folder);
        assert_eq!(found.len(), 1, "the pinned folder is a project now");
        assert!(found[0].pinned);
        assert_eq!(found[0].name, "new-app");
        assert!(matches!(found[0].source, berth_core::ProjectSource::Pinned));

        // A reconnect refresh built by a loop that still holds the old pins keeps it.
        apply(&dom, &mut applier, &mut state, empty_engine_data(engine));
        assert_eq!(projects_at(&dom, &state, &folder).len(), 1);
    }

    #[test]
    fn services_applier_removes_a_destroyed_container_and_regroups() {
        let (dom, mut state) = harness();
        let world = berth_engine::MockWorld::bundled();
        let containers = world.containers_for(&default_engine_id());
        let projects =
            berth_engine::projects::group(&default_engine_id(), &containers, &world.pins, false);
        let victim = containers
            .iter()
            .find(|c| c.name == "shop-api-api-1")
            .expect("api")
            .clone();
        let total = containers.len();
        let mut applier = Applier::new(world.pins.clone());
        apply(
            &dom,
            &mut applier,
            &mut state,
            EngineUpdate::EnginesDiscovered(world.engines.clone()),
        );
        apply(
            &dom,
            &mut applier,
            &mut state,
            EngineUpdate::EngineData {
                engine: default_engine_id(),
                info: None,
                containers,
                projects,
                images: Vec::new(),
                volumes: Vec::new(),
                networks: Vec::new(),
            },
        );
        apply(
            &dom,
            &mut applier,
            &mut state,
            EngineUpdate::ContainerChanged {
                engine: default_engine_id(),
                id: victim.id.clone(),
                container: None,
            },
        );
        let after = read_containers(&dom, &state);
        assert_eq!(after.len(), total - 1);
        assert!(!after.iter().any(|c| c.id == victim.id));
        let projects = dom.in_runtime(|| state.projects.peek().clone());
        let shop = projects
            .iter()
            .find(|p| p.name == "shop-api")
            .expect("shop-api still grouped");
        assert!(
            !shop
                .services
                .iter()
                .any(|s| s.containers.contains(&victim.id)),
            "destroyed container left its service summary"
        );
    }

    /// The row ids the Containers table shows, top to bottom, in the
    /// default view (no sort picked) for either grouping.
    fn table_order(
        dom: &VirtualDom,
        state: &AppState,
        group_by: crate::screens::containers::GroupBy,
    ) -> Vec<String> {
        use crate::screens::containers as screen;
        dom.in_runtime(|| {
            let containers = state.containers.peek();
            let projects = state.projects.peek();
            let engines = state.engines.peek();
            let scope = crate::state::EngineScope::All;
            let filters = screen::Filters::default();
            // Every group open: a group that has nothing running folds
            // itself away by default, which hides rows rather than moving
            // them.
            let mut collapsed = std::collections::BTreeMap::new();
            collapsed.insert("unassigned".to_string(), false);
            for project in projects.iter() {
                collapsed.insert(format!("project:{}", project.id.0), false);
            }
            let pending = std::collections::BTreeMap::new();
            let model = screen::build_list(&screen::ListInput {
                containers: &containers,
                projects: &projects,
                engines: &engines,
                scope: &scope,
                filters: &filters,
                group_by,
                sort: None,
                collapsed: &collapsed,
                pending: &pending,
            });
            model
                .selection_order()
                .iter()
                .filter_map(|id| containers.iter().find(|c| c.id == *id))
                .map(|c| c.name.clone())
                .collect()
        })
    }

    /// Bug: an event refetches one container, the applier moved it to the
    /// end of the list, and the table (which kept the list order) swapped
    /// the row under the pointer. A row keeps its place through stop,
    /// start, restart and a plain refetch.
    #[test]
    fn services_container_events_never_move_a_table_row() {
        use crate::screens::containers::GroupBy;
        let (dom, mut state) = harness();
        let world = berth_engine::MockWorld::bundled();
        let containers = world.containers_for(&default_engine_id());
        let projects =
            berth_engine::projects::group(&default_engine_id(), &containers, &world.pins, false);
        let mut applier = Applier::new(world.pins.clone());
        apply(
            &dom,
            &mut applier,
            &mut state,
            EngineUpdate::EnginesDiscovered(world.engines.clone()),
        );
        apply(
            &dom,
            &mut applier,
            &mut state,
            EngineUpdate::EngineData {
                engine: default_engine_id(),
                info: None,
                containers: containers.clone(),
                projects,
                images: Vec::new(),
                volumes: Vec::new(),
                networks: Vec::new(),
            },
        );
        let flat = table_order(&dom, &state, GroupBy::None);
        let grouped = table_order(&dom, &state, GroupBy::Project);
        assert!(flat.len() > 5, "rows: {}", flat.len());

        let steps = [
            berth_core::ContainerState::Exited,
            berth_core::ContainerState::Running,
            berth_core::ContainerState::Restarting,
            berth_core::ContainerState::Running,
        ];
        for original in &containers {
            for step in steps {
                let mut changed = original.clone();
                changed.state = step;
                apply(
                    &dom,
                    &mut applier,
                    &mut state,
                    EngineUpdate::ContainerChanged {
                        engine: default_engine_id(),
                        id: original.id.clone(),
                        container: Some(changed),
                    },
                );
                assert_eq!(
                    table_order(&dom, &state, GroupBy::None),
                    flat,
                    "{} -> {step:?} moved a flat row",
                    original.name
                );
                assert_eq!(
                    table_order(&dom, &state, GroupBy::Project),
                    grouped,
                    "{} -> {step:?} moved a grouped row",
                    original.name
                );
            }
        }
    }

    #[test]
    fn services_disconnect_and_failure_mark_stale_and_unreachable() {
        let (dom, mut state) = harness();
        let world = berth_engine::MockWorld::bundled();
        let mut applier = Applier::new(world.pins.clone());
        apply(
            &dom,
            &mut applier,
            &mut state,
            EngineUpdate::EnginesDiscovered(world.engines.clone()),
        );
        apply(
            &dom,
            &mut applier,
            &mut state,
            EngineUpdate::Disconnected {
                engine: default_engine_id(),
            },
        );
        assert!(dom.in_runtime(|| *state.snapshot_stale.peek()));
        apply(
            &dom,
            &mut applier,
            &mut state,
            EngineUpdate::EngineFailed {
                engine: default_engine_id(),
                reason: "socket gone".to_string(),
            },
        );
        let engines = dom.in_runtime(|| state.engines.peek().clone());
        let default = engines
            .iter()
            .find(|e| e.id == default_engine_id())
            .expect("default");
        assert!(matches!(
            &default.state,
            berth_core::EngineState::Unreachable { reason, .. } if reason == "socket gone"
        ));
    }

    #[test]
    fn services_save_then_load_snapshots_round_trip_through_the_store() {
        let world = berth_engine::MockWorld::bundled();
        let store = berth_store::Store::open_in_memory().expect("store");
        let names: Vec<String> = {
            let (dom, mut state) = harness();
            let containers = world.containers_for(&default_engine_id());
            let projects = berth_engine::projects::group(
                &default_engine_id(),
                &containers,
                &world.pins,
                false,
            );
            let mut applier = Applier::new(world.pins.clone());
            apply(
                &dom,
                &mut applier,
                &mut state,
                EngineUpdate::EnginesDiscovered(world.engines.clone()),
            );
            apply(
                &dom,
                &mut applier,
                &mut state,
                EngineUpdate::EngineData {
                    engine: default_engine_id(),
                    info: None,
                    containers,
                    projects,
                    images: Vec::new(),
                    volumes: Vec::new(),
                    networks: Vec::new(),
                },
            );
            let saved = dom.in_runtime(|| save_snapshots(&state, &store));
            assert_eq!(saved, 3, "one snapshot per discovered engine");
            read_containers(&dom, &state)
                .iter()
                .map(|c| c.name.clone())
                .collect()
        };
        assert!(!names.is_empty());
        let (dom, mut state) = harness();
        let loaded = dom.in_runtime(|| {
            state.engines.set(world.engines.clone());
            load_snapshots(&mut state, &store, &world.engines)
        });
        assert_eq!(loaded, 3);
        let restored: Vec<String> = read_containers(&dom, &state)
            .iter()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(restored, names);
        assert!(dom.in_runtime(|| *state.snapshot_stale.peek()));
    }
}
