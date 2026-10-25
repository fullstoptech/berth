//! `BollardEngine`: `EngineClient` over a unix socket. Read side P1-T4,
//! streams and actions P1-T5, mutations (create/recreate/exec/files/
//! images/volumes/networks/prune) P1-T6.

use std::collections::HashMap;

use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions, StartExecResults};
use bollard::models::{
    ContainerCreateBody, ContainerUpdateBody, EndpointSettings, HealthConfig, HostConfig, Ipam,
    IpamConfig, Mount, MountType, NetworkConnectRequest, NetworkCreateRequest,
    NetworkDisconnectRequest, NetworkingConfig, PortBinding, PortMap,
    RestartPolicy as BollardRestart, RestartPolicyNameEnum, VolumeCreateRequest,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, DataUsageOptions,
    DownloadFromContainerOptionsBuilder, InspectContainerOptionsBuilder,
    KillContainerOptionsBuilder, ListContainersOptionsBuilder, ListImagesOptionsBuilder,
    LogsOptionsBuilder, PruneBuildOptions, RemoveContainerOptionsBuilder,
    RemoveImageOptionsBuilder, RemoveVolumeOptions, RenameContainerOptionsBuilder,
    RestartContainerOptionsBuilder, SearchImagesOptionsBuilder, StartContainerOptions,
    StatsOptionsBuilder, StopContainerOptionsBuilder, UploadToContainerOptionsBuilder,
};
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken as Cancel;

use crate::client::{Bs, EngineClient, ExecHandle};
use crate::mappers;
use crate::EngineError;

use crate::timeouts::{CallKind, LIST_INSPECT};

pub struct BollardEngine {
    docker: bollard::Docker,
    id: berth_core::EngineId,
    docker_host: String,
    engine_name: String,
}

fn map_err(engine_name: &str, err: bollard::errors::Error) -> EngineError {
    use bollard::errors::Error as B;
    match err {
        B::DockerResponseServerError {
            status_code: 404,
            message,
        } => EngineError::NotFound(message),
        B::DockerResponseServerError { message, .. } => EngineError::Docker(message),
        B::SocketNotFoundError(path) => EngineError::Unreachable {
            engine: engine_name.to_string(),
            reason: format!("socket {path} not found"),
        },
        B::DockerStreamError { error } => EngineError::Docker(error),
        B::RequestTimeoutError => EngineError::Timeout(LIST_INSPECT),
        other => EngineError::Unreachable {
            engine: engine_name.to_string(),
            reason: other.to_string(),
        },
    }
}

impl BollardEngine {
    pub fn connect(engine: &berth_core::Engine) -> Result<Self, EngineError> {
        let host = engine
            .docker_host
            .clone()
            .or_else(|| match &engine.kind {
                berth_core::EngineKind::Socket { path, .. } => {
                    Some(format!("unix://{}", path.display()))
                }
                _ => None,
            })
            .ok_or_else(|| EngineError::Unreachable {
                engine: engine.name.clone(),
                reason: "engine has no docker host".to_string(),
            })?;
        let Some(socket) = host.strip_prefix("unix://") else {
            // ssh/tcp engines arrive in P8-T1.
            return Err(EngineError::Unsupported(format!("connect to {host}")));
        };
        let docker = bollard::Docker::connect_with_unix(
            socket,
            LIST_INSPECT.as_secs(),
            bollard::API_DEFAULT_VERSION,
        )
        .map_err(|e| map_err(&engine.name, e))?;
        Ok(Self {
            docker,
            id: engine.id.clone(),
            docker_host: host,
            engine_name: engine.name.clone(),
        })
    }

    fn err(&self, e: bollard::errors::Error) -> EngineError {
        map_err(&self.engine_name, e)
    }

    /// The error of a call made through [`Self::docker_for`]: a timeout
    /// names that call's budget, not the default one.
    fn err_for(&self, kind: CallKind, e: bollard::errors::Error) -> EngineError {
        match (e, kind.timeout()) {
            (bollard::errors::Error::RequestTimeoutError, Some(limit)) => {
                EngineError::Timeout(limit)
            }
            (e, _) => self.err(e),
        }
    }

    /// The client with the request timeout of `kind` (D29). The default
    /// client carries [`LIST_INSPECT`].
    fn docker_for(&self, kind: CallKind) -> bollard::Docker {
        self.docker.clone().with_timeout(kind.for_bollard())
    }

    async fn raw_containers(&self) -> Result<Vec<berth_core::Container>, EngineError> {
        let options = ListContainersOptionsBuilder::new().all(true).build();
        let summaries = self
            .docker
            .list_containers(Some(options))
            .await
            .map_err(|e| self.err(e))?;
        Ok(summaries
            .iter()
            .map(|s| mappers::container(&self.id, s))
            .collect())
    }

    async fn start_by_id(&self, id: &str) -> Result<(), EngineError> {
        self.docker
            .start_container(id, None::<StartContainerOptions>)
            .await
            .map_err(|e| self.err(e))
    }

    async fn remove_by_id(&self, id: &str, force: bool) -> Result<(), EngineError> {
        let options = RemoveContainerOptionsBuilder::new().force(force).build();
        self.docker
            .remove_container(id, Some(options))
            .await
            .map_err(|e| self.err(e))
    }

    /// Create from a spec against an explicit name (recreate restores the
    /// old name itself, so it needs this separate from `EngineClient::create`).
    async fn create_from_spec(
        &self,
        spec: &berth_core::ContainerSpec,
        start: bool,
    ) -> Result<berth_core::ContainerId, EngineError> {
        let mut builder = CreateContainerOptionsBuilder::new().name(&spec.name);
        if let Some(platform) = &spec.platform {
            builder = builder.platform(platform);
        }
        let created = self
            .docker
            .create_container(Some(builder.build()), spec_to_create_body(spec))
            .await
            .map_err(|e| self.err(e))?;
        let id = berth_core::ContainerId(created.id);
        // Docker only honours one endpoint at create time reliably; the
        // first network rides in NetworkingConfig, the rest connect here.
        for network in spec.networks.iter().skip(1) {
            let config = NetworkConnectRequest {
                container: id.0.clone(),
                endpoint_config: Some(endpoint_for(spec, network)),
            };
            if let Err(e) = self
                .docker
                .connect_network(network, config)
                .await
                .map_err(|e| self.err(e))
            {
                let _ = self.remove_by_id(&id.0, true).await;
                return Err(e);
            }
        }
        if start {
            if let Err(e) = self.start_by_id(&id.0).await {
                let _ = self.remove_by_id(&id.0, true).await;
                return Err(e);
            }
        }
        Ok(id)
    }

    /// D51 helper container for volume file access: `alpine:3` with the
    /// volume mounted at [`HELPER_MOUNT`]. The archive API works on a
    /// created (never started) container, so no process ever runs in it.
    /// The image is never pulled here (the caller confirms that, P1-T6);
    /// a missing helper image is a Docker error naming it.
    ///
    /// The returned guard removes the container when it is dropped, so a
    /// cancelled or dropped operation leaves nothing behind (D28).
    async fn helper_container(
        &self,
        volume: &str,
        read_only: bool,
    ) -> Result<HelperGuard, EngineError> {
        if self.docker.inspect_image(HELPER_IMAGE).await.is_err() {
            return Err(EngineError::Docker(format!(
                "helper image {HELPER_IMAGE} is not on this engine; pull it first"
            )));
        }
        let spec = helper_spec(&helper_name(), volume, read_only);
        let options = CreateContainerOptionsBuilder::new()
            .name(&spec.name)
            .build();
        let created = self
            .docker
            .create_container(Some(options), helper_create_body(&spec))
            .await
            .map_err(|e| self.err(e))?;
        Ok(HelperGuard {
            docker: self.docker.clone(),
            id: Some(created.id),
        })
    }

    /// Resolve a `FileTarget` to (container id, in-container path,
    /// helper-to-remove). `read_only` mounts the helper volume read-only.
    async fn resolve_target(
        &self,
        target: &berth_core::FileTarget,
        path: &str,
        read_only: bool,
    ) -> Result<(String, String, Option<HelperGuard>), EngineError> {
        match target {
            berth_core::FileTarget::Container(id) => Ok((id.0.clone(), path.to_string(), None)),
            berth_core::FileTarget::Volume(volume) => {
                let helper = self.helper_container(volume, read_only).await?;
                let id = helper.id().to_string();
                Ok((id, helper_path(path), Some(helper)))
            }
        }
    }
}

const HELPER_IMAGE: &str = "alpine:3";
const HELPER_MOUNT: &str = "/__berth_vol";
/// The label every helper container carries (D28). Fixtures use
/// `dev.berth.fixture`; a helper never does, so test cleanup cannot mistake
/// one for the other.
const HELPER_LABEL: &str = "dev.berth.helper";

fn helper_name() -> String {
    format!("berth-helper-{}", uuid::Uuid::now_v7().simple())
}

fn helper_spec(name: &str, volume: &str, read_only: bool) -> berth_core::ContainerSpec {
    berth_core::ContainerSpec {
        name: name.to_string(),
        image: berth_core::ImageRef {
            reference: HELPER_IMAGE.to_string(),
            id: None,
        },
        command: Some(vec!["sleep".to_string(), "300".to_string()]),
        entrypoint: None,
        env: Vec::new(),
        ports: Vec::new(),
        mounts: vec![berth_core::Mount {
            kind: berth_core::MountKind::Volume,
            source: volume.to_string(),
            destination: HELPER_MOUNT.to_string(),
            read_only,
            volume_name: Some(volume.to_string()),
        }],
        networks: Vec::new(),
        labels: [(HELPER_LABEL.to_string(), "1".to_string())]
            .into_iter()
            .collect(),
        restart: berth_core::RestartPolicy::No,
        limits: berth_core::Limits::default(),
        platform: None,
        working_dir: None,
        user: None,
        healthcheck: None,
        cap_add: Vec::new(),
        cap_drop: Vec::new(),
        network_aliases: std::collections::BTreeMap::new(),
    }
}

/// The create body of a helper: the spec's, with `auto_remove` so an
/// engine that ever starts it removes it on exit.
fn helper_create_body(spec: &berth_core::ContainerSpec) -> ContainerCreateBody {
    let mut body = spec_to_create_body(spec);
    let host = body.host_config.get_or_insert_with(HostConfig::default);
    host.auto_remove = Some(true);
    body
}

/// Owns one helper container. [`HelperGuard::remove`] removes it and waits;
/// dropping the guard without that (a cancelled stream, a dropped future, a
/// panic) force-removes it from a spawned task (D28).
struct HelperGuard {
    docker: bollard::Docker,
    id: Option<String>,
}

impl HelperGuard {
    fn id(&self) -> &str {
        self.id.as_deref().unwrap_or_default()
    }

    async fn remove(mut self) {
        if let Some(id) = self.id.take() {
            let options = RemoveContainerOptionsBuilder::new().force(true).build();
            let _ = self.docker.remove_container(&id, Some(options)).await;
        }
    }
}

impl Drop for HelperGuard {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else { return };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let docker = self.docker.clone();
        runtime.spawn(async move {
            let options = RemoveContainerOptionsBuilder::new().force(true).build();
            let _ = docker.remove_container(&id, Some(options)).await;
        });
    }
}

fn helper_path(path: &str) -> String {
    let rel = path.trim_matches('/');
    if rel.is_empty() {
        HELPER_MOUNT.to_string()
    } else {
        format!("{HELPER_MOUNT}/{rel}")
    }
}

#[async_trait::async_trait]
impl EngineClient for BollardEngine {
    fn id(&self) -> &berth_core::EngineId {
        &self.id
    }

    fn docker_host(&self) -> &str {
        &self.docker_host
    }

    async fn info(&self) -> Result<berth_core::EngineInfo, EngineError> {
        let info = self.docker.info().await.map_err(|e| self.err(e))?;
        let version = self.docker.version().await.map_err(|e| self.err(e))?;
        let arch_str = version
            .arch
            .clone()
            .or_else(|| info.architecture.clone())
            .unwrap_or_default();
        Ok(berth_core::EngineInfo {
            os: version
                .os
                .clone()
                .or_else(|| info.operating_system.clone())
                .unwrap_or_default(),
            kernel: version
                .kernel_version
                .clone()
                .or_else(|| info.kernel_version.clone())
                .unwrap_or_default(),
            docker_version: version
                .version
                .clone()
                .or_else(|| info.server_version.clone())
                .unwrap_or_default(),
            api_version: version.api_version.clone().unwrap_or_default(),
            compose_version: None,
            storage_driver: info.driver.clone().unwrap_or_default(),
            ncpu: info.ncpu.map_or(0, |n| u32::try_from(n).unwrap_or(0)),
            mem_total: bytesize::ByteSize(
                info.mem_total.map_or(0, |n| u64::try_from(n).unwrap_or(0)),
            ),
            arch: berth_core::Arch::parse(&arch_str),
        })
    }

    async fn containers(&self) -> Result<Vec<berth_core::Container>, EngineError> {
        self.raw_containers().await
    }

    async fn container(
        &self,
        id: &berth_core::ContainerId,
    ) -> Result<berth_core::ContainerDetail, EngineError> {
        let options = InspectContainerOptionsBuilder::new().size(false).build();
        let inspect = self
            .docker
            .inspect_container(&id.0, Some(options))
            .await
            .map_err(|e| match self.err(e) {
                EngineError::NotFound(_) => EngineError::NotFound(id.0.clone()),
                other => other,
            })?;
        Ok(mappers::detail(&self.id, &inspect))
    }

    async fn images(&self) -> Result<Vec<berth_core::Image>, EngineError> {
        let containers = self.raw_containers().await?;
        let options = ListImagesOptionsBuilder::new()
            .all(true)
            .manifests(true)
            .build();
        let summaries = self
            .docker
            .list_images(Some(options))
            .await
            .map_err(|e| self.err(e))?;
        Ok(summaries
            .iter()
            .map(|s| mappers::image(s, &containers))
            .collect())
    }

    async fn image(&self, id: &str) -> Result<berth_core::ImageDetail, EngineError> {
        let inspect = self
            .docker
            .inspect_image(id)
            .await
            .map_err(|e| match self.err(e) {
                EngineError::NotFound(_) => EngineError::NotFound(id.to_string()),
                other => other,
            })?;
        let history = self
            .docker
            .image_history(id)
            .await
            .map_err(|e| self.err(e))?;
        let containers = self.raw_containers().await?;
        let image = mappers::image_from_inspect(&inspect, &containers);
        let layers = history
            .iter()
            .map(|h| berth_core::ImageLayer {
                created_by: h.created_by.clone(),
                size: bytesize::ByteSize(u64::try_from(h.size).unwrap_or(0)),
                created: chrono::DateTime::from_timestamp(h.created, 0),
            })
            .collect();
        let config = inspect.config.as_ref();
        let env_keys = config
            .and_then(|c| c.env.as_ref())
            .map(|env| {
                env.iter()
                    .map(|e| e.split('=').next().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default();
        let exposed_ports = config
            .and_then(|c| c.exposed_ports.as_ref())
            .map(|ports| {
                let mut ports: Vec<u16> = ports
                    .iter()
                    .filter_map(|p| p.split('/').next()?.parse().ok())
                    .collect();
                ports.sort_unstable();
                ports.dedup();
                ports
            })
            .unwrap_or_default();
        Ok(berth_core::ImageDetail {
            image,
            layers,
            env_keys,
            exposed_ports,
        })
    }

    async fn volumes(&self) -> Result<Vec<berth_core::Volume>, EngineError> {
        let containers = self.raw_containers().await?;
        let list = self
            .docker
            .list_volumes(None::<bollard::query_parameters::ListVolumesOptions>)
            .await
            .map_err(|e| self.err(e))?;
        Ok(list
            .volumes
            .unwrap_or_default()
            .iter()
            .map(|v| mappers::volume(v, &containers))
            .collect())
    }

    async fn networks(&self) -> Result<Vec<berth_core::Network>, EngineError> {
        let list = self
            .docker
            .list_networks(None::<bollard::query_parameters::ListNetworksOptions>)
            .await
            .map_err(|e| self.err(e))?;
        let containers = self.raw_containers().await?;
        Ok(list
            .iter()
            .map(|n| {
                let mut network = mappers::network(n);
                network.containers = containers
                    .iter()
                    .filter(|c| c.networks.iter().any(|name| name == &network.name))
                    .map(|c| c.id.clone())
                    .collect();
                network
            })
            .collect())
    }

    async fn disk_usage(&self) -> Result<berth_core::DiskUsage, EngineError> {
        let df = self
            .docker_for(CallKind::DiskUsage)
            .df(None::<DataUsageOptions>)
            .await
            .map_err(|e| self.err_for(CallKind::DiskUsage, e))?;
        Ok(mappers::disk_usage(&df))
    }

    async fn volume_sizes(
        &self,
    ) -> Result<std::collections::BTreeMap<String, bytesize::ByteSize>, EngineError> {
        // `system df` walks every volume, so it has its own budget (D29).
        let df = self
            .docker_for(CallKind::DiskUsage)
            .df(None::<DataUsageOptions>)
            .await
            .map_err(|e| self.err_for(CallKind::DiskUsage, e))?;
        Ok(mappers::volume_sizes(&df))
    }

    fn events(&self, cancel: Cancel) -> Bs<berth_core::EngineEvent> {
        let docker = self.docker_for(CallKind::Events);
        let engine = self.id.clone();
        let name = self.engine_name.clone();
        let stream = docker.events(None::<bollard::query_parameters::EventsOptions>);
        cancellable(
            stream,
            cancel,
            move |msg| mappers::event(&engine, &msg),
            name,
        )
    }

    fn logs(
        &self,
        id: &berth_core::ContainerId,
        q: berth_core::LogQuery,
        cancel: Cancel,
    ) -> Bs<berth_core::LogLine> {
        let docker = self.docker_for(CallKind::Logs);
        let name = self.engine_name.clone();
        let tail = q.tail.map_or_else(|| "all".to_string(), |t| t.to_string());
        let mut builder = LogsOptionsBuilder::new()
            .follow(q.follow)
            .stdout(true)
            .stderr(true)
            .timestamps(true)
            .tail(&tail);
        if let Some(since) = q.since {
            builder = builder.since(i32::try_from(since.timestamp()).unwrap_or(0));
        }
        let stream = docker.logs(&id.0, Some(builder.build()));
        log_stream(stream, id.clone(), cancel, name)
    }

    fn stats(&self, id: &berth_core::ContainerId, cancel: Cancel) -> Bs<berth_core::LiveStats> {
        let docker = self.docker_for(CallKind::Stats);
        let name = self.engine_name.clone();
        let options = StatsOptionsBuilder::new()
            .stream(true)
            .one_shot(false)
            .build();
        let stream = docker.stats(&id.0, Some(options));
        cancellable(stream, cancel, |s| mappers::live_stats(&s), name)
    }

    async fn act(
        &self,
        id: &berth_core::ContainerId,
        action: berth_core::ContainerAction,
    ) -> Result<(), EngineError> {
        use berth_core::ContainerAction as A;
        match action {
            A::Start => self
                .docker
                .start_container(&id.0, None::<StartContainerOptions>)
                .await
                .map_err(|e| self.err(e)),
            A::Stop { timeout_s } => {
                let mut builder = StopContainerOptionsBuilder::new();
                if let Some(t) = timeout_s {
                    builder = builder.t(i32::try_from(t).unwrap_or(i32::MAX));
                }
                self.docker
                    .stop_container(&id.0, Some(builder.build()))
                    .await
                    .map_err(|e| self.err(e))
            }
            A::Restart { timeout_s } => {
                let mut builder = RestartContainerOptionsBuilder::new();
                if let Some(t) = timeout_s {
                    builder = builder.t(i32::try_from(t).unwrap_or(i32::MAX));
                }
                self.docker
                    .restart_container(&id.0, Some(builder.build()))
                    .await
                    .map_err(|e| self.err(e))
            }
            A::Pause => self
                .docker
                .pause_container(&id.0)
                .await
                .map_err(|e| self.err(e)),
            A::Unpause => self
                .docker
                .unpause_container(&id.0)
                .await
                .map_err(|e| self.err(e)),
            A::Kill { signal } => {
                let options = KillContainerOptionsBuilder::new()
                    .signal(signal.as_deref().unwrap_or("SIGKILL"))
                    .build();
                self.docker
                    .kill_container(&id.0, Some(options))
                    .await
                    .map_err(|e| self.err(e))
            }
            A::Remove { force, volumes } => {
                let options = RemoveContainerOptionsBuilder::new()
                    .force(force)
                    .v(volumes)
                    .build();
                self.docker
                    .remove_container(&id.0, Some(options))
                    .await
                    .map_err(|e| self.err(e))
            }
            A::Rename { name } => {
                let options = RenameContainerOptionsBuilder::new().name(&name).build();
                self.docker
                    .rename_container(&id.0, options)
                    .await
                    .map_err(|e| self.err(e))
            }
        }
    }

    async fn update_limits(
        &self,
        id: &berth_core::ContainerId,
        limits: &berth_core::Limits,
        restart: &berth_core::RestartPolicy,
    ) -> Result<(), EngineError> {
        let (restart_name, max_retries) = match restart {
            berth_core::RestartPolicy::No => (RestartPolicyNameEnum::NO, None),
            berth_core::RestartPolicy::Always => (RestartPolicyNameEnum::ALWAYS, None),
            berth_core::RestartPolicy::UnlessStopped => {
                (RestartPolicyNameEnum::UNLESS_STOPPED, None)
            }
            berth_core::RestartPolicy::OnFailure { max_retries } => (
                RestartPolicyNameEnum::ON_FAILURE,
                Some(i64::from(*max_retries)),
            ),
        };
        let body = ContainerUpdateBody {
            nano_cpus: limits.cpus.map(|c| (f64::from(c) * 1_000_000_000.0) as i64),
            memory: limits
                .memory
                .map(|b| i64::try_from(b.0).unwrap_or(i64::MAX)),
            memory_swap: limits
                .memory_swap
                .map(|b| i64::try_from(b.0).unwrap_or(i64::MAX)),
            restart_policy: Some(BollardRestart {
                name: Some(restart_name),
                maximum_retry_count: max_retries,
            }),
            ..Default::default()
        };
        self.docker
            .update_container(&id.0, body)
            .await
            .map_err(|e| self.err(e))
    }

    async fn create(
        &self,
        spec: &berth_core::ContainerSpec,
        start: bool,
    ) -> Result<berth_core::ContainerId, EngineError> {
        self.create_from_spec(spec, start).await
    }

    async fn recreate(
        &self,
        id: &berth_core::ContainerId,
        spec: &berth_core::ContainerSpec,
    ) -> Result<berth_core::ContainerId, EngineError> {
        // PLAN §6.5: stop, rename old to "<name>-berth-old", create, start,
        // remove old. On any failure the old container is renamed back and
        // restarted (when it was running).
        let detail = self.container(id).await?;
        let old_name = detail.container.name.clone();
        let was_running = matches!(
            detail.container.state,
            berth_core::ContainerState::Running | berth_core::ContainerState::Restarting
        );
        let was_paused = detail.container.state == berth_core::ContainerState::Paused;
        if was_paused {
            self.act(id, berth_core::ContainerAction::Unpause).await?;
        }
        if was_running || was_paused {
            // The list/inspect request timeout (5 s) is shorter than the stop, so a
            // container whose PID 1 ignores SIGTERM outlives the stop
            // request. The engine keeps stopping it server-side; poll for
            // the exit, then kill as the last resort.
            match self
                .act(
                    id,
                    berth_core::ContainerAction::Stop {
                        timeout_s: Some(10),
                    },
                )
                .await
            {
                Ok(()) => {}
                Err(EngineError::Timeout(_)) => {
                    let mut stopped = false;
                    for _ in 0..48 {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        match self.container(id).await {
                            Ok(d)
                                if !matches!(
                                    d.container.state,
                                    berth_core::ContainerState::Running
                                        | berth_core::ContainerState::Restarting
                                ) =>
                            {
                                stopped = true;
                                break;
                            }
                            Ok(_) => {}
                            Err(_) => {
                                stopped = true;
                                break;
                            }
                        }
                    }
                    if !stopped {
                        self.act(id, berth_core::ContainerAction::Kill { signal: None })
                            .await?;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        let backup_name = format!("{old_name}-berth-old");
        self.act(
            id,
            berth_core::ContainerAction::Rename { name: backup_name },
        )
        .await?;
        let start_new = was_running || was_paused;
        match self.create_from_spec(spec, start_new).await {
            Ok(new_id) => {
                if let Err(e) = self.remove_by_id(&id.0, true).await {
                    tracing::warn!(error = %e, "recreate: old container left behind");
                }
                Ok(new_id)
            }
            Err(create_err) => {
                let mut restore_errors = Vec::new();
                if let Err(e) = self
                    .act(
                        id,
                        berth_core::ContainerAction::Rename {
                            name: old_name.clone(),
                        },
                    )
                    .await
                {
                    restore_errors.push(e.to_string());
                }
                if start_new {
                    if let Err(e) = self.act(id, berth_core::ContainerAction::Start).await {
                        restore_errors.push(e.to_string());
                    }
                }
                if restore_errors.is_empty() {
                    Err(match create_err {
                        EngineError::Docker(m) => EngineError::Docker(m),
                        other => EngineError::Docker(other.to_string()),
                    })
                } else {
                    Err(EngineError::Docker(format!(
                        "recreate failed ({create_err}); restoring the old container also failed: {}",
                        restore_errors.join("; ")
                    )))
                }
            }
        }
    }

    async fn exec(
        &self,
        id: &berth_core::ContainerId,
        req: berth_core::ExecRequest,
        cancel: Cancel,
    ) -> Result<ExecHandle, EngineError> {
        // D41: probe bash, then sh, unless the caller picked a shell.
        let candidates: Vec<String> = match &req.shell {
            Some(shell) => vec![shell.clone()],
            None => vec!["bash".to_string(), "sh".to_string()],
        };
        let mut last_err: Option<EngineError> = None;
        for shell in candidates {
            let config = CreateExecOptions {
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                tty: Some(true),
                detach_keys: None,
                env: None::<Vec<String>>,
                cmd: Some(vec![shell.clone()]),
                privileged: None,
                user: req.user.clone(),
                working_dir: req.workdir.clone(),
            };
            let created = match self.docker.create_exec(&id.0, config).await {
                Ok(created) => created,
                Err(e) => {
                    last_err = Some(self.err(e));
                    continue;
                }
            };
            let started = self
                .docker_for(CallKind::Attach)
                .start_exec(
                    &created.id,
                    Some(StartExecOptions {
                        detach: false,
                        tty: true,
                        output_capacity: None,
                    }),
                )
                .await;
            let (output_stream, binput) = match started {
                Ok(StartExecResults::Attached { output, input }) => (output, input),
                Ok(StartExecResults::Detached) => {
                    last_err = Some(EngineError::Docker("exec detached unexpectedly".into()));
                    continue;
                }
                Err(e) => {
                    last_err = Some(self.err(e));
                    continue;
                }
            };
            let exec_id = created.id.clone();
            let _ = self
                .docker
                .resize_exec(
                    &exec_id,
                    ResizeExecOptions {
                        height: req.rows,
                        width: req.cols,
                    },
                )
                .await;
            let output = cancellable(
                output_stream,
                cancel.clone(),
                |out| match out {
                    LogOutput::StdErr { message }
                    | LogOutput::StdOut { message }
                    | LogOutput::StdIn { message }
                    | LogOutput::Console { message } => message,
                },
                self.engine_name.clone(),
            );
            let (input_tx, mut input_rx) = tokio::sync::mpsc::channel::<bytes::Bytes>(32);
            let input_cancel = cancel.clone();
            tokio::spawn(async move {
                let mut binput = binput;
                loop {
                    let chunk = tokio::select! {
                        biased;
                        () = input_cancel.cancelled() => break,
                        chunk = input_rx.recv() => chunk,
                    };
                    match chunk {
                        Some(chunk) => {
                            if binput.write_all(&chunk).await.is_err() {
                                break;
                            }
                            if binput.flush().await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                let _ = binput.shutdown().await;
            });
            let (resize_tx, mut resize_rx) = tokio::sync::mpsc::channel::<(u16, u16)>(8);
            let resize_docker = self.docker.clone();
            let resize_cancel = cancel.clone();
            tokio::spawn(async move {
                loop {
                    let size = tokio::select! {
                        biased;
                        () = resize_cancel.cancelled() => break,
                        size = resize_rx.recv() => size,
                    };
                    match size {
                        Some((cols, rows)) => {
                            let _ = resize_docker
                                .resize_exec(
                                    &exec_id,
                                    ResizeExecOptions {
                                        height: rows,
                                        width: cols,
                                    },
                                )
                                .await;
                        }
                        None => break,
                    }
                }
            });
            return Ok(ExecHandle::new(
                berth_core::ExecSession {
                    id: created.id,
                    container: id.clone(),
                    shell,
                    cols: req.cols,
                    rows: req.rows,
                },
                output,
                input_tx,
                resize_tx,
            ));
        }
        Err(last_err.unwrap_or_else(|| EngineError::Docker("no shell found in container".into())))
    }

    async fn list_files(
        &self,
        target: &berth_core::FileTarget,
        path: &str,
    ) -> Result<Vec<berth_core::FileEntry>, EngineError> {
        // Listing through the archive API (D51's fallback): works with no
        // shell and on stopped containers; volumes go through the helper.
        // The tar streams through a parser that keeps headers only.
        let shown = crate::files::normalize(path);
        // The archive's root is named after the path the engine sees: a
        // volume's `/` is the helper's mount point.
        let in_path = match target {
            berth_core::FileTarget::Container(_) => shown.clone(),
            berth_core::FileTarget::Volume(_) => helper_path(path),
        };
        // Dropping this future cancels the download.
        let cancel = Cancel::new();
        let _stop = cancel.clone().drop_guard();
        crate::files::list_from_stream(self.download(target, path, cancel), in_path, shown).await
    }

    fn download(
        &self,
        target: &berth_core::FileTarget,
        path: &str,
        cancel: Cancel,
    ) -> Bs<bytes::Bytes> {
        // Raw tar stream from the archive API (PLAN §6.5: "tar stream").
        let engine = Self {
            docker: self.docker.clone(),
            id: self.id.clone(),
            docker_host: self.docker_host.clone(),
            engine_name: self.engine_name.clone(),
        };
        let target = target.clone();
        let path = path.to_string();
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, EngineError>>(32);
        tokio::spawn(async move {
            let resolved = engine.resolve_target(&target, &path, true).await;
            let (container, in_path, helper) = match resolved {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            };
            let options = DownloadFromContainerOptionsBuilder::new()
                .path(&in_path)
                .build();
            let mut stream = Box::pin(
                engine
                    .docker_for(CallKind::Export)
                    .download_from_container(&container, Some(options)),
            );
            loop {
                let item = tokio::select! {
                    biased;
                    () = cancel.cancelled() => break,
                    item = stream.next() => item,
                };
                match item {
                    Some(Ok(chunk)) => {
                        if tx.send(Ok(chunk)).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        let _ = tx.send(Err(engine.err(e))).await;
                        break;
                    }
                    None => break,
                }
            }
            if let Some(helper) = helper {
                helper.remove().await;
            }
        });
        channel_stream(rx)
    }

    async fn upload(
        &self,
        target: &berth_core::FileTarget,
        dir: &str,
        tar: bytes::Bytes,
    ) -> Result<(), EngineError> {
        let (container, in_path, helper) = self.resolve_target(target, dir, false).await?;
        let options = UploadToContainerOptionsBuilder::new()
            .path(&in_path)
            .build();
        let result = self
            .docker_for(CallKind::Upload)
            .upload_to_container(&container, Some(options), bollard::body_full(tar))
            .await
            .map_err(|e| self.err(e));
        if let Some(helper) = helper {
            helper.remove().await;
        }
        result
    }

    fn pull(
        &self,
        reference: &str,
        platform: Option<&str>,
        cancel: Cancel,
    ) -> Bs<berth_core::PullProgress> {
        let (from_image, tag) = split_image_ref(reference);
        let mut builder = CreateImageOptionsBuilder::new().from_image(&from_image);
        if let Some(tag) = &tag {
            builder = builder.tag(tag);
        }
        if let Some(platform) = platform {
            builder = builder.platform(platform);
        }
        let stream =
            self.docker_for(CallKind::Pull)
                .create_image(Some(builder.build()), None, None);
        cancellable(
            stream,
            cancel,
            |info| berth_core::PullProgress {
                layer: info.id,
                status: info.status.unwrap_or_default(),
                current: info
                    .progress_detail
                    .as_ref()
                    .and_then(|p| p.current)
                    .map(|n| u64::try_from(n).unwrap_or(0)),
                total: info
                    .progress_detail
                    .as_ref()
                    .and_then(|p| p.total)
                    .map(|n| u64::try_from(n).unwrap_or(0)),
            },
            self.engine_name.clone(),
        )
    }

    async fn remove_image(&self, id: &str, force: bool) -> Result<(), EngineError> {
        let options = RemoveImageOptionsBuilder::new().force(force).build();
        self.docker
            .remove_image(id, Some(options), None)
            .await
            .map(|_| ())
            .map_err(|e| self.err(e))
    }

    async fn search_images(
        &self,
        term: &str,
        limit: u32,
    ) -> Result<Vec<berth_core::ImageSearchHit>, EngineError> {
        let options = SearchImagesOptionsBuilder::new()
            .term(term)
            .limit(i32::try_from(limit).unwrap_or(i32::MAX))
            .build();
        let items = self
            .docker
            .search_images(options)
            .await
            .map_err(|e| self.err(e))?;
        Ok(items
            .iter()
            .map(|i| berth_core::ImageSearchHit {
                name: i.name.clone().unwrap_or_default(),
                description: i.description.clone().unwrap_or_default(),
                stars: i.star_count.map_or(0, |n| u32::try_from(n).unwrap_or(0)),
                official: i.is_official.unwrap_or(false),
            })
            .collect())
    }

    async fn create_volume(
        &self,
        spec: &berth_core::VolumeSpec,
    ) -> Result<berth_core::Volume, EngineError> {
        let config = VolumeCreateRequest {
            name: Some(spec.name.clone()),
            driver: Some(spec.driver.clone()).filter(|d| !d.is_empty()),
            driver_opts: Some(
                spec.driver_opts
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
            labels: Some(
                spec.labels
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
            cluster_volume_spec: None,
        };
        let volume = self
            .docker
            .create_volume(config)
            .await
            .map_err(|e| self.err(e))?;
        Ok(mappers::volume(&volume, &[]))
    }

    async fn remove_volume(&self, name: &str) -> Result<(), EngineError> {
        self.docker
            .remove_volume(name, None::<RemoveVolumeOptions>)
            .await
            .map_err(|e| self.err(e))
    }

    async fn create_network(
        &self,
        spec: &berth_core::NetworkSpec,
    ) -> Result<berth_core::Network, EngineError> {
        let ipam = if spec.subnet.is_some() || spec.gateway.is_some() {
            Some(Ipam {
                driver: None,
                config: Some(vec![IpamConfig {
                    subnet: spec.subnet.clone(),
                    ip_range: None,
                    gateway: spec.gateway.clone(),
                    auxiliary_addresses: None,
                }]),
                options: None,
            })
        } else {
            None
        };
        let config = NetworkCreateRequest {
            name: spec.name.clone(),
            driver: Some(spec.driver.clone()).filter(|d| !d.is_empty()),
            internal: Some(spec.internal),
            ipam,
            labels: Some(
                spec.labels
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )
            .filter(|labels: &std::collections::HashMap<String, String>| !labels.is_empty()),
            ..Default::default()
        };
        let created = self
            .docker
            .create_network(config)
            .await
            .map_err(|e| self.err(e))?;
        for container in &spec.attach {
            if let Err(e) = self.connect_network(&spec.name, container).await {
                let _ = self.docker.remove_network(&created.id).await;
                return Err(e);
            }
        }
        let networks = self.networks().await?;
        networks
            .into_iter()
            .find(|n| n.name == spec.name || n.id == created.id)
            .ok_or_else(|| EngineError::NotFound(spec.name.clone()))
    }

    async fn remove_network(&self, id: &str) -> Result<(), EngineError> {
        self.docker
            .remove_network(id)
            .await
            .map_err(|e| self.err(e))
    }

    async fn connect_network(
        &self,
        network: &str,
        id: &berth_core::ContainerId,
    ) -> Result<(), EngineError> {
        let config = NetworkConnectRequest {
            container: id.0.clone(),
            endpoint_config: None,
        };
        self.docker
            .connect_network(network, config)
            .await
            .map_err(|e| self.err(e))
    }

    async fn disconnect_network(
        &self,
        network: &str,
        id: &berth_core::ContainerId,
    ) -> Result<(), EngineError> {
        let config = NetworkDisconnectRequest {
            container: id.0.clone(),
            force: Some(false),
        };
        self.docker
            .disconnect_network(network, config)
            .await
            .map_err(|e| self.err(e))
    }

    async fn prune(
        &self,
        plan: &berth_core::PrunePlan,
        cancel: Cancel,
    ) -> Result<berth_core::PruneReport, EngineError> {
        // Item by item through the ordinary remove APIs; build cache goes
        // through prune_build only when a BuildCache item is selected.
        let mut removed = Vec::new();
        let mut failed = Vec::new();
        let mut freed: u64 = 0;
        let mut build_cache_pruned = false;
        for item in plan.items.iter().filter(|i| i.selected) {
            if cancel.is_cancelled() {
                failed.push((item.clone(), "cancelled".to_string()));
                continue;
            }
            let (result, extra_freed) = match item.kind {
                berth_core::PruneKind::Container => (self.remove_by_id(&item.id, true).await, None),
                berth_core::PruneKind::Image => (self.remove_image(&item.id, false).await, None),
                berth_core::PruneKind::Volume => (self.remove_volume(&item.id).await, None),
                berth_core::PruneKind::Network => (self.remove_network(&item.id).await, None),
                berth_core::PruneKind::BuildCache => {
                    if build_cache_pruned {
                        (Ok(()), None)
                    } else {
                        build_cache_pruned = true;
                        match self
                            .docker_for(CallKind::Build)
                            .prune_build(None::<PruneBuildOptions>)
                            .await
                        {
                            Ok(resp) => (
                                Ok(()),
                                resp.space_reclaimed.map(|n| u64::try_from(n).unwrap_or(0)),
                            ),
                            Err(e) => (Err(self.err(e)), None),
                        }
                    }
                }
            };
            match result {
                Ok(()) => {
                    freed += extra_freed.or_else(|| item.size.map(|s| s.0)).unwrap_or(0);
                    removed.push(item.clone());
                }
                Err(e) => failed.push((item.clone(), e.to_string())),
            }
        }
        Ok(berth_core::PruneReport {
            removed,
            failed,
            freed: bytesize::ByteSize(freed),
        })
    }
}

fn proto_name(p: berth_core::Proto) -> &'static str {
    match p {
        berth_core::Proto::Tcp => "tcp",
        berth_core::Proto::Udp => "udp",
        berth_core::Proto::Sctp => "sctp",
    }
}

/// The endpoint a container gets on `network`: its aliases there (D16).
fn endpoint_for(spec: &berth_core::ContainerSpec, network: &str) -> EndpointSettings {
    EndpointSettings {
        aliases: spec.network_aliases.get(network).cloned(),
        ..Default::default()
    }
}

fn health_config(h: &berth_core::HealthCheck) -> HealthConfig {
    let nanos = |secs: Option<u64>| {
        secs.map(|s| i64::try_from(s.saturating_mul(1_000_000_000)).unwrap_or(i64::MAX))
    };
    HealthConfig {
        test: Some(h.test.clone()),
        interval: nanos(h.interval_s),
        timeout: nanos(h.timeout_s),
        retries: h.retries.map(i64::from),
        start_period: nanos(h.start_period_s),
        start_interval: None,
    }
}

/// Hand-built `HostConfig` + body from a core spec (D40 in the task text:
/// host_config built by hand, no compose machinery involved).
fn spec_to_create_body(spec: &berth_core::ContainerSpec) -> ContainerCreateBody {
    let mut port_bindings: PortMap = HashMap::new();
    let mut exposed_ports = Vec::new();
    for p in &spec.ports {
        let key = format!("{}/{}", p.container_port, proto_name(p.proto));
        exposed_ports.push(key.clone());
        port_bindings.insert(
            key,
            Some(vec![PortBinding {
                host_ip: p.host_ip.map(|ip| ip.to_string()),
                host_port: p.host_port.map(|port| port.to_string()),
            }]),
        );
    }
    let mounts: Vec<Mount> = spec
        .mounts
        .iter()
        .map(|m| {
            let (typ, source) = match m.kind {
                berth_core::MountKind::Bind => (Some(MountType::BIND), Some(m.source.clone())),
                berth_core::MountKind::Volume => (
                    Some(MountType::VOLUME),
                    Some(m.volume_name.clone().unwrap_or_else(|| m.source.clone())),
                ),
                berth_core::MountKind::Tmpfs => (Some(MountType::TMPFS), None),
                berth_core::MountKind::Other => (None, Some(m.source.clone())),
            };
            Mount {
                target: Some(m.destination.clone()),
                source,
                typ,
                read_only: Some(m.read_only),
                ..Default::default()
            }
        })
        .collect();
    let (restart_name, max_retries) = match &spec.restart {
        berth_core::RestartPolicy::No => (RestartPolicyNameEnum::EMPTY, None),
        berth_core::RestartPolicy::Always => (RestartPolicyNameEnum::ALWAYS, None),
        berth_core::RestartPolicy::UnlessStopped => (RestartPolicyNameEnum::UNLESS_STOPPED, None),
        berth_core::RestartPolicy::OnFailure { max_retries } => (
            RestartPolicyNameEnum::ON_FAILURE,
            Some(i64::from(*max_retries)),
        ),
    };
    let networking_config = spec.networks.first().map(|first| NetworkingConfig {
        endpoints_config: Some(
            [(first.clone(), endpoint_for(spec, first))]
                .into_iter()
                .collect(),
        ),
    });
    ContainerCreateBody {
        image: Some(spec.image.reference.clone()),
        working_dir: spec.working_dir.clone(),
        user: spec.user.clone(),
        healthcheck: spec.healthcheck.as_ref().map(health_config),
        cmd: spec.command.clone(),
        entrypoint: spec.entrypoint.clone(),
        env: Some(spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect()),
        labels: Some(
            spec.labels
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        exposed_ports: Some(exposed_ports),
        host_config: Some(HostConfig {
            port_bindings: Some(port_bindings),
            mounts: Some(mounts),
            restart_policy: Some(BollardRestart {
                name: Some(restart_name),
                maximum_retry_count: max_retries,
            }),
            nano_cpus: spec
                .limits
                .cpus
                .map(|c| (f64::from(c) * 1_000_000_000.0) as i64),
            memory: spec
                .limits
                .memory
                .map(|b| i64::try_from(b.0).unwrap_or(i64::MAX)),
            memory_swap: spec
                .limits
                .memory_swap
                .map(|b| i64::try_from(b.0).unwrap_or(i64::MAX)),
            cap_add: (!spec.cap_add.is_empty()).then(|| spec.cap_add.clone()),
            cap_drop: (!spec.cap_drop.is_empty()).then(|| spec.cap_drop.clone()),
            // The first network is the container's own, as `docker run
            // --network` makes it: Docker only applies that endpoint's
            // aliases when the network mode names it.
            network_mode: spec.networks.first().cloned(),
            ..Default::default()
        }),
        networking_config,
        ..Default::default()
    }
}

/// Split "repo:tag" for `create_image`; digests and registry ports stay
/// in the from_image part (only a `:` after the last `/` is a tag).
fn split_image_ref(reference: &str) -> (String, Option<String>) {
    if reference.contains('@') {
        return (reference.to_string(), None);
    }
    let name_part = reference.rsplit('/').next().unwrap_or(reference);
    match name_part.split_once(':') {
        Some((_, tag)) if !tag.is_empty() => {
            let cut = reference.len() - tag.len() - 1;
            (reference[..cut].to_string(), Some(tag.to_string()))
        }
        _ => (reference.to_string(), None),
    }
}

fn channel_stream<T: Send + 'static>(
    rx: tokio::sync::mpsc::Receiver<T>,
) -> futures::stream::BoxStream<'static, T> {
    Box::pin(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }))
}

/// Adapt a bollard stream to `Bs<T>` (D13). Bollard streams only open their
/// connection on first poll, which would miss an event fired between
/// subscribing and the caller's first read; a pump task is spawned now so
/// the subscription is live when this returns. Cancellation ends the stream
/// quietly (no error item) and drops the underlying connection; a bollard
/// error is yielded once, mapped, then the stream ends. Dropping the
/// returned stream also stops the pump.
fn cancellable<S, In, Out, F>(stream: S, cancel: Cancel, mut map: F, engine_name: String) -> Bs<Out>
where
    S: futures::Stream<Item = Result<In, bollard::errors::Error>> + Send + 'static,
    In: Send + 'static,
    Out: Send + 'static,
    F: FnMut(In) -> Out + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(async move {
        let mut stream = Box::pin(stream);
        loop {
            let item = tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                item = stream.next() => item,
            };
            match item {
                Some(Ok(value)) => {
                    if tx.send(Ok(map(value))).await.is_err() {
                        break;
                    }
                }
                Some(Err(err)) => {
                    let _ = tx.send(Err(map_err(&engine_name, err))).await;
                    break;
                }
                None => break,
            }
        }
    });
    channel_stream(rx)
}

/// Split one raw log chunk's text into (complete lines, remainder).
fn split_lines(buffer: &mut String, chunk: &str) -> Vec<String> {
    buffer.push_str(chunk);
    let mut lines = Vec::new();
    while let Some(pos) = buffer.find('\n') {
        let line: String = buffer.drain(..=pos).collect();
        lines.push(
            line.trim_end_matches('\n')
                .trim_end_matches('\r')
                .to_string(),
        );
    }
    lines
}

/// Parse Docker's `timestamps=true` prefix (`<RFC3339Nano> <text>`). Lines
/// without a parseable prefix keep their text and are stamped `fallback`.
fn parse_log_line(
    container: &berth_core::ContainerId,
    stream: berth_core::Stream,
    raw: &str,
    fallback: chrono::DateTime<chrono::Utc>,
) -> berth_core::LogLine {
    if let Some((stamp, text)) = raw.split_once(' ') {
        if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(stamp) {
            return berth_core::LogLine {
                container: container.clone(),
                ts: parsed.with_timezone(&chrono::Utc),
                stream,
                text: text.to_string(),
            };
        }
    }
    berth_core::LogLine {
        container: container.clone(),
        ts: fallback,
        stream,
        text: raw.to_string(),
    }
}

/// Demux a bollard log stream into `LogLine`s, one per line, through the
/// same eager pump as `cancellable`. Chunks are split on newlines with the
/// partial tail kept per stream; a final unterminated line is flushed when
/// the engine stream ends.
fn log_stream<S>(
    stream: S,
    container: berth_core::ContainerId,
    cancel: Cancel,
    engine_name: String,
) -> Bs<berth_core::LogLine>
where
    S: futures::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<berth_core::LogLine, EngineError>>(256);
    tokio::spawn(async move {
        let mut stream = Box::pin(stream);
        let mut stdout_rest = String::new();
        let mut stderr_rest = String::new();
        'pump: loop {
            let next = tokio::select! {
                biased;
                () = cancel.cancelled() => break 'pump,
                item = stream.next() => item,
            };
            match next {
                Some(Ok(output)) => {
                    let (kind, message) = match output {
                        LogOutput::StdErr { message } => (berth_core::Stream::Stderr, message),
                        LogOutput::StdOut { message }
                        | LogOutput::StdIn { message }
                        | LogOutput::Console { message } => (berth_core::Stream::Stdout, message),
                    };
                    let text = String::from_utf8_lossy(&message);
                    let rest = match kind {
                        berth_core::Stream::Stdout => &mut stdout_rest,
                        berth_core::Stream::Stderr => &mut stderr_rest,
                    };
                    let now = chrono::Utc::now();
                    for line in split_lines(rest, &text) {
                        let parsed = parse_log_line(&container, kind, &line, now);
                        if tx.send(Ok(parsed)).await.is_err() {
                            break 'pump;
                        }
                    }
                }
                Some(Err(err)) => {
                    let _ = tx.send(Err(map_err(&engine_name, err))).await;
                    break 'pump;
                }
                None => {
                    let now = chrono::Utc::now();
                    for (kind, rest) in [
                        (berth_core::Stream::Stdout, stdout_rest),
                        (berth_core::Stream::Stderr, stderr_rest),
                    ] {
                        if !rest.is_empty()
                            && tx
                                .send(Ok(parse_log_line(&container, kind, &rest, now)))
                                .await
                                .is_err()
                        {
                            break 'pump;
                        }
                    }
                    break 'pump;
                }
            }
        }
    });
    channel_stream(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fallback() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(7, 0).expect("ts")
    }

    #[test]
    fn helper_create_config_has_the_helper_label_not_the_fixture_label() {
        let name = helper_name();
        assert!(name.starts_with("berth-helper-"), "{name}");
        assert_ne!(name, helper_name(), "every helper gets its own name");
        let body = helper_create_body(&helper_spec(&name, "some-volume", true));
        let labels = body.labels.expect("labels");
        assert_eq!(
            labels.get("dev.berth.helper").map(String::as_str),
            Some("1")
        );
        assert!(!labels.contains_key("dev.berth.fixture"), "{labels:?}");
        let host = body.host_config.expect("host config");
        assert_eq!(host.auto_remove, Some(true));
        let mounts = host.mounts.expect("mounts");
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].read_only, Some(true));
    }

    #[test]
    fn parse_log_line_strips_rfc3339_timestamp_prefix() {
        let line = parse_log_line(
            &berth_core::ContainerId("c1".to_string()),
            berth_core::Stream::Stderr,
            "2026-10-24T10:00:00.123456789Z ERROR boom",
            fallback(),
        );
        assert_eq!(line.text, "ERROR boom");
        assert_eq!(line.stream, berth_core::Stream::Stderr);
        assert_eq!(line.ts.timestamp(), 1_792_836_000);
        assert_eq!(line.ts.timestamp_subsec_nanos(), 123_456_789);
    }

    #[test]
    fn parse_log_line_keeps_unstamped_text_with_fallback() {
        let line = parse_log_line(
            &berth_core::ContainerId("c1".to_string()),
            berth_core::Stream::Stdout,
            "plain line with spaces",
            fallback(),
        );
        assert_eq!(line.text, "plain line with spaces");
        assert_eq!(line.ts, fallback());
    }

    #[test]
    fn split_lines_buffers_partial_tail_across_chunks() {
        let mut buffer = String::new();
        assert_eq!(
            split_lines(&mut buffer, "one\ntwo par"),
            ["one".to_string()]
        );
        assert_eq!(
            split_lines(&mut buffer, "tial\nthree\r\n"),
            ["two partial".to_string(), "three".to_string()]
        );
        assert!(buffer.is_empty());
    }

    #[tokio::test]
    async fn cancellable_ends_quietly_on_cancel() {
        let cancel = Cancel::new();
        let pending = futures::stream::pending::<Result<i32, bollard::errors::Error>>();
        let mut stream = cancellable(pending, cancel.clone(), |n| n * 2, "test".to_string());
        cancel.cancel();
        assert!(stream.next().await.is_none());
    }

    fn sample_spec() -> berth_core::ContainerSpec {
        berth_core::ContainerSpec {
            name: "berthfx-scratch-unit".to_string(),
            image: berth_core::ImageRef {
                reference: "alpine:3".to_string(),
                id: None,
            },
            command: Some(vec!["sleep".to_string(), "60".to_string()]),
            entrypoint: None,
            env: vec![("FOO".to_string(), "bar".to_string())],
            ports: vec![berth_core::PortMapping {
                host_ip: None,
                host_port: Some(56123),
                container_port: 8080,
                proto: berth_core::Proto::Tcp,
            }],
            mounts: vec![berth_core::Mount {
                kind: berth_core::MountKind::Volume,
                source: "data-src".to_string(),
                destination: "/data".to_string(),
                read_only: true,
                volume_name: Some("berthfx-vol-unit".to_string()),
            }],
            networks: vec!["berthfx-net-unit".to_string(), "bridge".to_string()],
            labels: [("dev.berth.fixture".to_string(), "1".to_string())]
                .into_iter()
                .collect(),
            restart: berth_core::RestartPolicy::OnFailure { max_retries: 3 },
            limits: berth_core::Limits {
                cpus: Some(0.5),
                memory: Some(bytesize::ByteSize(268_435_456)),
                memory_swap: None,
            },
            platform: None,
            working_dir: Some("/srv/app".to_string()),
            user: Some("1000:1000".to_string()),
            healthcheck: Some(berth_core::HealthCheck {
                test: vec!["CMD".to_string(), "true".to_string()],
                interval_s: Some(30),
                timeout_s: Some(3),
                retries: Some(5),
                start_period_s: Some(10),
            }),
            cap_add: vec!["NET_ADMIN".to_string()],
            cap_drop: vec!["MKNOD".to_string()],
            network_aliases: [("berthfx-net-unit".to_string(), vec!["svc".to_string()])]
                .into_iter()
                .collect(),
        }
    }

    /// D16: detail (the inspect mapper) to spec to the create body keeps
    /// every run setting.
    #[test]
    fn spec_to_create_body_carries_the_run_settings_from_an_inspect() {
        let inspect: bollard::models::ContainerInspectResponse =
            serde_json::from_value(serde_json::json!({
                "Id": "0123456789abcdef0123456789abcdef",
                "Name": "/web",
                "State": { "Status": "running" },
                "Config": {
                    "Image": "nginx:alpine",
                    "User": "1000:1000",
                    "WorkingDir": "/srv/app",
                    "Cmd": ["nginx", "-g", "daemon off;"],
                    "Env": ["A=1"],
                    "Healthcheck": {
                        "Test": ["CMD", "curl", "-f", "http://localhost"],
                        "Interval": 30_000_000_000_i64,
                        "Timeout": 3_000_000_000_i64,
                        "Retries": 5,
                        "StartPeriod": 10_000_000_000_i64
                    }
                },
                "HostConfig": {
                    "CapAdd": ["NET_ADMIN"],
                    "CapDrop": ["MKNOD"],
                    "RestartPolicy": { "Name": "unless-stopped" }
                },
                "NetworkSettings": { "Networks": {
                    "shop_default": { "Aliases": ["web", "0123456789ab"] }
                } }
            }))
            .expect("an inspect response");
        let detail = crate::mappers::detail(&berth_core::EngineId("e".to_string()), &inspect);
        let spec = berth_core::ContainerSpec::from_detail(&detail);
        let body = spec_to_create_body(&spec);
        assert_eq!(body.user.as_deref(), Some("1000:1000"));
        assert_eq!(body.working_dir.as_deref(), Some("/srv/app"));
        let health = body.healthcheck.expect("healthcheck");
        assert_eq!(
            health.test,
            Some(vec![
                "CMD".to_string(),
                "curl".to_string(),
                "-f".to_string(),
                "http://localhost".to_string()
            ])
        );
        assert_eq!(health.interval, Some(30_000_000_000));
        assert_eq!(health.timeout, Some(3_000_000_000));
        assert_eq!(health.retries, Some(5));
        assert_eq!(health.start_period, Some(10_000_000_000));
        let host = body.host_config.expect("host config");
        assert_eq!(host.cap_add, Some(vec!["NET_ADMIN".to_string()]));
        assert_eq!(host.cap_drop, Some(vec!["MKNOD".to_string()]));
        // The first network is the container's network mode, or Docker
        // would ignore its aliases.
        assert_eq!(host.network_mode.as_deref(), Some("shop_default"));
        // The aliases ride on the first network's endpoint; the container's
        // own short id (Docker adds it) does not.
        let endpoints = body
            .networking_config
            .expect("networking")
            .endpoints_config
            .expect("endpoints");
        assert_eq!(
            endpoints
                .get("shop_default")
                .and_then(|e| e.aliases.clone()),
            Some(vec!["web".to_string()])
        );
        assert_eq!(
            endpoint_for(&spec, "other").aliases,
            None,
            "a network without aliases gets none"
        );
    }

    #[test]
    fn spec_to_create_body_maps_env_ports_mounts_limits_and_restart() {
        let body = spec_to_create_body(&sample_spec());
        assert_eq!(body.image.as_deref(), Some("alpine:3"));
        assert_eq!(body.cmd, Some(vec!["sleep".to_string(), "60".to_string()]));
        assert_eq!(body.env, Some(vec!["FOO=bar".to_string()]));
        assert_eq!(body.exposed_ports, Some(vec!["8080/tcp".to_string()]));
        let labels = body.labels.expect("labels");
        assert_eq!(
            labels.get("dev.berth.fixture").map(String::as_str),
            Some("1")
        );
        let host = body.host_config.expect("host config");
        let bindings = host.port_bindings.expect("port bindings");
        let binding = bindings
            .get("8080/tcp")
            .expect("8080 binding")
            .as_ref()
            .expect("binding list")
            .first()
            .expect("one binding");
        assert_eq!(binding.host_port.as_deref(), Some("56123"));
        let mount = host.mounts.expect("mounts").pop().expect("one mount");
        assert_eq!(mount.target.as_deref(), Some("/data"));
        assert_eq!(mount.source.as_deref(), Some("berthfx-vol-unit"));
        assert_eq!(mount.read_only, Some(true));
        assert_eq!(host.nano_cpus, Some(500_000_000));
        assert_eq!(host.memory, Some(268_435_456));
        let restart = host.restart_policy.expect("restart policy");
        assert_eq!(restart.maximum_retry_count, Some(3));
        let endpoints = body
            .networking_config
            .expect("networking")
            .endpoints_config
            .expect("endpoints");
        assert!(endpoints.contains_key("berthfx-net-unit"));
        assert_eq!(endpoints.len(), 1, "only the first network rides in create");
    }

    #[test]
    fn split_image_ref_handles_tag_registry_port_and_digest() {
        assert_eq!(
            split_image_ref("alpine:3"),
            ("alpine".to_string(), Some("3".to_string()))
        );
        assert_eq!(split_image_ref("alpine"), ("alpine".to_string(), None));
        assert_eq!(
            split_image_ref("127.0.0.1:56500/berthfx/app:1"),
            (
                "127.0.0.1:56500/berthfx/app".to_string(),
                Some("1".to_string())
            )
        );
        assert_eq!(
            split_image_ref("127.0.0.1:56500/berthfx/app"),
            ("127.0.0.1:56500/berthfx/app".to_string(), None)
        );
        assert_eq!(
            split_image_ref("alpine@sha256:abc"),
            ("alpine@sha256:abc".to_string(), None)
        );
    }

    #[tokio::test]
    async fn cancellable_maps_items_and_ends_with_source() {
        let source = futures::stream::iter(vec![Ok(1), Ok(2)]);
        let stream = cancellable(source, Cancel::new(), |n| n * 10, "test".to_string());
        let got: Vec<i32> = stream
            .map(|r| r.expect("ok item"))
            .collect::<Vec<_>>()
            .await;
        assert_eq!(got, [10, 20]);
    }
}
