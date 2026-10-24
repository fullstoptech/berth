//! `BollardEngine`: the read side of `EngineClient` over a unix socket
//! (P1-T4). Streams and actions land in P1-T5, mutations in P1-T6.

use bollard::container::LogOutput;
use bollard::models::{
    ContainerUpdateBody, RestartPolicy as BollardRestart, RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    DataUsageOptions, InspectContainerOptionsBuilder, KillContainerOptionsBuilder,
    ListContainersOptionsBuilder, ListImagesOptionsBuilder, LogsOptionsBuilder,
    RemoveContainerOptionsBuilder, RenameContainerOptionsBuilder, RestartContainerOptionsBuilder,
    StartContainerOptions, StatsOptionsBuilder, StopContainerOptionsBuilder,
};
use futures::StreamExt;
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

    fn events(&self, cancel: Cancel) -> Bs<berth_core::EngineEvent> {
        let docker = self.docker.clone();
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
        let docker = self.docker.clone();
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
        let docker = self.docker.clone();
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
