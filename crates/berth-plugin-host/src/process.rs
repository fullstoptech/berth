//! Process supervision: spawn the plugin, run the `initialize`
//! handshake, route requests and notifications, and restart on crash
//! with backoff (PLAN §6.10, P9-T2).
//!
//! Every child is spawned with `kill_on_drop(true)` and the supplied
//! `CancellationToken` ends the supervisor (clean `shutdown` first, then
//! kill). A request already sent to a generation that dies fails with
//! [`PluginError::Exited`] — actions are at-most-once, never replayed. A
//! request made while a restart is in flight waits (bounded by its own
//! timeout) for the next generation.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use berth_plugin_protocol::{decode, encode, methods, Message, Notification, Request};
use berth_store::SecretStore;
use berth_sys::SearchPath;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot, Notify};
use tokio_util::sync::CancellationToken;

use crate::dispatch;
use crate::{InstalledPlugin, PluginError, PluginState};

/// How long a plugin may take to answer `initialize`.
pub const INIT_TIMEOUT: Duration = Duration::from_secs(10);
/// Grace period between `shutdown` and killing the child.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const NOTIFICATION_CAPACITY: usize = 256;
const STDERR_TAIL_LINES: usize = 100;

/// Restart delays between generations. The default is the P9-T2 schedule
/// (1 s, 5 s, 30 s, then Error); tests inject shorter delays through
/// [`PluginHandle::start_with_backoff`] so nothing sleeps for 30 s. At
/// most `delays.len()` restarts happen per `start`; the app starts the
/// plugin again explicitly after that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backoff {
    pub delays: Vec<Duration>,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            delays: vec![
                Duration::from_secs(1),
                Duration::from_secs(5),
                Duration::from_secs(30),
            ],
        }
    }
}

type Pending = HashMap<u64, oneshot::Sender<Result<serde_json::Value, PluginError>>>;

struct Shared {
    plugin_id: String,
    pending: Mutex<Pending>,
    endpoint: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    endpoint_changed: Notify,
    state: Mutex<PluginState>,
    init_result: Mutex<Option<serde_json::Value>>,
    stderr_tail: Mutex<VecDeque<String>>,
    restarts: AtomicU32,
    next_id: AtomicU64,
    notifications: broadcast::Sender<Notification>,
    dispatch: dispatch::Context,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Shared {
    fn set_state(&self, state: PluginState) {
        *lock(&self.state) = state;
        self.endpoint_changed.notify_waiters();
    }

    fn set_endpoint(&self, endpoint: Option<mpsc::Sender<Vec<u8>>>) {
        *lock(&self.endpoint) = endpoint;
        self.endpoint_changed.notify_waiters();
    }

    fn fail_pending(&self, reason: &str) {
        let pending: Vec<_> = lock(&self.pending).drain().collect();
        for (_, sender) in pending {
            let _ = sender.send(Err(PluginError::Exited(reason.to_string())));
        }
    }

    /// Wait for the current generation's stdin, or fail when the plugin
    /// is stopped, in Error, cancelled, or out of time.
    async fn await_endpoint(
        &self,
        deadline: tokio::time::Instant,
        cancel: &CancellationToken,
    ) -> Result<mpsc::Sender<Vec<u8>>, PluginError> {
        loop {
            let notified = self.endpoint_changed.notified();
            tokio::pin!(notified);
            let _ = notified.as_mut().enable();
            if let Some(endpoint) = lock(&self.endpoint).clone() {
                return Ok(endpoint);
            }
            match &*lock(&self.state) {
                PluginState::Error { reason } => {
                    return Err(PluginError::Exited(reason.clone()));
                }
                PluginState::Disabled => {
                    return Err(PluginError::Exited("plugin is stopped".into()));
                }
                PluginState::Enabled => {}
            }
            if cancel.is_cancelled() {
                return Err(PluginError::Exited("cancelled".into()));
            }
            tokio::select! {
                _ = notified => {}
                _ = cancel.cancelled() => {
                    return Err(PluginError::Exited("cancelled".into()));
                }
                _ = tokio::time::sleep_until(deadline) => {
                    return Err(PluginError::Timeout);
                }
            }
        }
    }

    async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<serde_json::Value, PluginError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let endpoint = self.await_endpoint(deadline, cancel).await?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, tx);
        let request = Request {
            jsonrpc: "2.0".into(),
            id,
            method: method.to_string(),
            params,
        };
        if endpoint.send(encode(&request)).await.is_err() {
            lock(&self.pending).remove(&id);
            return Err(PluginError::Exited("plugin closed its stdin".into()));
        }
        tokio::select! {
            result = rx => result.unwrap_or_else(|_| Err(PluginError::Exited("plugin went away".into()))),
            _ = cancel.cancelled() => {
                lock(&self.pending).remove(&id);
                Err(PluginError::Exited("cancelled".into()))
            }
            _ = tokio::time::sleep_until(deadline) => {
                lock(&self.pending).remove(&id);
                Err(PluginError::Timeout)
            }
        }
    }
}

/// One running child: exit notification plus a kill switch.
struct Generation {
    exit_rx: oneshot::Receiver<String>,
    kill_tx: Option<oneshot::Sender<()>>,
}

impl Generation {
    /// Ask this generation to stop now and wait briefly for it to die.
    async fn kill(&mut self) {
        if let Some(kill_tx) = self.kill_tx.take() {
            let _ = kill_tx.send(());
        }
        let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut self.exit_rx).await;
    }
}

fn resolve_executable(
    plugin: &InstalledPlugin,
    path: &std::ffi::OsStr,
) -> Result<std::path::PathBuf, PluginError> {
    // A manifest shipped as a sealed resource of the app bundle names an
    // executable in the bundle's Contents/MacOS; nothing runs from Resources
    // (AD70).
    if let Some(macos) = crate::discover::bundle_macos_dir(&plugin.dir) {
        let in_macos = macos.join(&plugin.manifest.executable);
        return if in_macos.is_file() {
            Ok(in_macos)
        } else {
            Err(PluginError::Exited(format!(
                "executable not found: {}",
                in_macos.display()
            )))
        };
    }
    let joined = plugin.dir.join(&plugin.manifest.executable);
    if joined.is_file() {
        return Ok(joined);
    }
    // Bare names resolve through the same PATH the child will run with.
    if !plugin.manifest.executable.contains('/') {
        let search = SearchPath::from_dirs(std::env::split_paths(path).collect());
        if let Some(found) = search.find(&plugin.manifest.executable) {
            return Ok(found);
        }
    }
    Err(PluginError::Exited(format!(
        "executable not found: {}",
        joined.display()
    )))
}

fn spawn_generation(
    shared: &Arc<Shared>,
    executable: &std::path::Path,
    dir: &std::path::Path,
    path: &std::ffi::OsStr,
) -> Result<Generation, PluginError> {
    let mut command = tokio::process::Command::new(executable);
    command
        .current_dir(dir)
        .env("PATH", path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| {
        PluginError::Exited(format!("could not start {}: {e}", executable.display()))
    })?;
    let Some(mut stdin) = child.stdin.take() else {
        return Err(PluginError::Exited("child has no stdin".into()));
    };
    let Some(stdout) = child.stdout.take() else {
        return Err(PluginError::Exited("child has no stdout".into()));
    };
    let stderr = child.stderr.take();

    let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(64);
    tokio::spawn(async move {
        while let Some(bytes) = write_rx.recv().await {
            if stdin.write_all(&bytes).await.is_err() {
                return;
            }
        }
    });

    {
        let shared = Arc::clone(shared);
        let write_tx = write_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => match decode(line.as_bytes()) {
                        Ok(Message::Response(response)) => {
                            if let Some(sender) = lock(&shared.pending).remove(&response.id) {
                                let result = if let Some(error) = response.error {
                                    Err(PluginError::Rpc(format!(
                                        "{} [{}]",
                                        error.message, error.code
                                    )))
                                } else {
                                    Ok(response.result.unwrap_or(serde_json::Value::Null))
                                };
                                let _ = sender.send(result);
                            }
                        }
                        Ok(Message::Request(request)) => {
                            let response = dispatch::handle_request(&shared.dispatch, &request);
                            if write_tx.send(encode(&response)).await.is_err() {
                                return;
                            }
                        }
                        Ok(Message::Notification(notification)) => {
                            if notification.method == methods::LOG {
                                dispatch::handle_log(&shared.plugin_id, &notification.params);
                            } else {
                                let _ = shared.notifications.send(notification);
                            }
                        }
                        // Never log the line itself: it could carry a
                        // secret value the plugin echoed back.
                        Err(_) => {
                            tracing::warn!(
                                plugin = %shared.plugin_id,
                                "plugin sent a line that is not protocol JSON; ignored"
                            );
                        }
                    },
                    _ => return,
                }
            }
        });
    }

    if let Some(stderr) = stderr {
        let shared = Arc::clone(shared);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        tracing::debug!(plugin = %shared.plugin_id, stderr = %line);
                        let mut tail = lock(&shared.stderr_tail);
                        if tail.len() >= STDERR_TAIL_LINES {
                            tail.pop_front();
                        }
                        tail.push_back(line);
                    }
                    _ => return,
                }
            }
        });
    }

    let (exit_tx, exit_rx) = oneshot::channel();
    let (kill_tx, mut kill_rx) = oneshot::channel::<()>();
    {
        let shared = Arc::clone(shared);
        tokio::spawn(async move {
            let reason = tokio::select! {
                waited = child.wait() => match waited {
                    Ok(status) => format!("exit status {status}"),
                    Err(e) => format!("wait failed: {e}"),
                },
                _ = &mut kill_rx => {
                    let _ = child.start_kill();
                    match child.wait().await {
                        Ok(status) => format!("killed ({status})"),
                        Err(e) => format!("wait failed: {e}"),
                    }
                }
            };
            // Fail in-flight requests before telling the supervisor, so a
            // crash during a request (including initialize) is prompt.
            // The supervisor spawns the next generation only after this
            // exit, so clearing the endpoint here cannot race it.
            shared.set_endpoint(None);
            shared.fail_pending(&reason);
            let _ = exit_tx.send(reason);
        });
    }

    shared.set_endpoint(Some(write_tx));
    Ok(Generation {
        exit_rx,
        kill_tx: Some(kill_tx),
    })
}

async fn sleep_or_cancel(delay: Duration, cancel: &CancellationToken) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => true,
        _ = cancel.cancelled() => false,
    }
}

/// Send `shutdown`, give the child a grace period, then kill it.
async fn shutdown_generation(shared: &Arc<Shared>, generation: &mut Generation) {
    let notice = Request {
        jsonrpc: "2.0".into(),
        id: shared.next_id.fetch_add(1, Ordering::Relaxed),
        method: methods::SHUTDOWN.into(),
        params: serde_json::json!({}),
    };
    let endpoint = lock(&shared.endpoint).clone();
    if let Some(endpoint) = endpoint {
        let _ = endpoint.send(encode(&notice)).await;
    }
    shared.set_endpoint(None);
    if tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut generation.exit_rx)
        .await
        .is_err()
    {
        generation.kill().await;
    }
}

struct SupervisorJob {
    executable: std::path::PathBuf,
    dir: std::path::PathBuf,
    path: std::ffi::OsString,
    config: serde_json::Value,
    backoff: Backoff,
}

#[allow(clippy::too_many_lines)]
async fn supervise(
    shared: Arc<Shared>,
    job: SupervisorJob,
    cancel: CancellationToken,
    mut first_init: Option<oneshot::Sender<Result<(), PluginError>>>,
) {
    let mut failures: u32 = 0;
    loop {
        if cancel.is_cancelled() {
            if let Some(tx) = first_init.take() {
                let _ = tx.send(Err(PluginError::Exited("cancelled".into())));
            }
            shared.set_state(PluginState::Disabled);
            return;
        }
        let mut generation = match spawn_generation(&shared, &job.executable, &job.dir, &job.path) {
            Ok(generation) => generation,
            Err(error) => {
                if let Some(tx) = first_init.take() {
                    let reason = error.to_string();
                    let _ = tx.send(Err(error));
                    shared.set_state(PluginState::Error { reason });
                    return;
                }
                failures += 1;
                if failures as usize > job.backoff.delays.len() {
                    shared.set_state(PluginState::Error {
                        reason: format!("could not restart: {error}"),
                    });
                    return;
                }
                shared.restarts.store(failures, Ordering::Relaxed);
                if !sleep_or_cancel(job.backoff.delays[(failures - 1) as usize], &cancel).await {
                    shared.set_state(PluginState::Disabled);
                    return;
                }
                continue;
            }
        };

        let init_params = serde_json::json!({
            "protocol": berth_plugin_protocol::PROTOCOL_VERSION,
            "host_version": env!("CARGO_PKG_VERSION"),
            "config": job.config,
        });
        let init = shared
            .request(methods::INITIALIZE, init_params, INIT_TIMEOUT, &cancel)
            .await;
        let reason = match init {
            Ok(value) => {
                *lock(&shared.init_result) = Some(value);
                if let Some(tx) = first_init.take() {
                    let _ = tx.send(Ok(()));
                }
                tokio::select! {
                    reason = &mut generation.exit_rx => {
                        reason.unwrap_or_else(|_| "child went away".into())
                    }
                    _ = cancel.cancelled() => {
                        shutdown_generation(&shared, &mut generation).await;
                        shared.fail_pending("plugin stopped");
                        shared.set_state(PluginState::Disabled);
                        return;
                    }
                }
            }
            Err(error) => {
                generation.kill().await;
                if cancel.is_cancelled() {
                    if let Some(tx) = first_init.take() {
                        let _ = tx.send(Err(PluginError::Exited("cancelled".into())));
                    }
                    shared.set_endpoint(None);
                    shared.set_state(PluginState::Disabled);
                    return;
                }
                if let Some(tx) = first_init.take() {
                    let reason = format!("initialize failed: {error}");
                    let _ = tx.send(Err(error));
                    shared.set_endpoint(None);
                    shared.set_state(PluginState::Error { reason });
                    return;
                }
                format!("initialize failed: {error}")
            }
        };

        shared.set_endpoint(None);
        shared.fail_pending(&reason);
        failures += 1;
        if failures as usize > job.backoff.delays.len() {
            let reason = format!("{reason}; restarts exhausted");
            tracing::warn!(plugin = %shared.plugin_id, "plugin gave up: {reason}");
            shared.set_state(PluginState::Error { reason });
            return;
        }
        shared.restarts.store(failures, Ordering::Relaxed);
        tracing::warn!(
            plugin = %shared.plugin_id,
            restart = failures,
            "plugin exited ({reason}); restarting"
        );
        if !sleep_or_cancel(job.backoff.delays[(failures - 1) as usize], &cancel).await {
            shared.set_state(PluginState::Disabled);
            return;
        }
    }
}

/// A running (or restarting, or failed) plugin. See the module docs for
/// the request semantics across restarts.
pub struct PluginHandle {
    shared: Arc<Shared>,
    cancel: CancellationToken,
    supervisor: Option<tokio::task::JoinHandle<()>>,
}

impl PluginHandle {
    /// Start with the default restart backoff (1 s, 5 s, 30 s, then
    /// Error). `path` is the child's `PATH` (from
    /// `berth_sys::SearchPath::path_var`); bare executable names in the
    /// manifest resolve through it. Returns once the `initialize`
    /// handshake has succeeded; later crashes restart in the background.
    pub async fn start(
        plugin: &InstalledPlugin,
        secrets: Arc<dyn SecretStore>,
        config: serde_json::Value,
        path: std::ffi::OsString,
        cancel: CancellationToken,
    ) -> Result<Self, PluginError> {
        Self::start_with_backoff(plugin, secrets, config, path, cancel, Backoff::default()).await
    }

    /// [`PluginHandle::start`] with an injected restart backoff.
    pub async fn start_with_backoff(
        plugin: &InstalledPlugin,
        secrets: Arc<dyn SecretStore>,
        config: serde_json::Value,
        path: std::ffi::OsString,
        cancel: CancellationToken,
        backoff: Backoff,
    ) -> Result<Self, PluginError> {
        let executable = resolve_executable(plugin, &path)?;
        let (notifications, _) = broadcast::channel(NOTIFICATION_CAPACITY);
        let shared = Arc::new(Shared {
            plugin_id: plugin.manifest.id.clone(),
            pending: Mutex::new(HashMap::new()),
            endpoint: Mutex::new(None),
            endpoint_changed: Notify::new(),
            state: Mutex::new(PluginState::Enabled),
            init_result: Mutex::new(None),
            stderr_tail: Mutex::new(VecDeque::new()),
            restarts: AtomicU32::new(0),
            next_id: AtomicU64::new(1),
            notifications,
            dispatch: dispatch::Context::new(plugin, secrets),
        });
        // A child token so stop() ends this plugin without cancelling
        // the caller's token; the caller's token still ends both.
        let cancel = cancel.child_token();
        let (first_tx, first_rx) = oneshot::channel();
        let job = SupervisorJob {
            executable,
            dir: plugin.dir.clone(),
            path,
            config,
            backoff,
        };
        let supervisor = tokio::spawn(supervise(
            Arc::clone(&shared),
            job,
            cancel.clone(),
            Some(first_tx),
        ));
        let handle = Self {
            shared,
            cancel,
            supervisor: Some(supervisor),
        };
        match first_rx.await {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(error)) => {
                handle.stop().await;
                Err(error)
            }
            Err(_) => {
                handle.stop().await;
                Err(PluginError::Exited("supervisor stopped".into()))
            }
        }
    }

    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, PluginError> {
        self.shared
            .request(method, params, timeout, &self.cancel)
            .await
    }

    pub fn notifications(&self) -> broadcast::Receiver<Notification> {
        self.shared.notifications.subscribe()
    }

    /// `Enabled` while the supervisor runs (including during a restart),
    /// `Disabled` after a clean stop or cancellation, `Error` when the
    /// restart budget is exhausted.
    pub fn state(&self) -> PluginState {
        lock(&self.shared.state).clone()
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state(), PluginState::Enabled)
    }

    /// How many restarts have been scheduled since `start`.
    pub fn restarts(&self) -> u32 {
        self.shared.restarts.load(Ordering::Relaxed)
    }

    /// The latest successful `initialize` result (`{capabilities}`).
    pub fn init_result(&self) -> Option<serde_json::Value> {
        lock(&self.shared.init_result).clone()
    }

    /// The last stderr lines across generations (the Plugins screen's
    /// crash log).
    pub fn stderr_tail(&self) -> Vec<String> {
        lock(&self.shared.stderr_tail).iter().cloned().collect()
    }

    /// Stop the plugin: `shutdown`, a grace period, then kill. Cancelling
    /// the token given to `start` does the same in the background.
    pub async fn stop(mut self) {
        self.cancel.cancel();
        if let Some(supervisor) = self.supervisor.take() {
            if tokio::time::timeout(Duration::from_secs(5), supervisor)
                .await
                .is_err()
            {
                tracing::warn!(plugin = %self.shared.plugin_id, "supervisor did not stop in time");
            }
        }
    }
}

impl Drop for PluginHandle {
    fn drop(&mut self) {
        // The supervisor shuts the child down; kill_on_drop is the backstop.
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(dir: std::path::PathBuf, executable: &str) -> InstalledPlugin {
        InstalledPlugin {
            manifest: berth_plugin_protocol::Manifest {
                id: "coolify".into(),
                name: "Coolify".into(),
                version: "1".into(),
                protocol: berth_plugin_protocol::PROTOCOL_VERSION,
                executable: executable.into(),
                permissions: Vec::new(),
                contributes: Vec::new(),
                config: Vec::new(),
                secrets: Vec::new(),
            },
            dir,
            state: PluginState::Disabled,
            granted: Vec::new(),
        }
    }

    #[test]
    fn a_resources_manifest_resolves_its_executable_in_contents_macos() {
        let tmp = tempfile::tempdir().unwrap();
        let contents = tmp.path().join("Berth.app/Contents");
        let dir = contents.join("Resources/plugins/coolify");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::write(contents.join("MacOS/berth-coolify"), "x").unwrap();
        // A decoy in Resources must never be chosen.
        std::fs::write(dir.join("berth-coolify"), "decoy").unwrap();
        let found =
            resolve_executable(&installed(dir, "berth-coolify"), std::ffi::OsStr::new("")).unwrap();
        assert_eq!(found, contents.join("MacOS/berth-coolify"));
        assert!(!found.starts_with(contents.join("Resources")));
    }

    #[test]
    fn a_resources_manifest_never_falls_back_to_resources_or_path() {
        let tmp = tempfile::tempdir().unwrap();
        let contents = tmp.path().join("Berth.app/Contents");
        let dir = contents.join("Resources/plugins/coolify");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::write(dir.join("berth-coolify"), "decoy").unwrap();
        let on_path = tmp.path().join("bin");
        std::fs::create_dir_all(&on_path).unwrap();
        let p = on_path.join("berth-coolify");
        std::fs::write(&p, "x").unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let err = resolve_executable(&installed(dir, "berth-coolify"), on_path.as_os_str())
            .unwrap_err()
            .to_string();
        assert!(err.contains("Contents/MacOS/berth-coolify"), "{err}");
    }

    #[test]
    fn a_manifest_outside_a_bundle_resolves_next_to_the_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("plugins/coolify");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("berth-coolify"), "x").unwrap();
        let found = resolve_executable(
            &installed(dir.clone(), "berth-coolify"),
            std::ffi::OsStr::new(""),
        )
        .unwrap();
        assert_eq!(found, dir.join("berth-coolify"));
    }

    #[test]
    fn default_backoff_is_the_task_schedule() {
        assert_eq!(
            Backoff::default().delays,
            [
                Duration::from_secs(1),
                Duration::from_secs(5),
                Duration::from_secs(30)
            ]
        );
    }
}
