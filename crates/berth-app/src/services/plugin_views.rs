//! The data behind plugin views (P9-T5, PLAN §6.10 and §6.11, D23, D45): the
//! views a plugin returns for `view/load`, the log lines it streams, the
//! actions the user confirms, and the project badges it answers. The UI
//! (`screens/plugin_view.rs`) reads [`PluginViewStore`] and never calls a
//! plugin; every request goes through [`PluginsService::request`] from here.
//!
//! Three layers: pure rules (what the sidebar and the palette contribute,
//! the params of an action, how log lines fold), the async functions that
//! talk to the service (`load_view`, `invoke_action`, `follow_log`,
//! `load_badges`), and the store of signals that hands their results to the
//! screens. A task never writes a signal: log lines travel on a `watch`
//! channel and a Dioxus task copies each change into the store.

use std::collections::BTreeMap;
use std::time::Duration;

use berth_plugin_host::PluginState;
use berth_plugin_protocol::{
    methods, ActionResult, ActionSpec, Contribution, Notification, Tone, View,
};
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use serde_json::{json, Value};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::services::plugins::{PluginEntry, PluginsService};
use crate::state::{AppState, Toast, ToastTone};

/// How long a `view/load` may take (a Coolify call plus its retries).
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
/// How long an `action/invoke` may take.
pub const ACTION_TIMEOUT: Duration = Duration::from_secs(60);
/// The hidden view that carries a plugin's version warning (AD49, AD51).
pub const STATUS_VIEW: &str = "status";
/// Log lines kept per stream; older ones drop off the front.
pub const MAX_LOG_LINES: usize = 2000;
/// How often project badges are asked for again.
pub const BADGE_REFRESH: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------

/// A (plugin id, view key) pair: one slot of the store.
pub type ViewKey = (String, String);

/// What one view slot holds. A `View` is a protocol type, so the size gap
/// between the variants is not boxed away.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum ViewState {
    Loading,
    Ready(View),
    /// The plugin is not running, timed out, or sent something that is not a view.
    Failed(String),
}

/// A plugin's answer as a view. A plugin that sends a shape this host does
/// not know is a failed view, not a crash.
///
/// # Errors
/// The decode error, worded for a person.
pub fn parse_view(value: Value) -> Result<View, String> {
    serde_json::from_value(value)
        .map_err(|e| format!("The plugin sent a view Berth cannot read: {e}"))
}

/// Ask a running plugin for one view.
pub async fn load_view(service: &PluginsService, plugin: &str, view: &str) -> ViewState {
    let params = json!({ "view": view, "params": {} });
    match service
        .request(plugin, methods::VIEW_LOAD, params, LOAD_TIMEOUT)
        .await
    {
        Ok(value) => match parse_view(value) {
            Ok(view) => ViewState::Ready(view),
            Err(reason) => ViewState::Failed(reason),
        },
        Err(e) => ViewState::Failed(e.to_string()),
    }
}

/// The params `action/invoke` carries for `action`: its own params with each
/// of its options set to what the user chose (an option never chosen keeps
/// its default).
#[must_use]
pub fn action_params(action: &ActionSpec, chosen: &BTreeMap<String, bool>) -> Value {
    let mut params = match &action.params {
        Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    for option in &action.options {
        let on = chosen.get(&option.key).copied().unwrap_or(option.default);
        params.insert(option.key.clone(), Value::Bool(on));
    }
    Value::Object(params)
}

/// Run one action the user confirmed.
///
/// # Errors
/// The plugin's refusal or a transport failure, as text.
pub async fn invoke_action(
    service: &PluginsService,
    plugin: &str,
    action: &ActionSpec,
    chosen: &BTreeMap<String, bool>,
) -> Result<ActionResult, String> {
    let params = json!({ "action": action.id, "params": action_params(action, chosen) });
    let value = service
        .request(plugin, methods::ACTION_INVOKE, params, ACTION_TIMEOUT)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_value(value).map_err(|e| format!("The plugin's answer is unreadable: {e}"))
}

// ---------------------------------------------------------------------
// Contributions: the sidebar and the palette
// ---------------------------------------------------------------------

/// One sidebar group a plugin contributes.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarGroup {
    pub plugin: String,
    pub id: String,
    pub title: String,
    pub items: Vec<SidebarLink>,
}

/// One item of a group: it opens `view` of `plugin`.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarLink {
    pub id: String,
    pub title: String,
    pub view: String,
}

/// The sidebar groups of every plugin that is enabled (a disabled or failed
/// plugin has nothing to show).
#[must_use]
pub fn sidebar_groups(entries: &[PluginEntry]) -> Vec<SidebarGroup> {
    entries
        .iter()
        .filter(|e| e.plugin.state == PluginState::Enabled)
        .flat_map(|entry| {
            entry
                .plugin
                .manifest
                .contributes
                .iter()
                .filter_map(|c| match c {
                    Contribution::SidebarSection { id, title, items } => Some(SidebarGroup {
                        plugin: entry.id().to_string(),
                        id: id.clone(),
                        title: title.clone(),
                        items: items
                            .iter()
                            .map(|i| SidebarLink {
                                id: i.id.clone(),
                                title: i.title.clone(),
                                view: i.view.clone(),
                            })
                            .collect(),
                    }),
                    _ => None,
                })
        })
        .collect()
}

/// What a palette command does.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandKind {
    /// Show one of the plugin's views in its section.
    Open { view: String },
    /// Invoke an action the plugin named, with no params.
    Invoke { action: String },
}

/// A command the palette lists for an enabled plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginCommand {
    pub plugin: String,
    pub id: String,
    /// "Coolify: Deployments".
    pub title: String,
    pub kind: CommandKind,
}

/// The palette commands of every enabled plugin: one "Plugin: Item" that
/// opens each sidebar item, then the commands the manifest lists, each of
/// which invokes its action.
#[must_use]
pub fn palette_commands(entries: &[PluginEntry]) -> Vec<PluginCommand> {
    let mut out = Vec::new();
    for entry in entries
        .iter()
        .filter(|e| e.plugin.state == PluginState::Enabled)
    {
        let name = &entry.plugin.manifest.name;
        for group in sidebar_groups(std::slice::from_ref(entry)) {
            for item in group.items {
                out.push(PluginCommand {
                    plugin: entry.id().to_string(),
                    id: format!("{}/{}", group.id, item.id),
                    title: format!("{name}: {}", item.title),
                    kind: CommandKind::Open { view: item.view },
                });
            }
        }
        for contribution in &entry.plugin.manifest.contributes {
            if let Contribution::PaletteCommands { commands } = contribution {
                for command in commands {
                    out.push(PluginCommand {
                        plugin: entry.id().to_string(),
                        id: command.id.clone(),
                        title: format!("{name}: {}", command.title),
                        kind: CommandKind::Invoke {
                            action: command.action.clone(),
                        },
                    });
                }
            }
        }
    }
    out
}

/// The commands whose title matches every word of `query` (case-insensitive).
/// An empty query matches none: the palette lists plugin commands only once
/// the user types.
#[must_use]
pub fn match_commands<'a>(commands: &'a [PluginCommand], query: &str) -> Vec<&'a PluginCommand> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    commands
        .iter()
        .filter(|c| {
            let title = c.title.to_lowercase();
            words.iter().all(|w| title.contains(w))
        })
        .collect()
}

// ---------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------

/// The lines of one log stream so far.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LogState {
    /// Each line with the moment the host received it (plugin lines carry no time).
    pub lines: Vec<(DateTime<Utc>, String)>,
    /// `finished`, `failed`, `cancelled` or `error` once the stream has ended.
    pub final_status: Option<String>,
    /// Lines dropped off the front past [`MAX_LOG_LINES`].
    pub dropped: usize,
    /// Bumped on every change, so a view can tell new lines from the same ones.
    pub version: u64,
}

impl LogState {
    /// Fold one `logs/lines` notification's params in. Returns whether
    /// anything changed.
    pub fn apply(&mut self, params: &Value, now: DateTime<Utc>) -> bool {
        let mut changed = false;
        if let Some(lines) = params.get("lines").and_then(Value::as_array) {
            for line in lines.iter().filter_map(Value::as_str) {
                self.lines.push((now, line.to_string()));
                changed = true;
            }
        }
        if self.lines.len() > MAX_LOG_LINES {
            let over = self.lines.len() - MAX_LOG_LINES;
            self.lines.drain(..over);
            self.dropped += over;
        }
        if let Some(status) = params.get("final_status").and_then(Value::as_str) {
            if self.final_status.as_deref() != Some(status) {
                self.final_status = Some(status.to_string());
                changed = true;
            }
        }
        if changed {
            self.version += 1;
        }
        changed
    }

    /// Did the stream end badly?
    #[must_use]
    pub fn failed(&self) -> bool {
        matches!(self.final_status.as_deref(), Some("failed" | "error"))
    }
}

/// A running log follow: the state as it changes, and the way to stop it.
pub struct LogFollow {
    pub state: watch::Receiver<LogState>,
    cancel: CancellationToken,
}

impl LogFollow {
    /// Close the stream (the plugin is told with `logs/close`).
    pub fn close(&self) {
        self.cancel.cancel();
    }
}

impl Drop for LogFollow {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Subscribe, then `logs/open` the stream, then fold its notifications until
/// it is closed. The subscription comes first so no line is missed.
#[must_use]
pub fn follow_log(service: &PluginsService, plugin: &str, stream: &str) -> LogFollow {
    let (tx, rx) = watch::channel(LogState::default());
    let cancel = CancellationToken::new();
    let task = (
        service.clone(),
        plugin.to_string(),
        stream.to_string(),
        cancel.clone(),
    );
    tokio::spawn(async move {
        let (service, plugin, stream, cancel) = task;
        run_follow(&service, &plugin, &stream, &cancel, &tx).await;
    });
    LogFollow { state: rx, cancel }
}

async fn run_follow(
    service: &PluginsService,
    plugin: &str,
    stream: &str,
    cancel: &CancellationToken,
    tx: &watch::Sender<LogState>,
) {
    let fail = |reason: String| {
        tx.send_modify(|state| {
            state.apply(
                &json!({ "lines": [reason], "final_status": "error" }),
                Utc::now(),
            );
        });
    };
    let Some(mut notes) = service.notifications(plugin) else {
        fail("The plugin is not running.".to_string());
        return;
    };
    let open = json!({ "stream": stream, "params": {} });
    if let Err(e) = service
        .request(plugin, methods::LOGS_OPEN, open, LOAD_TIMEOUT)
        .await
    {
        fail(format!("Could not open the log: {e}"));
        return;
    }
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            note = notes.recv() => match note {
                Ok(note) => fold_note(tx, stream, &note),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
        }
    }
    let close = json!({ "stream": stream });
    let _ = service
        .request(plugin, methods::LOGS_CLOSE, close, Duration::from_secs(2))
        .await;
}

fn fold_note(tx: &watch::Sender<LogState>, stream: &str, note: &Notification) {
    if note.method != methods::LOG_LINES
        || note.params.get("stream").and_then(Value::as_str) != Some(stream)
    {
        return;
    }
    tx.send_modify(|state| {
        state.apply(&note.params, Utc::now());
    });
}

// ---------------------------------------------------------------------
// Project badges
// ---------------------------------------------------------------------

/// One local project as `project/badges` asks about it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BadgeRequest {
    pub id: String,
    pub git_remote: Option<String>,
}

/// A badge on a project row: "Deployed on Coolify: running".
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectBadge {
    pub text: String,
    pub tone: Tone,
}

/// What `project/badges` is asked about: the projects that have a git remote.
#[must_use]
pub fn badge_requests(projects: &[berth_core::Project]) -> Vec<BadgeRequest> {
    projects
        .iter()
        .filter_map(|p| {
            let remote = p.git.as_ref()?.remote.clone()?;
            Some(BadgeRequest {
                id: p.id.0.clone(),
                git_remote: Some(remote),
            })
        })
        .collect()
}

/// A plugin's `[{id, text, tone}]` answer; an entry that is not one is skipped.
#[must_use]
pub fn parse_badges(value: &Value) -> BTreeMap<String, ProjectBadge> {
    let mut out = BTreeMap::new();
    for entry in value.as_array().into_iter().flatten() {
        let (Some(id), Some(text)) = (
            entry.get("id").and_then(Value::as_str),
            entry.get("text").and_then(Value::as_str),
        ) else {
            continue;
        };
        let tone = entry
            .get("tone")
            .and_then(|t| serde_json::from_value(t.clone()).ok())
            .unwrap_or(Tone::Neutral);
        out.insert(
            id.to_string(),
            ProjectBadge {
                text: text.to_string(),
                tone,
            },
        );
    }
    out
}

/// Ask every enabled plugin that contributes a project badge.
pub async fn load_badges(
    service: &PluginsService,
    requests: &[BadgeRequest],
) -> BTreeMap<String, ProjectBadge> {
    let projects: Vec<Value> = requests
        .iter()
        .map(|r| json!({ "id": r.id, "git_remote": r.git_remote }))
        .collect();
    let mut out = BTreeMap::new();
    for entry in service.entries() {
        let contributes = entry
            .plugin
            .manifest
            .contributes
            .iter()
            .any(|c| matches!(c, Contribution::ProjectBadge { .. }));
        if entry.plugin.state != PluginState::Enabled || !contributes {
            continue;
        }
        let params = json!({ "projects": projects });
        match service
            .request(entry.id(), methods::BADGES, params, LOAD_TIMEOUT)
            .await
        {
            Ok(answer) => out.extend(parse_badges(&answer)),
            Err(e) => tracing::debug!(plugin = entry.id(), %e, "project badges unavailable"),
        }
    }
    out
}

// ---------------------------------------------------------------------
// The store the screens read
// ---------------------------------------------------------------------

/// Which view the plugin section shows, and what is opened beside it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginNav {
    pub plugin: String,
    /// The sidebar item's view.
    pub view: String,
    /// The selected row of a table.
    pub row: Option<String>,
    /// The view shown in the pane beside the table: the row's `open`, or what
    /// an action's result asked to open.
    pub open: Option<String>,
}

/// What a table row knows that an action or a harness entry needs: the row it
/// came from, a title for the assistant, the repository Berth links a local
/// project by, and the log stream the row opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowContext {
    pub row: String,
    /// The row's first cell ("landing"): what the assistant is told it is about.
    pub title: String,
    /// `owner/repo` from the row's `repository` cell.
    pub repository: Option<String>,
    /// The log stream (`deployment/<uuid>/logs`) the row opens, when it is one.
    pub stream: Option<String>,
    /// The row's status badge says danger.
    pub failed: bool,
}

/// An action waiting for the user's confirmation (`Overlay::PluginAction`,
/// AD52), or about to run.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingAction {
    pub plugin: String,
    pub action: ActionSpec,
    pub context: Option<RowContext>,
}

/// An action whose params are only `{ "open": view }` opens that view and
/// calls no plugin (the "Logs" row button).
#[must_use]
pub fn open_only(action: &ActionSpec) -> Option<&str> {
    let map = action.params.as_object()?;
    if map.len() != 1 {
        return None;
    }
    map.get("open")?.as_str()
}

/// `owner/repo`, lower-cased, from a git remote or a repository name:
/// `https://github.com/Owner/Repo.git`, `git@github.com:owner/repo` and a bare
/// `owner/repo` all read the same. The same rule the Coolify plugin links
/// projects by.
#[must_use]
pub fn normalize_repo(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = if let Some((_, rest)) = trimmed.split_once("://") {
        rest.split_once('/')?.1
    } else if let Some((_, rest)) = trimmed.split_once('@') {
        rest.split_once(':')?.1
    } else {
        trimmed
    };
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    let (owner, repo) = (parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    Some(format!(
        "{}/{}",
        owner.to_ascii_lowercase(),
        repo.to_ascii_lowercase()
    ))
}

/// The local project whose git remote names `repository` (a row's
/// `repository` cell) and that has a folder on this machine.
#[must_use]
pub fn linked_project<'a>(
    projects: &'a [berth_core::Project],
    repository: &str,
) -> Option<&'a berth_core::Project> {
    let wanted = normalize_repo(repository)?;
    projects.iter().find(|p| {
        p.folder.is_some()
            && p.git
                .as_ref()
                .and_then(|g| g.remote.as_deref())
                .and_then(normalize_repo)
                .is_some_and(|remote| remote == wanted)
    })
}

/// The confirm as one `EffectSummary`: its verb is the button's text and its
/// lines are the plugin's lines, so the dialog's button and text are read from
/// the one value (D33).
#[must_use]
pub fn plugin_effect(confirm: &berth_plugin_protocol::Confirm) -> berth_core::EffectSummary {
    berth_core::EffectSummary {
        verb: confirm.button.clone(),
        noun_one: String::new(),
        noun_many: String::new(),
        count: 1,
        bytes: None,
        lines: confirm
            .lines
            .iter()
            .map(|text| berth_core::EffectLine {
                kind: berth_core::EffectKind::Changes,
                text: text.clone(),
            })
            .collect(),
        typed_name: None,
    }
}

/// The signals plugin screens read. Provided once from the App root (and a
/// scene root); the loaders below write them.
#[derive(Clone, Copy)]
pub struct PluginViewStore {
    pub views: Signal<BTreeMap<ViewKey, ViewState>>,
    pub logs: Signal<BTreeMap<ViewKey, LogState>>,
    pub nav: Signal<PluginNav>,
    /// Badges by project id.
    pub badges: Signal<BTreeMap<String, ProjectBadge>>,
}

/// Provide one [`PluginViewStore`] from the App root (or a scene root).
pub fn provide_plugin_view_store() -> PluginViewStore {
    use_hook(|| {
        provide_context(PluginViewStore {
            views: Signal::new(BTreeMap::new()),
            logs: Signal::new(BTreeMap::new()),
            nav: Signal::new(PluginNav::default()),
            badges: Signal::new(BTreeMap::new()),
        })
    })
}

fn toast(mut state: AppState, text: String, tone: ToastTone) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone,
        action: None,
    });
}

impl PluginViewStore {
    /// The view in a slot, if one was asked for.
    #[must_use]
    pub fn view(&self, plugin: &str, view: &str) -> Option<ViewState> {
        self.views
            .read()
            .get(&(plugin.to_string(), view.to_string()))
            .cloned()
    }

    /// Put a view in a slot (a scene's recorded answer, or a test).
    pub fn seed_view(&self, plugin: &str, view: &str, state: ViewState) {
        let mut views = self.views;
        views
            .write()
            .insert((plugin.to_string(), view.to_string()), state);
    }

    /// Put a log's lines in a slot (a scene's recorded stream).
    pub fn seed_log(&self, plugin: &str, stream: &str, log: LogState) {
        let mut logs = self.logs;
        logs.write()
            .insert((plugin.to_string(), stream.to_string()), log);
    }

    /// Load a view unless its slot already holds one. Call from an effect,
    /// never from a render body.
    pub fn ensure(&self, service: Option<&PluginsService>, plugin: &str, view: &str) {
        let key: ViewKey = (plugin.to_string(), view.to_string());
        if self.views.peek().contains_key(&key) {
            return;
        }
        self.reload(service, plugin, view);
    }

    /// Ask for a view again. What the slot held stays on screen until the
    /// answer lands; an empty slot shows Loading.
    pub fn reload(&self, service: Option<&PluginsService>, plugin: &str, view: &str) {
        let mut views = self.views;
        let key: ViewKey = (plugin.to_string(), view.to_string());
        views
            .write()
            .entry(key.clone())
            .or_insert(ViewState::Loading);
        let Some(service) = service.cloned() else {
            views.write().insert(
                key,
                ViewState::Failed("Plugins are not running here.".to_string()),
            );
            return;
        };
        crate::services::actions::spawn_job(async move {
            let loaded = load_view(&service, &key.0, &key.1).await;
            views.write().insert(key, loaded);
        });
    }

    /// Follow a stream: its lines land in `logs` as they arrive. The returned
    /// guard closes the stream when dropped.
    #[must_use]
    pub fn open_log(
        &self,
        service: Option<&PluginsService>,
        plugin: &str,
        stream: &str,
    ) -> Option<LogFollow> {
        let mut logs = self.logs;
        let key: ViewKey = (plugin.to_string(), stream.to_string());
        logs.write().insert(key.clone(), LogState::default());
        let Some(service) = service else {
            logs.write().insert(
                key,
                LogState {
                    lines: vec![(Utc::now(), "Plugins are not running here.".to_string())],
                    final_status: Some("error".to_string()),
                    dropped: 0,
                    version: 1,
                },
            );
            return None;
        };
        let follow = follow_log(service, plugin, stream);
        let mut rx = follow.state.clone();
        // Scoped on purpose: it follows a stream the `LogFollow` guard owns and ends with it.
        spawn(async move {
            while rx.changed().await.is_ok() {
                let state = rx.borrow_and_update().clone();
                logs.write().insert(key.clone(), state);
            }
        });
        Some(follow)
    }

    /// Run a confirmed action, then refresh what it invalidated and open what
    /// it asked to open. The result's message is a toast.
    pub fn run_action(
        &self,
        service: Option<&PluginsService>,
        state: AppState,
        pending: PendingAction,
        chosen: BTreeMap<String, bool>,
    ) {
        let Some(service) = service.cloned() else {
            toast(
                state,
                "Plugins are not running here.".to_string(),
                ToastTone::Danger,
            );
            return;
        };
        let store = *self;
        crate::services::actions::spawn_job(async move {
            let plugin = pending.plugin.clone();
            match invoke_action(&service, &plugin, &pending.action, &chosen).await {
                Ok(result) => {
                    if !result.message.is_empty() {
                        toast(state, result.message.clone(), ToastTone::Success);
                    }
                    for view in &result.invalidate {
                        store.reload(Some(&service), &plugin, view);
                    }
                    if let Some(open) = result.open {
                        let mut nav = store.nav;
                        let mut next = nav.peek().clone();
                        next.row = pending.context.as_ref().map(|c| c.row.clone());
                        next.open = Some(open);
                        nav.set(next);
                    }
                }
                Err(reason) => toast(state, reason, ToastTone::Danger),
            }
        });
    }

    /// Reload every loaded view of `plugin` (the plugin said `view/invalidate`).
    pub fn reload_plugin(&self, service: &PluginsService, plugin: &str) {
        let keys: Vec<String> = self
            .views
            .peek()
            .keys()
            .filter(|(p, _)| p == plugin)
            .map(|(_, v)| v.clone())
            .collect();
        for view in keys {
            self.reload(Some(service), plugin, &view);
        }
    }
}

/// Carry out a palette command: show the view, or invoke the action it names.
pub fn run_command(state: &mut AppState, command: &PluginCommand) {
    let Some(store) = try_consume_context::<PluginViewStore>() else {
        return;
    };
    match &command.kind {
        CommandKind::Open { view } => {
            let mut nav = store.nav;
            nav.set(PluginNav {
                plugin: command.plugin.clone(),
                view: view.clone(),
                row: None,
                open: None,
            });
            state.section.set(crate::state::Section::Plugin);
        }
        CommandKind::Invoke { action } => {
            let service = try_consume_context::<Option<PluginsService>>().flatten();
            let pending = PendingAction {
                plugin: command.plugin.clone(),
                action: ActionSpec {
                    id: action.clone(),
                    label: command.title.clone(),
                    danger: false,
                    params: json!({}),
                    confirm: None,
                    options: Vec::new(),
                },
                context: None,
            };
            store.run_action(service.as_ref(), *state, pending, BTreeMap::new());
        }
    }
}

/// Reload a plugin's views when it says `view/invalidate`. One task per
/// running plugin, ending with the scope that started it.
pub fn watch_invalidations(store: PluginViewStore, service: PluginsService, plugin: String) {
    // Scoped on purpose: one task per mounted view, ending with it.
    spawn(async move {
        let Some(mut notes) = service.notifications(&plugin) else {
            return;
        };
        loop {
            match notes.recv().await {
                Ok(note) if note.method == methods::INVALIDATE => {
                    store.reload_plugin(&service, &plugin);
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Keep the project badges current: asked for when the projects with a git
/// remote or the set of enabled plugins change, and again every minute. Call
/// from the App root, after the plugins context exists.
pub fn use_plugin_badges(state: AppState, store: PluginViewStore) {
    let service = try_use_context::<Option<PluginsService>>().flatten();
    let entries = use_context::<crate::services::plugins::PluginsView>().entries;
    // What the badges depend on, compared by value so a container event that
    // changes nothing here asks nothing.
    let wanted = use_memo(move || {
        let requests = badge_requests(&state.projects.read());
        let plugins: Vec<String> = entries
            .read()
            .iter()
            .filter(|e| e.plugin.state == PluginState::Enabled)
            .map(|e| e.id().to_string())
            .collect();
        (requests, plugins)
    });
    let _badges = use_resource(move || {
        let (requests, plugins) = wanted.read().clone();
        let service = service.clone();
        let mut badges = store.badges;
        async move {
            let Some(service) = service else { return };
            if plugins.is_empty() || requests.is_empty() {
                badges.set(BTreeMap::new());
                return;
            }
            loop {
                badges.set(load_badges(&service, &requests).await);
                tokio::time::sleep(BADGE_REFRESH).await;
            }
        }
    });
}

#[cfg(test)]
#[path = "plugin_views_tests.rs"]
mod tests;
