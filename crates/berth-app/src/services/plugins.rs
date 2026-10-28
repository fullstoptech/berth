//! Plugins (P9-T2, PLAN §6.10 and §6.11, D45, AD48): which plugins are
//! installed, which the user enabled and with what permissions, their
//! configuration and secrets, and the running `PluginHandle`s.
//!
//! One persisted record per plugin in `Store::plugin_config`, as
//! `{ "enabled": bool, "granted": [Permission], "config": {..} }`. Only
//! non-secret settings live there. A secret (a `ConfigField` of kind
//! `Secret` or a manifest `secrets` declaration) goes to the
//! [`SecretStore`] under `plugin/<id>/<key>` and nowhere else; the host
//! reads it back through `host/secret.get` when the plugin asks.
//!
//! The service owns every plugin process. The UI reads the
//! [`PluginsView`] signal, which a bridge task fills from the service's
//! `watch` channel, and talks to plugins only through [`PluginsService`]
//! (D23). A plugin that crashes is restarted by the host with backoff;
//! when that runs out the entry turns `Error` with the tail of the
//! plugin's stderr as the crash log.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use berth_plugin_host::dispatch::secret_store_key;
use berth_plugin_host::{Backoff, InstalledPlugin, PluginError, PluginHandle, PluginState};
use berth_plugin_protocol::{ConfigField, FieldKind, Manifest, Notification, Permission};
use berth_store::{SecretStore, Store};
use dioxus::prelude::*;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;

/// How often a running plugin's state is read back for the screen.
const MONITOR_EVERY: Duration = Duration::from_millis(500);
/// Lines of a crash log kept per plugin.
const CRASH_LOG_LINES: usize = 40;
/// Development override: extra plugin roots, colon-separated.
pub const PLUGIN_DIRS_VAR: &str = "BERTH_PLUGIN_DIRS";

// ---------------------------------------------------------------------
// What the screen reads
// ---------------------------------------------------------------------

/// One installed plugin as the Plugins screen shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginEntry {
    /// The manifest, directory, run state and the permissions the user granted.
    pub plugin: InstalledPlugin,
    /// The saved non-secret settings (a JSON object).
    pub config: Value,
    /// Keys whose secret is stored (the value itself is never read into state).
    pub secrets_set: BTreeSet<String>,
    /// The plugin's last output before it failed, newest last. Empty while healthy.
    pub crash_log: Vec<String>,
    /// How many times the host restarted it since it started.
    pub restarts: u32,
}

impl PluginEntry {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.plugin.manifest.id
    }

    /// Whether the user's grants cover everything the manifest asks for.
    #[must_use]
    pub fn permissions_granted(&self) -> bool {
        self.plugin
            .manifest
            .permissions
            .iter()
            .all(|p| self.plugin.granted.contains(p))
    }

    /// The labels of required settings that have no value yet.
    #[must_use]
    pub fn missing_required(&self) -> Vec<String> {
        self.plugin
            .manifest
            .config
            .iter()
            .filter(|f| f.required && !self.has_value(f))
            .map(|f| f.label.clone())
            .collect()
    }

    fn has_value(&self, field: &ConfigField) -> bool {
        match field.kind {
            FieldKind::Secret => self.secrets_set.contains(&field.key),
            FieldKind::Bool => true,
            FieldKind::Text | FieldKind::Url => self
                .config
                .get(&field.key)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty()),
        }
    }

    /// The hosts of the manifest's `Network` permissions, for the request sheet.
    #[must_use]
    pub fn network_hosts(&self) -> Vec<String> {
        network_hosts(&self.plugin.manifest.permissions)
    }

    /// Every secret the Configure form offers: secret-kind settings first,
    /// then the manifest's `secrets` declarations, as `(key, label)`.
    #[must_use]
    pub fn secret_fields(&self) -> Vec<(String, String)> {
        secret_fields(&self.plugin.manifest)
    }
}

/// The hosts named by `Network` permissions, in order, without repeats.
#[must_use]
pub fn network_hosts(permissions: &[Permission]) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for permission in permissions {
        if let Permission::Network { hosts: named } = permission {
            for host in named {
                if !hosts.contains(host) {
                    hosts.push(host.clone());
                }
            }
        }
    }
    hosts
}

/// A permission as one plain sentence for the request sheet.
#[must_use]
pub fn permission_sentence(permission: &Permission) -> String {
    match permission {
        Permission::Network { hosts } if hosts.iter().any(|h| h == "*") => {
            "Connect to any address on the network".to_string()
        }
        Permission::Network { hosts } => format!("Connect to {}", hosts.join(", ")),
        Permission::ReadEngines => "Read the state of your engines".to_string(),
        Permission::RunEngineActions => "Ask Berth to act on your engines".to_string(),
        Permission::Secrets { keys } => {
            format!("Read these stored secrets: {}", keys.join(", "))
        }
        Permission::Mcp => "Offer MCP servers to the Assistant".to_string(),
    }
}

fn secret_fields(manifest: &Manifest) -> Vec<(String, String)> {
    let mut fields: Vec<(String, String)> = manifest
        .config
        .iter()
        .filter(|f| f.kind == FieldKind::Secret)
        .map(|f| (f.key.clone(), f.label.clone()))
        .collect();
    for decl in &manifest.secrets {
        if !fields.iter().any(|(key, _)| *key == decl.key) {
            fields.push((decl.key.clone(), decl.label.clone()));
        }
    }
    fields
}

/// The signal the Plugins screen reads (and, later, the Coolify section).
#[derive(Clone, Copy)]
pub struct PluginsView {
    pub entries: Signal<Vec<PluginEntry>>,
}

impl PluginsView {
    /// One entry by plugin id.
    #[must_use]
    pub fn entry(&self, id: &str) -> Option<PluginEntry> {
        self.entries.read().iter().find(|e| e.id() == id).cloned()
    }
}

/// Provide one [`PluginsView`] from the App root (or a scene root).
pub fn provide_plugins_view() -> PluginsView {
    use_hook(|| {
        provide_context(PluginsView {
            entries: Signal::new(Vec::new()),
        })
    })
}

/// Copy the service's published entries into the view as they change. Call
/// from the App root inside `spawn`-able code; the task ends with its scope.
pub fn bridge(service: &PluginsService, mut view: PluginsView) {
    let mut rx = service.subscribe();
    view.entries.set(rx.borrow_and_update().clone());
    spawn(async move {
        while rx.changed().await.is_ok() {
            let entries = rx.borrow_and_update().clone();
            view.entries.set(entries);
        }
    });
}

// ---------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------

/// The record kept in `Store::plugin_config`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    granted: Vec<Permission>,
    #[serde(default)]
    config: Map<String, Value>,
}

fn load_saved(store: &Store, id: &str) -> Saved {
    store
        .plugin_config(id)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn write_saved(store: &Store, id: &str, saved: &Saved) -> Result<(), String> {
    let value = serde_json::to_value(saved).map_err(|e| e.to_string())?;
    store
        .set_plugin_config(id, &value)
        .map_err(|e| format!("Berth could not save the plugin's settings: {e}"))
}

// ---------------------------------------------------------------------
// Where plugins live
// ---------------------------------------------------------------------

/// The directories to scan, first match wins (PLAN §6.10): the development
/// override, the app binary's directory and its `plugins/` child, the app
/// bundle's `Contents/Resources/plugins` (the bundled Coolify manifest, only
/// when the binary runs from `…/Contents/MacOS`; AD70), then
/// `<data dir>/plugins`.
#[must_use]
pub fn plugin_dirs_from(
    extra: Option<&OsStr>,
    data_dir: &Path,
    exe_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = extra
        .map(|e| std::env::split_paths(e).collect())
        .unwrap_or_default();
    if let Some(exe_dir) = exe_dir {
        dirs.push(exe_dir.to_path_buf());
        dirs.push(exe_dir.join("plugins"));
        dirs.extend(berth_plugin_host::bundle_plugins_dir(exe_dir));
    }
    dirs.push(data_dir.join("plugins"));
    let mut seen = Vec::new();
    for dir in dirs {
        if !dir.as_os_str().is_empty() && !seen.contains(&dir) {
            seen.push(dir);
        }
    }
    seen
}

/// [`plugin_dirs_from`] for this process.
#[must_use]
pub fn plugin_dirs() -> Vec<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    plugin_dirs_from(
        std::env::var_os(PLUGIN_DIRS_VAR).as_deref(),
        &berth_sys::paths::data_dir(),
        exe_dir.as_deref(),
    )
}

/// What a plugin child gets as `PATH`: the app binary's directory (so a
/// bundled `berth-coolify` resolves by name) then the shared login-shell
/// search path (`berth_sys::SearchPath`, AD34).
pub async fn plugin_path() -> OsString {
    let shared = crate::services::login_path::search_path().await;
    let mut dirs: Vec<PathBuf> = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .into_iter()
        .collect();
    dirs.extend(shared.dirs().iter().cloned());
    berth_sys::SearchPath::from_dirs(dirs).path_var()
}

// ---------------------------------------------------------------------
// The service
// ---------------------------------------------------------------------

/// What [`PluginsService::new`] needs.
pub struct PluginsDeps {
    pub store: Arc<Store>,
    /// The one secret store; Configure writes and the host reads through it.
    pub secrets: Arc<dyn SecretStore>,
    pub dirs: Vec<PathBuf>,
    /// `PATH` for plugin children; `None` resolves [`plugin_path`] at start.
    pub path: Option<OsString>,
    pub cancel: CancellationToken,
    /// Restart delays; [`Backoff::default`] is 1 s, 5 s, 30 s.
    pub backoff: Backoff,
}

/// Settings typed into the Configure form.
#[derive(Default)]
pub struct ConfigUpdate {
    /// Text, URL and switch settings by key. Keys that are not settings of
    /// the manifest are ignored.
    pub values: BTreeMap<String, Value>,
    /// New secret values by key. A key left out keeps what is stored.
    pub secrets: BTreeMap<String, SecretString>,
}

struct Running {
    handle: Arc<PluginHandle>,
    cancel: CancellationToken,
}

struct Inner {
    deps: PluginsDeps,
    entries: Mutex<Vec<PluginEntry>>,
    running: Mutex<HashMap<String, Running>>,
    publish: watch::Sender<Vec<PluginEntry>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Owns the plugin processes and the persisted enablement.
#[derive(Clone)]
pub struct PluginsService {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PluginsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginsService").finish_non_exhaustive()
    }
}

impl PluginsService {
    /// A service that has scanned nothing yet; call [`PluginsService::start`].
    #[must_use]
    pub fn new(deps: PluginsDeps) -> Self {
        let (publish, _) = watch::channel(Vec::new());
        Self {
            inner: Arc::new(Inner {
                deps,
                entries: Mutex::new(Vec::new()),
                running: Mutex::new(HashMap::new()),
                publish,
            }),
        }
    }

    /// The real service: the app's store and secret store, the real plugin
    /// directories and the login-shell `PATH` (resolved at start).
    #[must_use]
    pub fn live(
        store: Arc<Store>,
        secrets: Arc<dyn SecretStore>,
        cancel: CancellationToken,
    ) -> Self {
        Self::new(PluginsDeps {
            store,
            secrets,
            dirs: plugin_dirs(),
            path: None,
            cancel,
            backoff: Backoff::default(),
        })
    }

    /// Entries as they change; the receiver starts with the current list.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<Vec<PluginEntry>> {
        self.inner.publish.subscribe()
    }

    #[must_use]
    pub fn entries(&self) -> Vec<PluginEntry> {
        lock(&self.inner.entries).clone()
    }

    #[must_use]
    pub fn entry(&self, id: &str) -> Option<PluginEntry> {
        lock(&self.inner.entries)
            .iter()
            .find(|e| e.id() == id)
            .cloned()
    }

    /// Scan for plugins, then start every one the user enabled.
    pub async fn start(&self) {
        self.scan().await;
        let enabled: Vec<String> = self
            .entries()
            .into_iter()
            .filter(|e| e.plugin.state == PluginState::Enabled)
            .map(|e| e.id().to_string())
            .collect();
        for id in enabled {
            self.start_plugin(&id).await;
        }
    }

    /// Discover plugins and overlay the saved state. A plugin the user
    /// enabled stays `Enabled` here only if its grants still cover its
    /// manifest; a manifest that asks for more drops it back to `Disabled`
    /// until the user approves again.
    pub async fn scan(&self) {
        let inner = Arc::clone(&self.inner);
        let found = tokio::task::spawn_blocking(move || {
            berth_plugin_host::discover(&inner.deps.dirs)
                .into_iter()
                .map(|plugin| entry_from_disk(&inner.deps, plugin))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        let ids: Vec<&str> = found.iter().map(PluginEntry::id).collect();
        tracing::info!(plugins = ?ids, "plugins discovered");
        *lock(&self.inner.entries) = found;
        self.publish();
    }

    /// The user approved the manifest's permissions: record the grant, mark
    /// the plugin enabled and start it.
    ///
    /// # Errors
    /// A sentence: the plugin is unknown or its record could not be saved.
    /// A plugin that fails to start is not an error here; it shows `Error`.
    pub async fn enable(&self, id: &str) -> Result<(), String> {
        let entry = self.entry(id).ok_or_else(|| unknown(id))?;
        let saved = Saved {
            enabled: true,
            granted: entry.plugin.manifest.permissions.clone(),
            config: object(&entry.config),
        };
        self.persist(id, saved).await?;
        self.update(id, |e| {
            e.plugin.granted = e.plugin.manifest.permissions.clone();
            e.plugin.state = PluginState::Enabled;
            e.crash_log.clear();
            e.restarts = 0;
        });
        self.start_plugin(id).await;
        Ok(())
    }

    /// Stop the plugin and forget its grants. Settings and stored secrets
    /// stay, so enabling again asks for permissions only.
    ///
    /// # Errors
    /// A sentence: the plugin is unknown or its record could not be saved.
    pub async fn disable(&self, id: &str) -> Result<(), String> {
        let entry = self.entry(id).ok_or_else(|| unknown(id))?;
        self.stop_plugin(id).await;
        let saved = Saved {
            enabled: false,
            granted: Vec::new(),
            config: object(&entry.config),
        };
        self.persist(id, saved).await?;
        self.update(id, |e| {
            e.plugin.granted.clear();
            e.plugin.state = PluginState::Disabled;
            e.crash_log.clear();
            e.restarts = 0;
        });
        Ok(())
    }

    /// Save the Configure form: settings to the store, secrets to the
    /// secret store. A running plugin restarts so `initialize` sees the new
    /// settings.
    ///
    /// # Errors
    /// A sentence naming the setting that is missing or malformed, or the
    /// store that failed. Nothing is written when validation fails.
    pub async fn configure(&self, id: &str, update: ConfigUpdate) -> Result<(), String> {
        let entry = self.entry(id).ok_or_else(|| unknown(id))?;
        let config = merge_config(&entry, &update)?;
        let saved = Saved {
            enabled: entry.plugin.state != PluginState::Disabled,
            granted: entry.plugin.granted.clone(),
            config: config.clone(),
        };
        let inner = Arc::clone(&self.inner);
        let plugin_id = id.to_string();
        let manifest = entry.plugin.manifest.clone();
        let secrets = update.secrets;
        let secret_keys: BTreeSet<String> = secret_fields(&manifest)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        let stored = tokio::task::spawn_blocking(move || {
            write_saved(&inner.deps.store, &plugin_id, &saved)?;
            for (key, value) in &secrets {
                if !secret_keys.contains(key) || value.expose_secret().is_empty() {
                    continue;
                }
                inner
                    .deps
                    .secrets
                    .set(&secret_store_key(&plugin_id, key), value)
                    .map_err(|e| format!("Berth could not store the secret: {e}"))?;
            }
            Ok::<_, String>(stored_secrets(&inner.deps, &manifest))
        })
        .await
        .map_err(|e| e.to_string())??;
        self.update(id, |e| {
            e.config = Value::Object(config);
            e.secrets_set = stored;
        });
        if lock(&self.inner.running).contains_key(id) {
            self.stop_plugin(id).await;
            self.start_plugin(id).await;
        }
        Ok(())
    }

    /// Forget one stored secret.
    ///
    /// # Errors
    /// A sentence: the plugin or key is unknown or the secret store failed.
    pub async fn clear_secret(&self, id: &str, key: &str) -> Result<(), String> {
        let entry = self.entry(id).ok_or_else(|| unknown(id))?;
        if !entry.secret_fields().iter().any(|(k, _)| k == key) {
            return Err(format!(
                "{} has no secret called {key}",
                entry.plugin.manifest.name
            ));
        }
        let inner = Arc::clone(&self.inner);
        let store_key = secret_store_key(id, key);
        tokio::task::spawn_blocking(move || inner.deps.secrets.delete(&store_key))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("Berth could not remove the secret: {e}"))?;
        let key = key.to_string();
        self.update(id, |e| {
            e.secrets_set.remove(&key);
        });
        if lock(&self.inner.running).contains_key(id) {
            self.stop_plugin(id).await;
            self.start_plugin(id).await;
        }
        Ok(())
    }

    /// Start an enabled plugin again (the Error state's "Try again").
    ///
    /// # Errors
    /// A sentence: the plugin is unknown or not enabled.
    pub async fn restart(&self, id: &str) -> Result<(), String> {
        let entry = self.entry(id).ok_or_else(|| unknown(id))?;
        if entry.plugin.state == PluginState::Disabled {
            return Err(format!("{} is not enabled", entry.plugin.manifest.name));
        }
        self.stop_plugin(id).await;
        self.update(id, |e| {
            e.plugin.state = PluginState::Enabled;
            e.crash_log.clear();
            e.restarts = 0;
        });
        self.start_plugin(id).await;
        Ok(())
    }

    /// One request to a running plugin (`view/load`, `action/invoke`, ...).
    ///
    /// # Errors
    /// [`PluginError::Exited`] when the plugin is not running, otherwise
    /// whatever the host returns.
    pub async fn request(
        &self,
        id: &str,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, PluginError> {
        let handle = lock(&self.inner.running)
            .get(id)
            .map(|r| Arc::clone(&r.handle))
            .ok_or_else(|| PluginError::Exited("the plugin is not running".to_string()))?;
        handle.request(method, params, timeout).await
    }

    /// The plugin's notifications (`logs/lines`, `view/invalidate`, ...), when it runs.
    #[must_use]
    pub fn notifications(&self, id: &str) -> Option<broadcast::Receiver<Notification>> {
        lock(&self.inner.running)
            .get(id)
            .map(|r| r.handle.notifications())
    }

    /// Stop every plugin (app quit).
    pub async fn shutdown(&self) {
        let ids: Vec<String> = lock(&self.inner.running).keys().cloned().collect();
        for id in ids {
            self.stop_plugin(&id).await;
        }
    }

    // -- internals ------------------------------------------------------

    fn publish(&self) {
        let entries = lock(&self.inner.entries).clone();
        self.inner.publish.send_replace(entries);
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut PluginEntry)) {
        {
            let mut entries = lock(&self.inner.entries);
            if let Some(entry) = entries.iter_mut().find(|e| e.id() == id) {
                change(entry);
            }
        }
        self.publish();
    }

    async fn persist(&self, id: &str, saved: Saved) -> Result<(), String> {
        let inner = Arc::clone(&self.inner);
        let id = id.to_string();
        tokio::task::spawn_blocking(move || write_saved(&inner.deps.store, &id, &saved))
            .await
            .map_err(|e| e.to_string())?
    }

    /// Start the plugin the entry describes and record how it went.
    async fn start_plugin(&self, id: &str) {
        let Some(entry) = self.entry(id) else { return };
        self.stop_plugin(id).await;
        let cancel = self.inner.deps.cancel.child_token();
        let path = match &self.inner.deps.path {
            Some(path) => path.clone(),
            None => plugin_path().await,
        };
        let mut plugin = entry.plugin.clone();
        plugin.state = PluginState::Enabled;
        let started = PluginHandle::start_with_backoff(
            &plugin,
            Arc::clone(&self.inner.deps.secrets),
            entry.config.clone(),
            path,
            cancel.clone(),
            self.inner.deps.backoff.clone(),
        )
        .await;
        match started {
            Ok(handle) => {
                let handle = Arc::new(handle);
                lock(&self.inner.running).insert(
                    id.to_string(),
                    Running {
                        handle: Arc::clone(&handle),
                        cancel,
                    },
                );
                self.update(id, |e| {
                    e.plugin.state = PluginState::Enabled;
                    e.crash_log.clear();
                    e.restarts = 0;
                });
                self.monitor(id.to_string(), handle);
            }
            Err(error) => {
                tracing::warn!(plugin = %id, "plugin did not start: {error}");
                let reason = error.to_string();
                self.update(id, |e| {
                    e.plugin.state = PluginState::Error {
                        reason: reason.clone(),
                    };
                    e.crash_log = vec![reason];
                });
            }
        }
    }

    async fn stop_plugin(&self, id: &str) {
        let running = lock(&self.inner.running).remove(id);
        if let Some(running) = running {
            running.cancel.cancel();
            if let Ok(handle) = Arc::try_unwrap(running.handle) {
                handle.stop().await;
            }
        }
    }

    /// Read a running plugin's state back into its entry until it stops or
    /// fails for good.
    fn monitor(&self, id: String, handle: Arc<PluginHandle>) {
        let service = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(MONITOR_EVERY).await;
                let current = lock(&service.inner.running)
                    .get(&id)
                    .is_some_and(|r| Arc::ptr_eq(&r.handle, &handle));
                if !current {
                    return;
                }
                let state = handle.state();
                let restarts = handle.restarts();
                let log = if restarts > 0 || matches!(state, PluginState::Error { .. }) {
                    tail(handle.stderr_tail())
                } else {
                    Vec::new()
                };
                let failed = matches!(state, PluginState::Error { .. });
                service.update(&id, |e| {
                    if e.plugin.state != state || e.restarts != restarts || e.crash_log != log {
                        e.plugin.state = state.clone();
                        e.restarts = restarts;
                        e.crash_log = log.clone();
                    }
                });
                if failed {
                    let removed = {
                        let mut running = lock(&service.inner.running);
                        match running.get(&id) {
                            Some(r) if Arc::ptr_eq(&r.handle, &handle) => running.remove(&id),
                            _ => None,
                        }
                    };
                    if let Some(removed) = removed {
                        removed.cancel.cancel();
                    }
                    return;
                }
            }
        });
    }
}

fn unknown(id: &str) -> String {
    format!("There is no plugin called {id}")
}

fn tail(mut lines: Vec<String>) -> Vec<String> {
    if lines.len() > CRASH_LOG_LINES {
        lines.drain(..lines.len() - CRASH_LOG_LINES);
    }
    lines
}

fn object(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn stored_secrets(deps: &PluginsDeps, manifest: &Manifest) -> BTreeSet<String> {
    secret_fields(manifest)
        .into_iter()
        .map(|(key, _)| key)
        .filter(|key| {
            deps.secrets
                .get(&secret_store_key(&manifest.id, key))
                .ok()
                .flatten()
                .is_some_and(|v| !v.expose_secret().is_empty())
        })
        .collect()
}

/// Build the entry for a plugin found on disk: the saved record, with the
/// grants checked against the manifest.
fn entry_from_disk(deps: &PluginsDeps, mut plugin: InstalledPlugin) -> PluginEntry {
    let saved = load_saved(&deps.store, &plugin.manifest.id);
    let covered = plugin
        .manifest
        .permissions
        .iter()
        .all(|p| saved.granted.contains(p));
    plugin.granted = if saved.enabled && covered {
        plugin.manifest.permissions.clone()
    } else {
        Vec::new()
    };
    plugin.state = if saved.enabled && covered {
        PluginState::Enabled
    } else {
        PluginState::Disabled
    };
    let secrets_set = stored_secrets(deps, &plugin.manifest);
    PluginEntry {
        plugin,
        config: Value::Object(saved.config),
        secrets_set,
        crash_log: Vec::new(),
        restarts: 0,
    }
}

/// The settings object after the update, validated against the manifest.
fn merge_config(entry: &PluginEntry, update: &ConfigUpdate) -> Result<Map<String, Value>, String> {
    let mut config = object(&entry.config);
    for field in &entry.plugin.manifest.config {
        match field.kind {
            FieldKind::Secret => {
                let given = update
                    .secrets
                    .get(&field.key)
                    .is_some_and(|v| !v.expose_secret().is_empty());
                if field.required && !given && !entry.secrets_set.contains(&field.key) {
                    return Err(format!("{} is required", field.label));
                }
            }
            FieldKind::Bool => {
                if let Some(value) = update.values.get(&field.key) {
                    config.insert(
                        field.key.clone(),
                        Value::Bool(value.as_bool().unwrap_or(false)),
                    );
                }
            }
            FieldKind::Text | FieldKind::Url => {
                let text = match update.values.get(&field.key) {
                    Some(value) => value.as_str().unwrap_or_default().trim().to_string(),
                    None => config
                        .get(&field.key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                };
                if let Some(problem) = field_problem(field, &text) {
                    return Err(problem);
                }
                if text.is_empty() {
                    config.remove(&field.key);
                } else {
                    config.insert(field.key.clone(), json!(text));
                }
            }
        }
    }
    // A required secret declared only in `secrets` is the plugin's own
    // business: it reports "not configured" in its view.
    Ok(config)
}

/// What is wrong with a typed Text or URL setting, as a sentence for the form.
#[must_use]
pub fn field_problem(field: &ConfigField, text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return field
            .required
            .then(|| format!("{} is required", field.label));
    }
    if field.kind != FieldKind::Url {
        return None;
    }
    let label = &field.label;
    let Ok(parsed) = url::Url::parse(text) else {
        return Some(format!("{label} is not a web address"));
    };
    (!matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none_or(str::is_empty))
        .then(|| format!("{label} must start with http:// or https://"))
}

#[cfg(test)]
#[path = "plugins_tests.rs"]
mod tests;
