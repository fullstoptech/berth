//! `BollardEngine`: the read side of `EngineClient` over a unix socket
//! (P1-T4). Streams and actions land in P1-T5, mutations in P1-T6.

use bollard::query_parameters::{
    DataUsageOptions, InspectContainerOptionsBuilder, ListContainersOptionsBuilder,
    ListImagesOptionsBuilder,
};
use tokio_util::sync::CancellationToken as Cancel;

use crate::client::{unsupported_stream, Bs, EngineClient, ExecHandle};
use crate::mappers;
use crate::EngineError;

/// Bollard's per-request timeout installed by `connect_with_unix` (seconds).
/// PLAN §6.5 asks for a 2 s connect timeout; bollard has no separate connect
/// timeout for unix sockets, so the request timeout carries the 2 s budget.
const CONNECT_TIMEOUT_SECS: u64 = 2;

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
        B::RequestTimeoutError => {
            EngineError::Timeout(std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS))
        }
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
            CONNECT_TIMEOUT_SECS,
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
            .docker
            .df(None::<DataUsageOptions>)
            .await
            .map_err(|e| self.err(e))?;
        Ok(mappers::disk_usage(&df))
    }

    async fn volume_sizes(
        &self,
    ) -> Result<std::collections::BTreeMap<String, bytesize::ByteSize>, EngineError> {
        let df = self
            .docker
            .df(None::<DataUsageOptions>)
            .await
            .map_err(|e| self.err(e))?;
        Ok(mappers::volume_sizes(&df))
    }

    fn events(&self, _cancel: Cancel) -> Bs<berth_core::EngineEvent> {
        unsupported_stream("events") // P1-T5
    }

    fn logs(
        &self,
        _id: &berth_core::ContainerId,
        _q: berth_core::LogQuery,
        _cancel: Cancel,
    ) -> Bs<berth_core::LogLine> {
        unsupported_stream("logs") // P1-T5
    }

    fn stats(&self, _id: &berth_core::ContainerId, _cancel: Cancel) -> Bs<berth_core::LiveStats> {
        unsupported_stream("stats") // P1-T5
    }

    async fn act(
        &self,
        _id: &berth_core::ContainerId,
        _action: berth_core::ContainerAction,
    ) -> Result<(), EngineError> {
        Err(EngineError::Unsupported("act".into())) // P1-T5
    }

    async fn update_limits(
        &self,
        _id: &berth_core::ContainerId,
        _limits: &berth_core::Limits,
        _restart: &berth_core::RestartPolicy,
    ) -> Result<(), EngineError> {
        Err(EngineError::Unsupported("update_limits".into())) // P1-T5
    }

    async fn create(
        &self,
        _spec: &berth_core::ContainerSpec,
        _start: bool,
    ) -> Result<berth_core::ContainerId, EngineError> {
        Err(EngineError::Unsupported("create".into())) // P1-T6
    }

    async fn recreate(
        &self,
        _id: &berth_core::ContainerId,
        _spec: &berth_core::ContainerSpec,
    ) -> Result<berth_core::ContainerId, EngineError> {
        Err(EngineError::Unsupported("recreate".into())) // P1-T6
    }

    async fn exec(
        &self,
        _id: &berth_core::ContainerId,
        _req: berth_core::ExecRequest,
        _cancel: Cancel,
    ) -> Result<ExecHandle, EngineError> {
        Err(EngineError::Unsupported("exec".into())) // P1-T6
    }

    async fn list_files(
        &self,
        _target: &berth_core::FileTarget,
        _path: &str,
    ) -> Result<Vec<berth_core::FileEntry>, EngineError> {
        Err(EngineError::Unsupported("list_files".into())) // P1-T6
    }

    fn download(
        &self,
        _target: &berth_core::FileTarget,
        _path: &str,
        _cancel: Cancel,
    ) -> Bs<bytes::Bytes> {
        unsupported_stream("download") // P1-T6
    }

    async fn upload(
        &self,
        _target: &berth_core::FileTarget,
        _dir: &str,
        _tar: bytes::Bytes,
    ) -> Result<(), EngineError> {
        Err(EngineError::Unsupported("upload".into())) // P1-T6
    }

    fn pull(
        &self,
        _reference: &str,
        _platform: Option<&str>,
        _cancel: Cancel,
    ) -> Bs<berth_core::PullProgress> {
        unsupported_stream("pull") // P1-T6
    }

    async fn remove_image(&self, _id: &str, _force: bool) -> Result<(), EngineError> {
        Err(EngineError::Unsupported("remove_image".into())) // P1-T6
    }

    async fn search_images(
        &self,
        _term: &str,
        _limit: u32,
    ) -> Result<Vec<berth_core::ImageSearchHit>, EngineError> {
        Err(EngineError::Unsupported("search_images".into())) // P1-T6
    }

    async fn create_volume(
        &self,
        _spec: &berth_core::VolumeSpec,
    ) -> Result<berth_core::Volume, EngineError> {
        Err(EngineError::Unsupported("create_volume".into())) // P1-T6
    }

    async fn remove_volume(&self, _name: &str) -> Result<(), EngineError> {
        Err(EngineError::Unsupported("remove_volume".into())) // P1-T6
    }

    async fn create_network(
        &self,
        _spec: &berth_core::NetworkSpec,
    ) -> Result<berth_core::Network, EngineError> {
        Err(EngineError::Unsupported("create_network".into())) // P1-T6
    }

    async fn remove_network(&self, _id: &str) -> Result<(), EngineError> {
        Err(EngineError::Unsupported("remove_network".into())) // P1-T6
    }

    async fn connect_network(
        &self,
        _network: &str,
        _id: &berth_core::ContainerId,
    ) -> Result<(), EngineError> {
        Err(EngineError::Unsupported("connect_network".into())) // P1-T6
    }

    async fn disconnect_network(
        &self,
        _network: &str,
        _id: &berth_core::ContainerId,
    ) -> Result<(), EngineError> {
        Err(EngineError::Unsupported("disconnect_network".into())) // P1-T6
    }

    async fn prune(
        &self,
        _plan: &berth_core::PrunePlan,
        _cancel: Cancel,
    ) -> Result<berth_core::PruneReport, EngineError> {
        Err(EngineError::Unsupported("prune".into())) // P1-T6
    }
}
