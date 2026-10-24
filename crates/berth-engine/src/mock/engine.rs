//! [`MockEngine`]: a full in-memory [`EngineClient`] over one engine's
//! slice of the [`MockWorld`](super::MockWorld). Reads come from the world;
//! `act` mutates state and emits exactly one event about 150 ms later;
//! `logs`/`stats` replay the scripted lines. Mutations the mock world has
//! no script for (`create`, `recreate`, `exec`, file transfer) return
//! [`EngineError::Unsupported`].

use futures::StreamExt;
use tokio_util::sync::CancellationToken as Cancel;

use super::{MockWorld, BUNDLED_JSON};
use crate::client::{unsupported_stream, Bs, EngineClient, ExecHandle};
use crate::EngineError;

const EVENT_DELAY: std::time::Duration = std::time::Duration::from_millis(150);
const STREAM_PACE: std::time::Duration = std::time::Duration::from_millis(50);

struct State {
    containers: Vec<berth_core::Container>,
    images: Vec<berth_core::Image>,
    volumes: Vec<berth_core::Volume>,
    networks: Vec<berth_core::Network>,
    limits: std::collections::BTreeMap<String, (berth_core::Limits, berth_core::RestartPolicy)>,
}

pub struct MockEngine {
    id: berth_core::EngineId,
    docker_host: String,
    info: berth_core::EngineInfo,
    disk_usage: berth_core::DiskUsage,
    world: MockWorld,
    state: std::sync::Arc<std::sync::Mutex<State>>,
    events: tokio::sync::broadcast::Sender<berth_core::EngineEvent>,
}

fn lock_err() -> EngineError {
    EngineError::Docker("mock engine lock poisoned".to_string())
}

fn find_container<'a>(
    containers: &'a [berth_core::Container],
    id: &berth_core::ContainerId,
) -> Option<&'a berth_core::Container> {
    containers
        .iter()
        .find(|c| c.id == *id || c.id.0.starts_with(id.0.as_str()))
}

impl MockEngine {
    pub fn from_fixture(json: &str, engine: &berth_core::EngineId) -> Result<Self, EngineError> {
        Self::from_world(MockWorld::from_json(json)?, engine)
    }

    pub fn from_world(
        world: MockWorld,
        engine: &berth_core::EngineId,
    ) -> Result<Self, EngineError> {
        let found = world
            .engine(engine)
            .ok_or_else(|| EngineError::NotFound(format!("mock engine {}", engine.0)))?;
        let docker_host = found
            .docker_host
            .clone()
            .unwrap_or_else(|| format!("mock://{}", engine.0));
        let info = world
            .engine_infos
            .get(&engine.0)
            .cloned()
            .unwrap_or(berth_core::EngineInfo {
                os: "linux".to_string(),
                kernel: String::new(),
                docker_version: found.docker_version.clone().unwrap_or_default(),
                api_version: String::new(),
                compose_version: None,
                storage_driver: "overlay2".to_string(),
                ncpu: 0,
                mem_total: bytesize::ByteSize(0),
                arch: berth_core::Arch::Aarch64,
            });
        let state = State {
            containers: world.containers_for(engine),
            images: world
                .images_by_engine
                .get(&engine.0)
                .cloned()
                .unwrap_or_default(),
            volumes: world
                .volumes_by_engine
                .get(&engine.0)
                .cloned()
                .unwrap_or_default(),
            networks: world
                .networks_by_engine
                .get(&engine.0)
                .cloned()
                .unwrap_or_default(),
            limits: std::collections::BTreeMap::new(),
        };
        let disk_usage = world
            .disk_usage_by_engine
            .get(&engine.0)
            .cloned()
            .unwrap_or_default();
        let (events, _) = tokio::sync::broadcast::channel(64);
        Ok(Self {
            id: engine.clone(),
            docker_host,
            info,
            disk_usage,
            world,
            state: std::sync::Arc::new(std::sync::Mutex::new(state)),
            events,
        })
    }

    /// The bundled world (PLAN section 6.5). An engine id the fixture does
    /// not name yields an empty engine rather than panicking; use
    /// [`Self::from_fixture`] to surface that as an error.
    #[must_use]
    pub fn bundled(engine: &berth_core::EngineId) -> Self {
        Self::from_fixture(BUNDLED_JSON, engine).unwrap_or_else(|_| {
            let (events, _) = tokio::sync::broadcast::channel(64);
            Self {
                id: engine.clone(),
                docker_host: format!("mock://{}", engine.0),
                info: berth_core::EngineInfo {
                    os: "linux".to_string(),
                    kernel: String::new(),
                    docker_version: String::new(),
                    api_version: String::new(),
                    compose_version: None,
                    storage_driver: "overlay2".to_string(),
                    ncpu: 0,
                    mem_total: bytesize::ByteSize(0),
                    arch: berth_core::Arch::Aarch64,
                },
                disk_usage: berth_core::DiskUsage::default(),
                world: MockWorld::default(),
                state: std::sync::Arc::new(std::sync::Mutex::new(State {
                    containers: Vec::new(),
                    images: Vec::new(),
                    volumes: Vec::new(),
                    networks: Vec::new(),
                    limits: std::collections::BTreeMap::new(),
                })),
                events,
            }
        })
    }

    fn emit_later(&self, event: berth_core::EngineEvent) {
        let sender = self.events.clone();
        tokio::spawn(async move {
            tokio::time::sleep(EVENT_DELAY).await;
            let _ = sender.send(event);
        });
    }

    fn container_event(
        &self,
        id: &berth_core::ContainerId,
        action: &str,
    ) -> berth_core::EngineEvent {
        berth_core::EngineEvent {
            engine: self.id.clone(),
            at: chrono::Utc::now(),
            kind: berth_core::EventKind::Container {
                id: id.clone(),
                action: action.to_string(),
            },
            attributes: std::collections::BTreeMap::new(),
        }
    }
}

#[async_trait::async_trait]
impl EngineClient for MockEngine {
    fn id(&self) -> &berth_core::EngineId {
        &self.id
    }

    fn docker_host(&self) -> &str {
        &self.docker_host
    }

    async fn info(&self) -> Result<berth_core::EngineInfo, EngineError> {
        Ok(self.info.clone())
    }

    async fn containers(&self) -> Result<Vec<berth_core::Container>, EngineError> {
        self.state
            .lock()
            .map(|s| s.containers.clone())
            .map_err(|_| lock_err())
    }

    async fn container(
        &self,
        id: &berth_core::ContainerId,
    ) -> Result<berth_core::ContainerDetail, EngineError> {
        let state = self.state.lock().map_err(|_| lock_err())?;
        let container = find_container(&state.containers, id)
            .cloned()
            .ok_or_else(|| EngineError::NotFound(format!("container {}", id.0)))?;
        let (limits, restart) = state
            .limits
            .get(&container.id.0)
            .cloned()
            .map_or((None, None), |(l, r)| (Some(l), Some(r)));
        drop(state);
        let over = self.world.details.get(&container.name);
        let cmd = over.and_then(|o| o.cmd.clone()).or_else(|| {
            container.command.as_ref().map(|command| {
                command
                    .split_whitespace()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
        });
        Ok(berth_core::ContainerDetail {
            inspect: serde_json::to_value(&container).unwrap_or(serde_json::Value::Null),
            entrypoint: over.and_then(|o| o.entrypoint.clone()),
            cmd,
            env: over.map(|o| o.env.clone()).unwrap_or_default(),
            working_dir: over.and_then(|o| o.working_dir.clone()),
            user: over.and_then(|o| o.user.clone()),
            restart: restart
                .unwrap_or_else(|| over.map_or(berth_core::RestartPolicy::No, |o| o.restart)),
            limits: limits.unwrap_or_else(|| {
                over.map_or_else(berth_core::Limits::default, |o| o.limits.clone())
            }),
            health_log: Vec::new(),
            container,
        })
    }

    async fn images(&self) -> Result<Vec<berth_core::Image>, EngineError> {
        let state = self.state.lock().map_err(|_| lock_err())?;
        let mut images = state.images.clone();
        for image in &mut images {
            image.used_by = state
                .containers
                .iter()
                .filter(|c| {
                    c.image.id.as_deref() == Some(image.id.as_str())
                        || image.repo_tags.contains(&c.image.reference)
                })
                .map(|c| c.id.clone())
                .collect();
        }
        Ok(images)
    }

    async fn image(&self, id: &str) -> Result<berth_core::ImageDetail, EngineError> {
        let images = self.images().await?;
        let image = images
            .into_iter()
            .find(|i| {
                i.id == id
                    || i.id
                        .trim_start_matches("sha256:")
                        .starts_with(id.trim_start_matches("sha256:"))
                    || i.repo_tags.iter().any(|t| t == id)
            })
            .ok_or_else(|| EngineError::NotFound(format!("image {id}")))?;
        let state = self.state.lock().map_err(|_| lock_err())?;
        let mut exposed: Vec<u16> = state
            .containers
            .iter()
            .filter(|c| image.used_by.contains(&c.id))
            .flat_map(|c| c.ports.iter().map(|p| p.container_port))
            .collect();
        exposed.sort_unstable();
        exposed.dedup();
        drop(state);
        Ok(berth_core::ImageDetail {
            layers: vec![
                berth_core::ImageLayer {
                    created_by: "/bin/sh -c #(nop) mock base layer".to_string(),
                    size: bytesize::ByteSize(image.size.as_u64() / 2),
                    created: Some(image.created),
                },
                berth_core::ImageLayer {
                    created_by: "/bin/sh -c #(nop) mock app layer".to_string(),
                    size: bytesize::ByteSize(image.size.as_u64() - image.size.as_u64() / 2),
                    created: Some(image.created),
                },
            ],
            env_keys: vec!["PATH".to_string()],
            exposed_ports: exposed,
            image,
        })
    }

    async fn volumes(&self) -> Result<Vec<berth_core::Volume>, EngineError> {
        self.state
            .lock()
            .map(|s| s.volumes.clone())
            .map_err(|_| lock_err())
    }

    async fn networks(&self) -> Result<Vec<berth_core::Network>, EngineError> {
        let state = self.state.lock().map_err(|_| lock_err())?;
        let mut networks = state.networks.clone();
        for network in &mut networks {
            network.containers = state
                .containers
                .iter()
                .filter(|c| c.networks.contains(&network.name))
                .map(|c| c.id.clone())
                .collect();
        }
        Ok(networks)
    }

    async fn disk_usage(&self) -> Result<berth_core::DiskUsage, EngineError> {
        Ok(self.disk_usage.clone())
    }

    async fn volume_sizes(
        &self,
    ) -> Result<std::collections::BTreeMap<String, bytesize::ByteSize>, EngineError> {
        let state = self.state.lock().map_err(|_| lock_err())?;
        Ok(state
            .volumes
            .iter()
            .filter_map(|v| v.size.map(|size| (v.name.clone(), size)))
            .collect())
    }

    fn events(&self, cancel: Cancel) -> Bs<berth_core::EngineEvent> {
        let receiver = self.events.subscribe();
        futures::stream::unfold((receiver, cancel), |(mut receiver, cancel)| async move {
            loop {
                if cancel.is_cancelled() {
                    return None;
                }
                tokio::select! {
                    () = cancel.cancelled() => return None,
                    received = receiver.recv() => match received {
                        Ok(event) => return Some((Ok(event), (receiver, cancel))),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    },
                }
            }
        })
        .boxed()
    }

    fn logs(
        &self,
        id: &berth_core::ContainerId,
        q: berth_core::LogQuery,
        cancel: Cancel,
    ) -> Bs<berth_core::LogLine> {
        let found = self
            .state
            .lock()
            .ok()
            .and_then(|s| find_container(&s.containers, id).cloned());
        let Some(container) = found else {
            let message = format!("container {}", id.0);
            return futures::stream::once(async move { Err(EngineError::NotFound(message)) })
                .boxed();
        };
        let scripted = self
            .world
            .logs
            .get(&container.name)
            .cloned()
            .unwrap_or_default();
        let base =
            chrono::DateTime::from_timestamp(1_761_325_330, 0).unwrap_or_else(chrono::Utc::now);
        let mut lines: Vec<berth_core::LogLine> = scripted
            .into_iter()
            .enumerate()
            .map(|(i, line)| berth_core::LogLine {
                container: container.id.clone(),
                ts: base + chrono::Duration::seconds(i64::try_from(i).unwrap_or(0)),
                stream: line.stream,
                text: line.text,
            })
            .collect();
        if let Some(since) = q.since {
            lines.retain(|l| l.ts >= since);
        }
        if let Some(tail) = q.tail {
            let keep = usize::try_from(tail).unwrap_or(usize::MAX);
            if lines.len() > keep {
                lines = lines.split_off(lines.len() - keep);
            }
        }
        let follow = q.follow;
        futures::stream::unfold(
            (lines, 0_usize, cancel, follow),
            |(lines, idx, cancel, follow)| async move {
                if cancel.is_cancelled() || lines.is_empty() {
                    return None;
                }
                if idx >= lines.len() {
                    if !follow {
                        return None;
                    }
                    tokio::select! {
                        () = cancel.cancelled() => return None,
                        () = tokio::time::sleep(STREAM_PACE * 2) => {}
                    }
                    let line = lines[0].clone();
                    return Some((Ok(line), (lines, 1, cancel, follow)));
                }
                let line = lines[idx].clone();
                Some((Ok(line), (lines, idx + 1, cancel, follow)))
            },
        )
        .boxed()
    }

    fn stats(&self, id: &berth_core::ContainerId, cancel: Cancel) -> Bs<berth_core::LiveStats> {
        let found = self
            .state
            .lock()
            .ok()
            .and_then(|s| find_container(&s.containers, id).cloned());
        let Some(container) = found else {
            let message = format!("container {}", id.0);
            return futures::stream::once(async move { Err(EngineError::NotFound(message)) })
                .boxed();
        };
        let series = self
            .world
            .stats_series
            .get(&container.name)
            .cloned()
            .unwrap_or_else(|| container.stats.iter().cloned().collect());
        if series.is_empty() {
            return futures::stream::empty().boxed();
        }
        futures::stream::unfold(
            (series, 0_usize, cancel),
            |(series, idx, cancel)| async move {
                if cancel.is_cancelled() {
                    return None;
                }
                if idx > 0 {
                    tokio::select! {
                        () = cancel.cancelled() => return None,
                        () = tokio::time::sleep(STREAM_PACE) => {}
                    }
                }
                let sample = series[idx % series.len()].clone();
                Some((Ok(sample), (series, idx + 1, cancel)))
            },
        )
        .boxed()
    }

    async fn act(
        &self,
        id: &berth_core::ContainerId,
        action: berth_core::ContainerAction,
    ) -> Result<(), EngineError> {
        use berth_core::ContainerState as S;
        let (event_id, event_action) = {
            let mut state = self.state.lock().map_err(|_| lock_err())?;
            let pos = state
                .containers
                .iter()
                .position(|c| c.id == *id || c.id.0.starts_with(id.0.as_str()))
                .ok_or_else(|| EngineError::NotFound(format!("container {}", id.0)))?;
            let container_id = state.containers[pos].id.clone();
            let event_action = match &action {
                berth_core::ContainerAction::Start => {
                    state.containers[pos].state = S::Running;
                    state.containers[pos].exit_code = None;
                    state.containers[pos].started = Some(chrono::Utc::now());
                    "start"
                }
                berth_core::ContainerAction::Stop { .. } => {
                    state.containers[pos].state = S::Exited;
                    state.containers[pos].exit_code = Some(0);
                    state.containers[pos].health = None;
                    "die"
                }
                berth_core::ContainerAction::Restart { .. } => {
                    state.containers[pos].state = S::Running;
                    state.containers[pos].exit_code = None;
                    state.containers[pos].restart_count =
                        state.containers[pos].restart_count.saturating_add(1);
                    state.containers[pos].started = Some(chrono::Utc::now());
                    "start"
                }
                berth_core::ContainerAction::Pause => {
                    state.containers[pos].state = S::Paused;
                    "pause"
                }
                berth_core::ContainerAction::Unpause => {
                    state.containers[pos].state = S::Running;
                    "unpause"
                }
                berth_core::ContainerAction::Kill { .. } => {
                    state.containers[pos].state = S::Exited;
                    state.containers[pos].exit_code = Some(137);
                    state.containers[pos].health = None;
                    "die"
                }
                berth_core::ContainerAction::Remove { .. } => {
                    state.containers.remove(pos);
                    "destroy"
                }
                berth_core::ContainerAction::Rename { name } => {
                    state.containers[pos].name = name.clone();
                    "rename"
                }
            };
            (container_id, event_action)
        };
        self.emit_later(self.container_event(&event_id, event_action));
        Ok(())
    }

    async fn update_limits(
        &self,
        id: &berth_core::ContainerId,
        limits: &berth_core::Limits,
        restart: &berth_core::RestartPolicy,
    ) -> Result<(), EngineError> {
        let mut state = self.state.lock().map_err(|_| lock_err())?;
        let container = find_container(&state.containers, id)
            .ok_or_else(|| EngineError::NotFound(format!("container {}", id.0)))?;
        let key = container.id.0.clone();
        state.limits.insert(key, (limits.clone(), *restart));
        Ok(())
    }

    async fn create(
        &self,
        _spec: &berth_core::ContainerSpec,
        _start: bool,
    ) -> Result<berth_core::ContainerId, EngineError> {
        Err(EngineError::Unsupported("create".into()))
    }

    async fn recreate(
        &self,
        _id: &berth_core::ContainerId,
        _spec: &berth_core::ContainerSpec,
    ) -> Result<berth_core::ContainerId, EngineError> {
        Err(EngineError::Unsupported("recreate".into()))
    }

    async fn exec(
        &self,
        _id: &berth_core::ContainerId,
        _req: berth_core::ExecRequest,
        _cancel: Cancel,
    ) -> Result<ExecHandle, EngineError> {
        Err(EngineError::Unsupported("exec".into()))
    }

    async fn list_files(
        &self,
        _target: &berth_core::FileTarget,
        _path: &str,
    ) -> Result<Vec<berth_core::FileEntry>, EngineError> {
        Err(EngineError::Unsupported("list_files".into()))
    }

    fn download(
        &self,
        _target: &berth_core::FileTarget,
        _path: &str,
        _cancel: Cancel,
    ) -> Bs<bytes::Bytes> {
        unsupported_stream("download")
    }

    async fn upload(
        &self,
        _target: &berth_core::FileTarget,
        _dir: &str,
        _tar: bytes::Bytes,
    ) -> Result<(), EngineError> {
        Err(EngineError::Unsupported("upload".into()))
    }

    fn pull(
        &self,
        reference: &str,
        _platform: Option<&str>,
        cancel: Cancel,
    ) -> Bs<berth_core::PullProgress> {
        let reference = reference.to_string();
        let state = self.state.clone();
        futures::stream::unfold(
            (0_usize, cancel, reference, state),
            |(step, cancel, reference, state)| async move {
                if cancel.is_cancelled() {
                    return None;
                }
                match step {
                    0 => Some((
                        Ok(berth_core::PullProgress {
                            layer: Some("mock-layer".to_string()),
                            status: format!("Pulling {reference}"),
                            current: Some(0),
                            total: Some(1),
                        }),
                        (1, cancel, reference, state),
                    )),
                    1 => {
                        if let Ok(mut state) = state.lock() {
                            if !state
                                .images
                                .iter()
                                .any(|i| i.repo_tags.contains(&reference))
                            {
                                state.images.push(berth_core::Image {
                                    id: format!(
                                        "sha256:mock-{}",
                                        reference.replace(['/', ':'], "-")
                                    ),
                                    repo_tags: vec![reference.clone()],
                                    digests: Vec::new(),
                                    size: bytesize::ByteSize::mb(10),
                                    created: chrono::Utc::now(),
                                    arch: berth_core::Arch::Aarch64,
                                    os: "linux".to_string(),
                                    used_by: Vec::new(),
                                    dangling: false,
                                });
                            }
                        }
                        Some((
                            Ok(berth_core::PullProgress {
                                layer: Some("mock-layer".to_string()),
                                status: "Pull complete".to_string(),
                                current: Some(1),
                                total: Some(1),
                            }),
                            (2, cancel, reference, state),
                        ))
                    }
                    _ => None,
                }
            },
        )
        .boxed()
    }

    async fn remove_image(&self, id: &str, force: bool) -> Result<(), EngineError> {
        let detail = self.image(id).await?;
        if !force && !detail.image.used_by.is_empty() {
            return Err(EngineError::Docker(format!(
                "image {} is used by {} container(s)",
                detail.image.id,
                detail.image.used_by.len()
            )));
        }
        let mut state = self.state.lock().map_err(|_| lock_err())?;
        state.images.retain(|i| i.id != detail.image.id);
        Ok(())
    }

    async fn search_images(
        &self,
        term: &str,
        limit: u32,
    ) -> Result<Vec<berth_core::ImageSearchHit>, EngineError> {
        let images = self.images().await?;
        Ok(images
            .iter()
            .flat_map(|i| i.repo_tags.iter())
            .filter(|tag| tag.contains(term))
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .map(|tag| berth_core::ImageSearchHit {
                name: tag.clone(),
                description: String::new(),
                stars: 0,
                official: false,
            })
            .collect())
    }

    async fn create_volume(
        &self,
        spec: &berth_core::VolumeSpec,
    ) -> Result<berth_core::Volume, EngineError> {
        let volume = berth_core::Volume {
            name: spec.name.clone(),
            driver: spec.driver.clone(),
            size: Some(bytesize::ByteSize(0)),
            mountpoint: format!("/var/lib/docker/volumes/{}/_data", spec.name),
            used_by: Vec::new(),
            project: None,
            created: Some(chrono::Utc::now()),
            labels: spec.labels.clone(),
        };
        let mut state = self.state.lock().map_err(|_| lock_err())?;
        if state.volumes.iter().any(|v| v.name == spec.name) {
            return Err(EngineError::Docker(format!(
                "volume {} already exists",
                spec.name
            )));
        }
        state.volumes.push(volume.clone());
        Ok(volume)
    }

    async fn remove_volume(&self, name: &str) -> Result<(), EngineError> {
        let mut state = self.state.lock().map_err(|_| lock_err())?;
        let before = state.volumes.len();
        state.volumes.retain(|v| v.name != name);
        if state.volumes.len() == before {
            return Err(EngineError::NotFound(format!("volume {name}")));
        }
        Ok(())
    }

    async fn create_network(
        &self,
        spec: &berth_core::NetworkSpec,
    ) -> Result<berth_core::Network, EngineError> {
        let network = berth_core::Network {
            id: format!("mock-net-{}", spec.name),
            name: spec.name.clone(),
            driver: spec.driver.clone(),
            subnet: spec.subnet.clone(),
            gateway: spec.gateway.clone(),
            internal: spec.internal,
            containers: spec.attach.clone(),
            project: None,
            built_in: false,
        };
        let mut state = self.state.lock().map_err(|_| lock_err())?;
        if state.networks.iter().any(|n| n.name == spec.name) {
            return Err(EngineError::Docker(format!(
                "network {} already exists",
                spec.name
            )));
        }
        for id in &spec.attach {
            if let Some(c) = state.containers.iter_mut().find(|c| c.id == *id) {
                if !c.networks.contains(&spec.name) {
                    c.networks.push(spec.name.clone());
                }
            }
        }
        state.networks.push(network.clone());
        Ok(network)
    }

    async fn remove_network(&self, id: &str) -> Result<(), EngineError> {
        let mut state = self.state.lock().map_err(|_| lock_err())?;
        let name = state
            .networks
            .iter()
            .find(|n| n.id == id || n.name == id)
            .map(|n| n.name.clone())
            .ok_or_else(|| EngineError::NotFound(format!("network {id}")))?;
        state.networks.retain(|n| n.name != name);
        for c in &mut state.containers {
            c.networks.retain(|n| n != &name);
        }
        Ok(())
    }

    async fn connect_network(
        &self,
        network: &str,
        id: &berth_core::ContainerId,
    ) -> Result<(), EngineError> {
        let mut state = self.state.lock().map_err(|_| lock_err())?;
        if !state
            .networks
            .iter()
            .any(|n| n.name == network || n.id == network)
        {
            return Err(EngineError::NotFound(format!("network {network}")));
        }
        let name = state
            .networks
            .iter()
            .find(|n| n.name == network || n.id == network)
            .map(|n| n.name.clone())
            .ok_or_else(|| EngineError::NotFound(format!("network {network}")))?;
        let container = state
            .containers
            .iter_mut()
            .find(|c| c.id == *id)
            .ok_or_else(|| EngineError::NotFound(format!("container {}", id.0)))?;
        if !container.networks.contains(&name) {
            container.networks.push(name);
        }
        Ok(())
    }

    async fn disconnect_network(
        &self,
        network: &str,
        id: &berth_core::ContainerId,
    ) -> Result<(), EngineError> {
        let mut state = self.state.lock().map_err(|_| lock_err())?;
        let container = state
            .containers
            .iter_mut()
            .find(|c| c.id == *id)
            .ok_or_else(|| EngineError::NotFound(format!("container {}", id.0)))?;
        container.networks.retain(|n| n != network);
        Ok(())
    }

    async fn prune(
        &self,
        plan: &berth_core::PrunePlan,
        cancel: Cancel,
    ) -> Result<berth_core::PruneReport, EngineError> {
        let mut removed = Vec::new();
        let mut failed = Vec::new();
        let mut freed = 0_u64;
        for item in plan.items.iter().filter(|i| i.selected) {
            if cancel.is_cancelled() {
                return Err(EngineError::Cancelled);
            }
            let result = match item.kind {
                berth_core::PruneKind::Container => self
                    .act(
                        &berth_core::ContainerId(item.id.clone()),
                        berth_core::ContainerAction::Remove {
                            force: true,
                            volumes: false,
                        },
                    )
                    .await
                    .map_err(|e| e.to_string()),
                berth_core::PruneKind::Image => self
                    .remove_image(&item.id, true)
                    .await
                    .map_err(|e| e.to_string()),
                berth_core::PruneKind::Volume => self
                    .remove_volume(&item.id)
                    .await
                    .map_err(|e| e.to_string()),
                berth_core::PruneKind::Network => self
                    .remove_network(&item.id)
                    .await
                    .map_err(|e| e.to_string()),
                berth_core::PruneKind::BuildCache => Ok(()),
            };
            match result {
                Ok(()) => {
                    freed = freed.saturating_add(item.size.map_or(0, |s| s.as_u64()));
                    removed.push(item.clone());
                }
                Err(message) => failed.push((item.clone(), message)),
            }
        }
        Ok(berth_core::PruneReport {
            removed,
            failed,
            freed: bytesize::ByteSize(freed),
        })
    }
}
