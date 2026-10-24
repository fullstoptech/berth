//! The `EngineClient` trait (PLAN §6.5), exactly as specified. Read-side
//! methods are implemented by `BollardEngine` in P1-T4; the rest return
//! `EngineError::Unsupported` until P1-T5/P1-T6.

use tokio_util::sync::CancellationToken as Cancel;

use crate::EngineError;

pub struct ExecHandle {
    pub session: berth_core::ExecSession,
    pub output: futures::stream::BoxStream<'static, Result<bytes::Bytes, EngineError>>,
    pub input: tokio::sync::mpsc::Sender<bytes::Bytes>,
    resize: tokio::sync::mpsc::Sender<(u16, u16)>,
}

impl ExecHandle {
    pub async fn resize(&self, cols: u16, rows: u16) -> Result<(), EngineError> {
        self.resize
            .send((cols, rows))
            .await
            .map_err(|_| EngineError::Docker("exec session is closed".to_string()))
    }
}

pub type Bs<T> = futures::stream::BoxStream<'static, Result<T, EngineError>>;

pub struct RegistryAuth {
    pub server: String,
    pub username: String,
    pub password: secrecy::SecretString,
}

pub fn unsupported_stream<T: Send + 'static>(what: &str) -> Bs<T> {
    let what = what.to_string();
    Box::pin(futures::stream::once(async move {
        Err(EngineError::Unsupported(what))
    }))
}

#[async_trait::async_trait]
pub trait EngineClient: Send + Sync {
    fn id(&self) -> &berth_core::EngineId;
    fn docker_host(&self) -> &str;
    async fn info(&self) -> Result<berth_core::EngineInfo, EngineError>;
    async fn containers(&self) -> Result<Vec<berth_core::Container>, EngineError>; // all states
    async fn container(
        &self,
        id: &berth_core::ContainerId,
    ) -> Result<berth_core::ContainerDetail, EngineError>;
    async fn images(&self) -> Result<Vec<berth_core::Image>, EngineError>;
    async fn image(&self, id: &str) -> Result<berth_core::ImageDetail, EngineError>;
    async fn volumes(&self) -> Result<Vec<berth_core::Volume>, EngineError>; // size None until disk_usage
    async fn networks(&self) -> Result<Vec<berth_core::Network>, EngineError>;
    async fn disk_usage(&self) -> Result<berth_core::DiskUsage, EngineError>;
    async fn volume_sizes(
        &self,
    ) -> Result<std::collections::BTreeMap<String, bytesize::ByteSize>, EngineError>; // from system df
    fn events(&self, cancel: Cancel) -> Bs<berth_core::EngineEvent>;
    fn logs(
        &self,
        id: &berth_core::ContainerId,
        q: berth_core::LogQuery,
        cancel: Cancel,
    ) -> Bs<berth_core::LogLine>;
    fn stats(&self, id: &berth_core::ContainerId, cancel: Cancel) -> Bs<berth_core::LiveStats>;
    async fn act(
        &self,
        id: &berth_core::ContainerId,
        action: berth_core::ContainerAction,
    ) -> Result<(), EngineError>;
    async fn update_limits(
        &self,
        id: &berth_core::ContainerId,
        limits: &berth_core::Limits,
        restart: &berth_core::RestartPolicy,
    ) -> Result<(), EngineError>;
    async fn create(
        &self,
        spec: &berth_core::ContainerSpec,
        start: bool,
    ) -> Result<berth_core::ContainerId, EngineError>;
    /// Stop, rename old to "<name>-berth-old", create, start, remove old. On any failure the old container is renamed back and restarted.
    async fn recreate(
        &self,
        id: &berth_core::ContainerId,
        spec: &berth_core::ContainerSpec,
    ) -> Result<berth_core::ContainerId, EngineError>;
    async fn exec(
        &self,
        id: &berth_core::ContainerId,
        req: berth_core::ExecRequest,
        cancel: Cancel,
    ) -> Result<ExecHandle, EngineError>;
    async fn list_files(
        &self,
        target: &berth_core::FileTarget,
        path: &str,
    ) -> Result<Vec<berth_core::FileEntry>, EngineError>;
    fn download(
        &self,
        target: &berth_core::FileTarget,
        path: &str,
        cancel: Cancel,
    ) -> Bs<bytes::Bytes>; // tar stream
    async fn upload(
        &self,
        target: &berth_core::FileTarget,
        dir: &str,
        tar: bytes::Bytes,
    ) -> Result<(), EngineError>;
    fn pull(
        &self,
        reference: &str,
        platform: Option<&str>,
        cancel: Cancel,
    ) -> Bs<berth_core::PullProgress>;
    async fn remove_image(&self, id: &str, force: bool) -> Result<(), EngineError>;
    async fn search_images(
        &self,
        term: &str,
        limit: u32,
    ) -> Result<Vec<berth_core::ImageSearchHit>, EngineError>;
    async fn create_volume(
        &self,
        spec: &berth_core::VolumeSpec,
    ) -> Result<berth_core::Volume, EngineError>;
    async fn remove_volume(&self, name: &str) -> Result<(), EngineError>;
    async fn create_network(
        &self,
        spec: &berth_core::NetworkSpec,
    ) -> Result<berth_core::Network, EngineError>;
    async fn remove_network(&self, id: &str) -> Result<(), EngineError>;
    async fn connect_network(
        &self,
        network: &str,
        id: &berth_core::ContainerId,
    ) -> Result<(), EngineError>;
    async fn disconnect_network(
        &self,
        network: &str,
        id: &berth_core::ContainerId,
    ) -> Result<(), EngineError>;
    async fn prune(
        &self,
        plan: &berth_core::PrunePlan,
        cancel: Cancel,
    ) -> Result<berth_core::PruneReport, EngineError>;
    // M4 (P8). Default bodies return EngineError::Unsupported until P8-T3/T4 implement them.
    async fn tag_image(&self, _id: &str, _repo: &str, _tag: &str) -> Result<(), EngineError> {
        Err(EngineError::Unsupported("tag".into()))
    }
    fn push(
        &self,
        _reference: &str,
        _auth: Option<RegistryAuth>,
        _cancel: Cancel,
    ) -> Bs<berth_core::PullProgress> {
        unsupported_stream("push")
    }
    fn export_image(&self, _id: &str, _cancel: Cancel) -> Bs<bytes::Bytes> {
        unsupported_stream("export")
    }
    async fn import_image(&self, _tar: bytes::Bytes) -> Result<Vec<String>, EngineError> {
        Err(EngineError::Unsupported("import".into()))
    }
    async fn commit(
        &self,
        _id: &berth_core::ContainerId,
        _repo: &str,
        _tag: &str,
    ) -> Result<String, EngineError> {
        Err(EngineError::Unsupported("commit".into()))
    }
}
