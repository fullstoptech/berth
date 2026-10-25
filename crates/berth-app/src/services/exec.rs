//! Terminal sessions (P3-T6, PLAN D41, D42, §7.5, ADR 0004).
//!
//! One slot per terminal tab. A slot owns what the web bundle must not: the
//! spec (container, shell, user, working directory), the status, a bounded
//! scrollback and the viewers (a mounted xterm host in some window). The
//! xterm instance only draws; every byte goes through here.
//!
//! The shell probe lives here ([`probe`]): `bash`, then `sh`; a shell the
//! user picked is tried alone. "Executable file not found" for every
//! candidate is the no-shell state, never an error. Docker reports a missing
//! shell either as a failed start or as text on the stream, so the first
//! output chunk is read before a session is called open.
//!
//! Nothing here logs or stores terminal bytes, commands or environments: a
//! `cat .env` in a shell must never leave the pane. The one Berth activity
//! entry a session records names the container and the outcome only.
//!
//! Threading: tasks never touch signals. They send [`UiMsg`] over a channel
//! that [`ExecCore::start`] drains on the UI side, and they feed each
//! viewer through an unbounded channel the pane reads.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use dioxus::prelude::*;
use futures::StreamExt;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::actions::{spawn_io, ActionsService, ContainerTarget};
use super::engines::ClientMap;
use crate::bridge::TermEvent;
use crate::state::{AppState, BottomTab, TerminalTab};

/// Shells the toolbar offers, in picker order.
pub const SHELL_CHOICES: [&str; 4] = ["bash", "sh", "zsh", "ash"];
/// Size used until the first host reports its own.
pub const DEFAULT_SIZE: (u16, u16) = (80, 24);
/// Bytes of output kept per tab for replay into a new viewer.
pub const SCROLLBACK_BYTES: usize = 256 * 1024;
/// How long the first output of a new exec is awaited. A shell prints its
/// prompt at once; Docker's "executable file not found" arrives as that
/// first chunk when it is not an error response.
const FIRST_OUTPUT_WAIT: Duration = Duration::from_millis(1500);
/// A stopped container is started first; its exec is retried this long.
const START_WAIT: Duration = Duration::from_secs(30);
const RETRY_EVERY: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------

/// The shells to try, in order: the one the user picked alone, else bash
/// then sh (D41).
#[must_use]
pub fn shell_candidates(requested: Option<&str>) -> Vec<String> {
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(shell) => vec![shell.to_string()],
        None => vec!["bash".to_string(), "sh".to_string()],
    }
}

/// Docker's wording for a shell that is not in the image.
#[must_use]
pub fn is_missing_executable(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("executable file not found")
        || (text.contains("oci runtime") && text.contains("no such file or directory"))
}

/// Docker's wording for an exec into a container that is not up (yet).
#[must_use]
pub fn is_not_running(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("is not running") || text.contains("is restarting")
}

/// Does the first chunk of an exec's output say the shell could not start?
#[must_use]
pub fn chunk_is_exec_failure(chunk: &[u8]) -> bool {
    let text = String::from_utf8_lossy(chunk).to_ascii_lowercase();
    text.contains("executable file not found") || text.contains("oci runtime exec failed")
}

/// One attempt of the probe failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// This shell is not in the image: try the next.
    NoShell,
    /// Anything else ends the probe at once.
    Other(String),
}

/// How the whole probe failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// Every candidate was missing.
    NoShell {
        tried: Vec<String>,
    },
    Failed(String),
}

/// Try [`shell_candidates`] in order with `open`. The first success wins;
/// "no such shell" moves on; any other failure stops the probe.
pub async fn probe<T, Fut>(
    requested: Option<&str>,
    mut open: impl FnMut(String) -> Fut,
) -> Result<(String, T), ProbeError>
where
    Fut: Future<Output = Result<T, OpenError>>,
{
    let candidates = shell_candidates(requested);
    for shell in &candidates {
        match open(shell.clone()).await {
            Ok(opened) => return Ok((shell.clone(), opened)),
            Err(OpenError::NoShell) => {}
            Err(OpenError::Other(message)) => return Err(ProbeError::Failed(message)),
        }
    }
    Err(ProbeError::NoShell { tried: candidates })
}

/// The tab state a failed probe leaves.
#[must_use]
pub fn status_of(error: ProbeError) -> Status {
    match error {
        ProbeError::NoShell { tried } => Status::NoShell { tried },
        ProbeError::Failed(message) => Status::Failed(message),
    }
}

/// How an engine error reads to the probe.
#[must_use]
pub fn classify(error: &berth_engine::EngineError) -> OpenError {
    let message = error.to_string();
    if is_missing_executable(&message) {
        OpenError::NoShell
    } else {
        OpenError::Other(message)
    }
}

/// "shop-api/api-1" for a Compose container (project prefix dropped from
/// the name), else the name itself.
#[must_use]
pub fn service_label(project: Option<&str>, name: &str) -> String {
    match project.and_then(|p| name.strip_prefix(p)?.strip_prefix('-').map(|r| (p, r))) {
        Some((project, rest)) if !rest.is_empty() => format!("{project}/{rest}"),
        _ => name.to_string(),
    }
}

/// Why a container cannot get a terminal, or `None`. A stopped container
/// can: it is started first.
#[must_use]
pub fn terminal_disabled_reason(state: berth_core::ContainerState) -> Option<String> {
    use berth_core::ContainerState as S;
    match state {
        S::Running | S::Created | S::Exited | S::Restarting => None,
        S::Paused => Some("Resume the container to open a terminal".to_string()),
        S::Removing => Some("The container is being removed".to_string()),
        S::Dead => Some("The container is dead".to_string()),
    }
}

/// Split what the palette typed into docker's arguments: spaces separate,
/// single and double quotes group, a backslash escapes the next character
/// outside single quotes.
#[must_use]
pub fn split_args(typed: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = typed.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') | (None, '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                    started = true;
                }
            }
            (Some(_), c) => current.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, c) => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

/// A process' lines become terminal text: xterm needs CR LF without a TTY
/// to do it.
#[must_use]
pub fn terminal_line(text: &str) -> Vec<u8> {
    format!("{text}\r\n").into_bytes()
}

/// Bounded output history, cut at a line start so a replay does not begin
/// inside an escape sequence when it can be avoided.
#[derive(Debug, Default, Clone)]
pub struct Scrollback {
    bytes: Vec<u8>,
}

impl Scrollback {
    pub fn push(&mut self, data: &[u8]) {
        self.bytes.extend_from_slice(data);
        if self.bytes.len() > SCROLLBACK_BYTES {
            let excess = self.bytes.len() - SCROLLBACK_BYTES;
            let cut = self.bytes[excess..]
                .iter()
                .take(4096)
                .position(|b| *b == b'\n')
                .map_or(excess, |at| excess + at + 1);
            self.bytes.drain(..cut);
        }
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn clear(&mut self) {
        self.bytes.clear();
    }
}

/// The bridge id of one viewer of a tab: unique per mounted host, so two
/// windows showing one session never share an xterm instance.
#[must_use]
pub fn view_id(tab: Uuid, viewer: u64) -> String {
    format!("{tab}.{viewer}")
}

#[must_use]
pub fn parse_view_id(id: &str) -> Option<(Uuid, u64)> {
    let (tab, viewer) = id.split_once('.')?;
    Some((Uuid::parse_str(tab).ok()?, viewer.parse().ok()?))
}

// ---------------------------------------------------------------------
// Tab state
// ---------------------------------------------------------------------

/// Where a session stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Waiting for the host to report its size, or for the exec to open.
    Connecting,
    /// The container is being started first.
    Starting,
    Live,
    /// Every candidate shell is missing: the image has none.
    NoShell {
        tried: Vec<String>,
    },
    Failed(String),
    Exited,
}

impl Status {
    #[must_use]
    pub fn is_live(&self) -> bool {
        matches!(self, Status::Live)
    }

    /// The tab shows as ended (grey "Exited").
    #[must_use]
    pub fn is_ended(&self) -> bool {
        matches!(self, Status::Exited | Status::Failed(_))
    }
}

/// What a tab runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A shell in a container (D41).
    Exec(ExecSpec),
    /// `docker <args>` on this Mac against the engine (palette "Run as docker
    /// command"): no PTY, output only.
    Docker { args: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecSpec {
    pub container: berth_core::ContainerId,
    pub name: String,
    /// "shop-api/api-1": the tab's name part.
    pub label: String,
    pub image: String,
    pub shell: Option<String>,
    pub user: Option<String>,
    pub workdir: Option<String>,
    /// The container was stopped and is being started: retry while Docker
    /// says it is not running yet.
    pub wait_running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub engine: berth_core::EngineId,
    pub project: Option<berth_core::ProjectId>,
    pub kind: Kind,
}

impl Spec {
    /// The tab title: "shop-api/api-1 · bash"; the no-shell state shows the
    /// name alone; a docker command shows the command.
    #[must_use]
    pub fn title(&self, status: &Status, shell_used: Option<&str>) -> String {
        match &self.kind {
            Kind::Exec(exec) => match status {
                Status::NoShell { .. } => exec.label.clone(),
                _ => {
                    let shell = shell_used.or(exec.shell.as_deref()).unwrap_or("bash");
                    format!("{} \u{b7} {shell}", exec.label)
                }
            },
            Kind::Docker { args } => format!("docker {}", args.join(" ")),
        }
    }
}

/// Everything a pane draws, pushed to viewers on every change.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneView {
    pub title: String,
    pub status: Status,
    pub spec: Spec,
    pub session: Option<berth_core::ExecSession>,
    pub shell_used: Option<String>,
    pub generation: u32,
}

/// The label a tab strip shows: the title, with "Exited" in place of the
/// shell once the session ended ("shop-api/worker-1 · Exited").
#[must_use]
pub fn tab_label(tab: &TerminalTab) -> String {
    if !tab.exited {
        return tab.title.clone();
    }
    match tab.title.rsplit_once(" \u{b7} ") {
        Some((name, _)) => format!("{name} \u{b7} Exited"),
        None => format!("{} \u{b7} Exited", tab.title),
    }
}

// ---------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------

/// To the UI thread, for the tab strip.
#[derive(Debug, Clone, PartialEq)]
pub enum UiMsg {
    Snapshot {
        tab: Uuid,
        view: Box<PaneView>,
    },
    /// Output arrived; `seen` re-arms the notice.
    Output {
        tab: Uuid,
    },
}

/// To a mounted pane.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewMsg {
    /// The scrollback, once the host reported ready.
    Replay(Vec<u8>),
    Output(Vec<u8>),
    Snapshot(Box<PaneView>),
    Clear,
}

enum Control {
    Input(Vec<u8>),
    Resize(u16, u16),
}

/// Apply one message to the tabs. `visible` is the tab the user is looking
/// at (panel open, Terminal tab, that tab active): output there is never
/// "unread".
pub fn apply_ui(tabs: &mut [TerminalTab], visible: Option<Uuid>, msg: &UiMsg) {
    match msg {
        UiMsg::Snapshot { tab, view } => {
            if let Some(entry) = tabs.iter_mut().find(|t| t.id == *tab) {
                entry.title.clone_from(&view.title);
                entry.session.clone_from(&view.session);
                entry.exited = view.status.is_ended();
            }
        }
        UiMsg::Output { tab } => {
            if let Some(entry) = tabs.iter_mut().find(|t| t.id == *tab) {
                if visible != Some(*tab) {
                    entry.unread = true;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// The core
// ---------------------------------------------------------------------

struct Viewer {
    id: u64,
    tx: UnboundedSender<ViewMsg>,
    ready: bool,
}

struct Slot {
    spec: Spec,
    status: Status,
    size: (u16, u16),
    shell_used: Option<String>,
    session: Option<berth_core::ExecSession>,
    scrollback: Scrollback,
    viewers: Vec<Viewer>,
    control: Option<UnboundedSender<Control>>,
    cancel: Option<CancellationToken>,
    generation: u32,
    next_viewer: u64,
    unread_sent: bool,
    launched: bool,
}

impl Slot {
    fn view(&self) -> PaneView {
        PaneView {
            title: self.spec.title(&self.status, self.shell_used.as_deref()),
            status: self.status.clone(),
            spec: self.spec.clone(),
            session: self.session.clone(),
            shell_used: self.shell_used.clone(),
            generation: self.generation,
        }
    }
}

struct Inner {
    clients: ClientMap,
    store: Arc<berth_store::Store>,
    cancel: CancellationToken,
    ui: UnboundedSender<UiMsg>,
    slots: Mutex<HashMap<Uuid, Slot>>,
}

/// What `attach` hands a pane.
pub struct Attached {
    pub viewer: u64,
    pub view: PaneView,
    pub rx: UnboundedReceiver<ViewMsg>,
}

/// The terminal sessions. Cheap to clone and `Send`: panes in any window
/// hold one. Provide it as context from the App root.
#[derive(Clone)]
pub struct ExecCore {
    inner: Arc<Inner>,
}

impl PartialEq for ExecCore {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl ExecCore {
    /// A core plus the receiver of its [`UiMsg`]s. [`ExecCore::start`] wires
    /// the receiver to the signals; tests drain it by hand.
    pub fn new(
        clients: ClientMap,
        store: Arc<berth_store::Store>,
        cancel: CancellationToken,
    ) -> (Self, UnboundedReceiver<UiMsg>) {
        let (ui, rx) = unbounded_channel();
        let core = Self {
            inner: Arc::new(Inner {
                clients,
                store,
                cancel,
                ui,
                slots: Mutex::new(HashMap::new()),
            }),
        };
        (core, rx)
    }

    /// Start from the App root (inside `use_hook`): builds the core and
    /// spawns the Dioxus task that keeps the tab strip's state current.
    pub fn start(
        mut state: AppState,
        view: TerminalView,
        clients: ClientMap,
        store: Arc<berth_store::Store>,
        cancel: CancellationToken,
    ) -> Self {
        let (core, mut rx) = Self::new(clients, store, cancel);
        let seen = core.clone();
        spawn(async move {
            while let Some(msg) = rx.recv().await {
                let visible = visible_tab(&state, &view);
                apply_ui(&mut state.bottom.write().terminals, visible, &msg);
                if let UiMsg::Output { tab } = msg {
                    if visible == Some(tab) {
                        seen.seen(tab);
                    }
                }
            }
        });
        core
    }

    fn slots(&self) -> MutexGuard<'_, HashMap<Uuid, Slot>> {
        match self.inner.slots.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Add a tab's slot. The session starts when a pane reports ready, so
    /// the exec is sized from the real host.
    pub fn register(&self, tab: Uuid, spec: Spec) -> PaneView {
        let slot = Slot {
            spec,
            status: Status::Connecting,
            size: DEFAULT_SIZE,
            shell_used: None,
            session: None,
            scrollback: Scrollback::default(),
            viewers: Vec::new(),
            control: None,
            cancel: None,
            generation: 0,
            next_viewer: 0,
            unread_sent: false,
            launched: false,
        };
        let view = slot.view();
        self.slots().insert(tab, slot);
        view
    }

    /// Start a session from a scene or test with its status and output
    /// already in place and no engine behind it.
    pub fn seed(&self, tab: Uuid, spec: Spec, status: Status, shell: Option<&str>, output: &[u8]) {
        let mut slot = Slot {
            spec,
            status,
            size: DEFAULT_SIZE,
            shell_used: shell.map(str::to_string),
            session: None,
            scrollback: Scrollback::default(),
            viewers: Vec::new(),
            control: None,
            cancel: None,
            generation: 1,
            next_viewer: 0,
            unread_sent: false,
            launched: true,
        };
        slot.scrollback.push(output);
        self.slots().insert(tab, slot);
    }

    /// The size the host last reported (80x24 until one does).
    #[must_use]
    pub fn size(&self, tab: Uuid) -> Option<(u16, u16)> {
        self.slots().get(&tab).map(|s| s.size)
    }

    #[must_use]
    pub fn view(&self, tab: Uuid) -> Option<PaneView> {
        self.slots().get(&tab).map(Slot::view)
    }

    /// Mount a viewer: the pane gets the current view and a channel. The
    /// replay arrives once the host says it is ready.
    pub fn attach(&self, tab: Uuid) -> Option<Attached> {
        let mut slots = self.slots();
        let slot = slots.get_mut(&tab)?;
        let (tx, rx) = unbounded_channel();
        let viewer = slot.next_viewer;
        slot.next_viewer += 1;
        slot.viewers.push(Viewer {
            id: viewer,
            tx,
            ready: false,
        });
        Some(Attached {
            viewer,
            view: slot.view(),
            rx,
        })
    }

    pub fn detach(&self, tab: Uuid, viewer: u64) {
        if let Some(slot) = self.slots().get_mut(&tab) {
            slot.viewers.retain(|v| v.id != viewer);
        }
    }

    /// A message from an xterm host (the window's bridge pump calls this).
    pub fn term_event(&self, event: TermEvent) {
        let Some((tab, viewer)) = parse_view_id(event.id()) else {
            return;
        };
        match event {
            TermEvent::Ready { .. } => {
                let launch = {
                    let mut slots = self.slots();
                    let Some(slot) = slots.get_mut(&tab) else {
                        return;
                    };
                    if let Some(v) = slot.viewers.iter_mut().find(|v| v.id == viewer) {
                        v.ready = true;
                        let replay = slot.scrollback.bytes().to_vec();
                        if !replay.is_empty() {
                            let _ = v.tx.send(ViewMsg::Replay(replay));
                        }
                    }
                    !slot.launched
                };
                if launch {
                    self.inner.launch(tab);
                }
            }
            TermEvent::Input { data, .. } => {
                if let Some(slot) = self.slots().get(&tab) {
                    if let Some(control) = slot.control.as_ref().filter(|_| slot.status.is_live()) {
                        let _ = control.send(Control::Input(data));
                    }
                }
            }
            TermEvent::Resize { cols, rows, .. } => {
                if cols == 0 || rows == 0 {
                    return;
                }
                if let Some(slot) = self.slots().get_mut(&tab) {
                    slot.size = (cols, rows);
                    if let Some(control) = slot.control.as_ref().filter(|_| slot.status.is_live()) {
                        let _ = control.send(Control::Resize(cols, rows));
                    }
                }
            }
            TermEvent::Title { .. } => {}
        }
    }

    /// The tab was looked at: re-arm the unread notice.
    pub fn seen(&self, tab: Uuid) {
        if let Some(slot) = self.slots().get_mut(&tab) {
            slot.unread_sent = false;
        }
    }

    /// Start the session again (Reconnect, or after a toolbar change).
    pub fn reconnect(&self, tab: Uuid) {
        if self.slots().contains_key(&tab) {
            self.inner.launch(tab);
        }
    }

    /// Change the toolbar's shell, user or working directory and reconnect.
    pub fn configure(&self, tab: Uuid, change: impl FnOnce(&mut ExecSpec)) -> Option<PaneView> {
        {
            let mut slots = self.slots();
            let slot = slots.get_mut(&tab)?;
            let Kind::Exec(exec) = &mut slot.spec.kind else {
                return None;
            };
            change(exec);
            exec.wait_running = false;
        }
        self.inner.launch(tab);
        self.view(tab)
    }

    /// Wipe the pane and its replay history.
    pub fn clear(&self, tab: Uuid) {
        if let Some(slot) = self.slots().get_mut(&tab) {
            slot.scrollback.clear();
            for viewer in &slot.viewers {
                let _ = viewer.tx.send(ViewMsg::Clear);
            }
        }
    }

    /// End the session and forget the tab.
    pub fn close(&self, tab: Uuid) {
        if let Some(slot) = self.slots().remove(&tab) {
            if let Some(cancel) = slot.cancel {
                cancel.cancel();
            }
        }
    }

    /// The `DOCKER_HOST` an engine answers on (Open in Terminal app).
    #[must_use]
    pub fn docker_host(&self, engine: &berth_core::EngineId) -> Option<String> {
        let map = self.inner.clients.lock().ok()?;
        map.get(engine).map(|c| c.docker_host().to_string())
    }

    /// The spec of a tab, to open a sibling (Split).
    #[must_use]
    pub fn spec(&self, tab: Uuid) -> Option<Spec> {
        self.slots().get(&tab).map(|s| s.spec.clone())
    }
}

impl Inner {
    fn slots(&self) -> MutexGuard<'_, HashMap<Uuid, Slot>> {
        match self.slots.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Tell every viewer and the tab strip what the slot looks like now.
    fn publish(&self, tab: Uuid, slot: &Slot) {
        let view = Box::new(slot.view());
        for viewer in &slot.viewers {
            let _ = viewer.tx.send(ViewMsg::Snapshot(view.clone()));
        }
        let _ = self.ui.send(UiMsg::Snapshot { tab, view });
    }

    /// Change a slot unless a newer session replaced the caller's.
    fn update(&self, tab: Uuid, generation: u32, change: impl FnOnce(&mut Slot)) -> bool {
        let mut slots = self.slots();
        let Some(slot) = slots.get_mut(&tab) else {
            return false;
        };
        if slot.generation != generation {
            return false;
        }
        change(slot);
        self.publish(tab, slot);
        true
    }

    fn output(&self, tab: Uuid, generation: u32, bytes: &[u8]) {
        let mut slots = self.slots();
        let Some(slot) = slots.get_mut(&tab) else {
            return;
        };
        if slot.generation != generation || bytes.is_empty() {
            return;
        }
        slot.scrollback.push(bytes);
        for viewer in slot.viewers.iter().filter(|v| v.ready) {
            let _ = viewer.tx.send(ViewMsg::Output(bytes.to_vec()));
        }
        if !slot.unread_sent {
            slot.unread_sent = true;
            let _ = self.ui.send(UiMsg::Output { tab });
        }
    }

    /// A new session for the slot: the old one is cancelled, the status
    /// goes back to Connecting and a task opens the exec.
    fn launch(self: &Arc<Self>, tab: Uuid) {
        let (generation, token, control_rx) = {
            let mut slots = self.slots();
            let Some(slot) = slots.get_mut(&tab) else {
                return;
            };
            if let Some(old) = slot.cancel.take() {
                old.cancel();
            }
            if slot.launched && !slot.scrollback.bytes().is_empty() {
                slot.scrollback
                    .push(b"\r\n\x1b[2m\xe2\x80\x94 reconnecting \xe2\x80\x94\x1b[0m\r\n");
            }
            slot.launched = true;
            slot.generation += 1;
            slot.status = match &slot.spec.kind {
                Kind::Exec(exec) if exec.wait_running => Status::Starting,
                _ => Status::Connecting,
            };
            slot.shell_used = None;
            slot.session = None;
            let token = self.cancel.child_token();
            slot.cancel = Some(token.clone());
            let (tx, rx) = unbounded_channel();
            slot.control = Some(tx);
            self.publish(tab, slot);
            (slot.generation, token, rx)
        };
        let inner = Arc::clone(self);
        spawn_io(async move {
            inner.run(tab, generation, token, control_rx).await;
        });
    }

    async fn run(
        self: &Arc<Self>,
        tab: Uuid,
        generation: u32,
        token: CancellationToken,
        control: UnboundedReceiver<Control>,
    ) {
        let (spec, size) = {
            let slots = self.slots();
            let Some(slot) = slots.get(&tab) else { return };
            (slot.spec.clone(), slot.size)
        };
        let client = self
            .clients
            .lock()
            .ok()
            .and_then(|map| map.get(&spec.engine).cloned());
        let Some(client) = client else {
            let message = format!("Engine {} is not connected", spec.engine.0);
            self.update(tab, generation, |s| s.status = Status::Failed(message));
            return;
        };
        match &spec.kind {
            Kind::Exec(exec) => {
                self.run_exec(tab, generation, &token, control, &spec, exec, size, &client)
                    .await;
            }
            Kind::Docker { args } => {
                self.run_docker(tab, generation, &token, &client, args)
                    .await;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_exec(
        self: &Arc<Self>,
        tab: Uuid,
        generation: u32,
        token: &CancellationToken,
        mut control: UnboundedReceiver<Control>,
        spec: &Spec,
        exec: &ExecSpec,
        size: (u16, u16),
        client: &Arc<dyn berth_engine::EngineClient>,
    ) {
        let opened = open_exec(client.as_ref(), exec, size, token).await;
        let outcome = match &opened {
            Ok(_) => berth_core::Outcome::Ok,
            Err(ProbeError::NoShell { .. }) => berth_core::Outcome::Failed {
                reason: "The image has no shell".to_string(),
            },
            Err(ProbeError::Failed(reason)) => berth_core::Outcome::Failed {
                reason: reason.clone(),
            },
        };
        if token.is_cancelled() {
            return;
        }
        // The command line and environment never reach the entry.
        super::activity::record(
            &self.store,
            &super::activity::entry_for_action(
                spec.engine.clone(),
                "terminal.open",
                berth_core::Subject::Container(exec.container.clone()),
                exec.name.clone(),
                spec.project.clone(),
                outcome,
            ),
        );
        let (shell, mut handle, first) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let status = status_of(error);
                self.update(tab, generation, |s| s.status = status);
                return;
            }
        };
        let session = handle.session.clone();
        self.update(tab, generation, |s| {
            s.status = Status::Live;
            s.shell_used = Some(shell.clone());
            s.session = Some(session);
        });
        self.output(tab, generation, &first);
        loop {
            tokio::select! {
                biased;
                () = token.cancelled() => return,
                message = control.recv() => match message {
                    Some(Control::Input(bytes)) => {
                        if handle.input.send(bytes.into()).await.is_err() {
                            break;
                        }
                    }
                    Some(Control::Resize(cols, rows)) => {
                        // `ExecHandle` is not `Sync`, so its `&self` method
                        // cannot be held across an await in a spawned task;
                        // the send has room (capacity 8) and is polled once.
                        let _ = futures::FutureExt::now_or_never(handle.resize(cols, rows));
                    }
                    None => break,
                },
                chunk = handle.output.next() => match chunk {
                    Some(Ok(bytes)) => self.output(tab, generation, &bytes),
                    Some(Err(_)) | None => break,
                },
            }
        }
        self.output(
            tab,
            generation,
            b"\r\n\x1b[2m[The session ended]\x1b[0m\r\n",
        );
        self.update(tab, generation, |s| {
            s.status = Status::Exited;
            s.control = None;
        });
    }

    async fn run_docker(
        self: &Arc<Self>,
        tab: Uuid,
        generation: u32,
        token: &CancellationToken,
        client: &Arc<dyn berth_engine::EngineClient>,
        args: &[String],
    ) {
        let search = berth_sys::SearchPath::from_process()
            .with_login_shell()
            .await;
        let Some(docker) = search.find("docker") else {
            let message = "The docker command was not found on this Mac".to_string();
            self.update(tab, generation, |s| s.status = Status::Failed(message));
            return;
        };
        // docker prints no prompt and takes no input here; echo what runs.
        self.output(
            tab,
            generation,
            &terminal_line(&format!("$ docker {}", args.join(" "))),
        );
        let opts = berth_sys::SpawnOpts {
            env: vec![(
                "DOCKER_HOST".into(),
                client.docker_host().to_string().into(),
            )],
            path: Some(search.path_var()),
            ..berth_sys::SpawnOpts::default()
        };
        let os_args: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
        let mut lines = berth_sys::spawn_lines(&docker, &os_args, opts, token.clone());
        self.update(tab, generation, |s| s.status = Status::Live);
        // The command may carry secrets: one entry, no text.
        super::activity::record(
            &self.store,
            &super::activity::entry_for_action(
                client.id().clone(),
                "terminal.docker",
                berth_core::Subject::Engine(client.id().clone()),
                client.id().0.clone(),
                None,
                berth_core::Outcome::Ok,
            ),
        );
        while let Some(item) = lines.next().await {
            match item {
                Ok(berth_sys::ProcessLine::Stdout(text) | berth_sys::ProcessLine::Stderr(text)) => {
                    self.output(tab, generation, &terminal_line(&text));
                }
                Ok(berth_sys::ProcessLine::Exit(code)) => {
                    let note = match code {
                        Some(0) => "\x1b[2m[Done]\x1b[0m".to_string(),
                        Some(code) => format!("\x1b[2m[Exited with code {code}]\x1b[0m"),
                        None => "\x1b[2m[Stopped by a signal]\x1b[0m".to_string(),
                    };
                    self.output(tab, generation, &terminal_line(&note));
                }
                Err(err) => {
                    let message = err.to_string();
                    self.update(tab, generation, |s| s.status = Status::Failed(message));
                    return;
                }
            }
        }
        if !token.is_cancelled() {
            self.update(tab, generation, |s| {
                s.status = Status::Exited;
                s.control = None;
            });
        }
    }
}

/// Open the exec: probe the shells and, when the container is being
/// started, retry while Docker says it is not running.
async fn open_exec(
    client: &dyn berth_engine::EngineClient,
    exec: &ExecSpec,
    size: (u16, u16),
    token: &CancellationToken,
) -> Result<(String, berth_engine::ExecHandle, Vec<u8>), ProbeError> {
    let deadline = tokio::time::Instant::now() + START_WAIT;
    loop {
        let result = probe(exec.shell.as_deref(), |shell| {
            try_shell(client, exec, shell, size, token)
        })
        .await
        .map(|(shell, (handle, first))| (shell, handle, first));
        match result {
            Err(ProbeError::Failed(message))
                if exec.wait_running
                    && is_not_running(&message)
                    && tokio::time::Instant::now() < deadline
                    && !token.is_cancelled() =>
            {
                tokio::select! {
                    () = token.cancelled() => return Err(ProbeError::Failed(message)),
                    () = tokio::time::sleep(RETRY_EVERY) => {}
                }
            }
            other => return other,
        }
    }
}

async fn try_shell(
    client: &dyn berth_engine::EngineClient,
    exec: &ExecSpec,
    shell: String,
    size: (u16, u16),
    token: &CancellationToken,
) -> Result<(berth_engine::ExecHandle, Vec<u8>), OpenError> {
    let request = berth_core::ExecRequest {
        shell: Some(shell),
        user: exec.user.clone(),
        workdir: exec.workdir.clone(),
        cols: size.0,
        rows: size.1,
    };
    let mut handle = client
        .exec(&exec.container, request, token.clone())
        .await
        .map_err(|e| classify(&e))?;
    match tokio::time::timeout(FIRST_OUTPUT_WAIT, handle.output.next()).await {
        Ok(Some(Ok(chunk))) if chunk_is_exec_failure(&chunk) => Err(OpenError::NoShell),
        Ok(Some(Ok(chunk))) => Ok((handle, chunk.to_vec())),
        Ok(Some(Err(e))) => Err(classify(&e)),
        Ok(None) => Err(OpenError::Other(
            "The shell ended as soon as it started".to_string(),
        )),
        // A shell that prints nothing until it is typed to is still a shell.
        Err(_) => Ok((handle, Vec::new())),
    }
}

// ---------------------------------------------------------------------
// The UI side: which tab is shown, opening and closing tabs
// ---------------------------------------------------------------------

/// Which terminal the panel shows, and the one beside it when split.
/// Provided as context from the App root.
#[derive(Clone, Copy)]
pub struct TerminalView {
    pub active: Signal<Option<Uuid>>,
    pub split: Signal<Option<Uuid>>,
}

/// Create the view signals once, in the App root's body.
pub fn use_terminal_view() -> TerminalView {
    TerminalView {
        active: use_signal(|| None),
        split: use_signal(|| None),
    }
}

/// Terminal contexts for a scene rendered without the App root (SSR): a
/// core with no engines behind it and the view signals.
pub fn provide_scene_terminals() {
    let view = use_terminal_view();
    use_context_provider(|| view);
    if let Ok(store) = berth_store::Store::open_in_memory() {
        use_context_provider(|| {
            ExecCore::new(
                super::engines::client_map(),
                Arc::new(store),
                CancellationToken::new(),
            )
            .0
        });
    }
}

/// The tab the user is looking at, or `None` (panel closed, another tab,
/// or no terminal).
#[must_use]
pub fn visible_tab(state: &AppState, view: &TerminalView) -> Option<Uuid> {
    let bottom = state.bottom.peek();
    if !bottom.open || bottom.tab != BottomTab::Terminal {
        return None;
    }
    *view.active.peek()
}

/// Spec for a shell in `container`.
#[must_use]
pub fn container_spec(
    container: &berth_core::Container,
    project_name: Option<&str>,
    shell: Option<String>,
) -> Spec {
    let running = container.state == berth_core::ContainerState::Running;
    Spec {
        engine: container.engine.clone(),
        project: container.project.clone(),
        kind: Kind::Exec(ExecSpec {
            container: container.id.clone(),
            name: container.name.clone(),
            label: service_label(project_name, &container.name),
            image: container.image.reference.clone(),
            shell,
            user: None,
            workdir: None,
            wait_running: !running,
        }),
    }
}

/// Add a tab for `spec` to the bottom panel and show it. The session
/// starts once the pane is mounted and sized.
pub fn open_tab(
    state: &mut AppState,
    view: &mut TerminalView,
    core: &ExecCore,
    spec: Spec,
) -> Uuid {
    let id = Uuid::now_v7();
    let pane = core.register(id, spec);
    {
        let mut bottom = state.bottom.write();
        bottom.terminals.push(TerminalTab {
            id,
            title: pane.title,
            session: None,
            exited: false,
            unread: false,
        });
        bottom.open = true;
        bottom.tab = BottomTab::Terminal;
    }
    view.active.set(Some(id));
    id
}

/// Show a tab and clear its unread dot.
pub fn select_tab(state: &mut AppState, view: &mut TerminalView, core: &ExecCore, id: Uuid) {
    {
        let mut bottom = state.bottom.write();
        if let Some(tab) = bottom.terminals.iter_mut().find(|t| t.id == id) {
            tab.unread = false;
        }
    }
    core.seen(id);
    if *view.split.peek() == Some(id) {
        let mut split = view.split;
        split.set(None);
    }
    view.active.set(Some(id));
}

/// Close a tab: ends its session, shows a neighbour.
pub fn close_tab(state: &mut AppState, view: &mut TerminalView, core: &ExecCore, id: Uuid) {
    core.close(id);
    let next = {
        let mut bottom = state.bottom.write();
        let at = bottom.terminals.iter().position(|t| t.id == id);
        if let Some(at) = at {
            bottom.terminals.remove(at);
        }
        let neighbour = at.map(|at| at.min(bottom.terminals.len().saturating_sub(1)));
        neighbour.and_then(|i| bottom.terminals.get(i).map(|t| t.id))
    };
    if *view.split.peek() == Some(id) {
        let mut split = view.split;
        split.set(None);
    }
    if *view.active.peek() == Some(id) {
        view.active.set(next);
    }
}

/// Open a shell in one container. A stopped container is started first
/// (through the actions service, so the row shows the pending state); a
/// paused or dying one is refused with the reason.
pub fn open_container_terminal(
    state: &mut AppState,
    view: &mut TerminalView,
    core: &ExecCore,
    actions: Option<&ActionsService>,
    container: &berth_core::Container,
) -> Result<Uuid, String> {
    if let Some(reason) = terminal_disabled_reason(container.state) {
        return Err(reason);
    }
    if matches!(
        container.state,
        berth_core::ContainerState::Created | berth_core::ContainerState::Exited
    ) {
        let service =
            actions.ok_or_else(|| "Containers cannot be started right now".to_string())?;
        service.container(
            state,
            &ContainerTarget::from(container),
            berth_core::ContainerAction::Start,
        )?;
    }
    let spec = spec_in_project(state, container, None);
    Ok(open_tab(state, view, core, spec))
}

/// The project a container belongs to: its own link, else the project
/// listing it as a service container (the live list carries no link).
#[must_use]
pub fn member_project<'a>(
    projects: &'a [berth_core::Project],
    container: &berth_core::Container,
) -> Option<&'a berth_core::Project> {
    projects.iter().find(|p| {
        container.project.as_ref() == Some(&p.id)
            || p.services
                .iter()
                .any(|s| s.containers.contains(&container.id))
    })
}

/// The Compose project's name for a container: its label first, else the
/// project it was grouped into.
#[must_use]
pub fn project_name(state: &AppState, container: &berth_core::Container) -> Option<String> {
    if let Some(name) = container.compose_project.clone() {
        return Some(name);
    }
    member_project(&state.projects.peek(), container).map(|p| p.name.clone())
}

fn spec_in_project(
    state: &AppState,
    container: &berth_core::Container,
    shell: Option<String>,
) -> Spec {
    let name = project_name(state, container);
    let mut spec = container_spec(container, name.as_deref(), shell);
    if spec.project.is_none() {
        spec.project = member_project(&state.projects.peek(), container).map(|p| p.id.clone());
    }
    spec
}

/// The running containers of a project.
fn running_members<'a>(
    containers: &'a [berth_core::Container],
    project: &berth_core::Project,
) -> impl Iterator<Item = &'a berth_core::Container> + 'a {
    let ids: Vec<berth_core::ContainerId> = project
        .services
        .iter()
        .flat_map(|s| s.containers.iter().cloned())
        .collect();
    let id = project.id.clone();
    containers.iter().filter(move |c| {
        c.state == berth_core::ContainerState::Running
            && (ids.contains(&c.id) || c.project.as_ref() == Some(&id))
    })
}

/// Open a terminal for every running service of a project. `Err` when
/// nothing in it is running.
pub fn open_project_terminals(
    state: &mut AppState,
    view: &mut TerminalView,
    core: &ExecCore,
    project: &berth_core::ProjectId,
) -> Result<usize, String> {
    let found = state
        .projects
        .peek()
        .iter()
        .find(|p| p.id == *project)
        .cloned();
    let members: Vec<berth_core::Container> = match &found {
        Some(project) => running_members(&state.containers.peek(), project)
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    if members.is_empty() {
        return Err("Nothing is running in this project".to_string());
    }
    for container in &members {
        let spec = spec_in_project(state, container, None);
        open_tab(state, view, core, spec);
    }
    Ok(members.len())
}

/// Why a project cannot open its terminals, or `None`.
#[must_use]
pub fn project_terminals_reason(
    containers: &[berth_core::Container],
    project: &berth_core::Project,
) -> Option<String> {
    let any = running_members(containers, project).next().is_some();
    (!any).then(|| "Nothing is running in this project".to_string())
}

/// Run `docker <typed>` in a tab against `engine` (palette "Run as docker
/// command?"). Returns the tab, or `Err` when nothing was typed.
pub fn open_docker_tab(
    state: &mut AppState,
    view: &mut TerminalView,
    core: &ExecCore,
    engine: berth_core::EngineId,
    typed: &str,
) -> Result<Uuid, String> {
    let args = split_args(typed);
    if args.is_empty() {
        return Err("Type a docker command first".to_string());
    }
    let spec = Spec {
        engine,
        project: None,
        kind: Kind::Docker { args },
    };
    Ok(open_tab(state, view, core, spec))
}

/// The engine a docker command runs against: the scoped one, else the
/// first running engine.
#[must_use]
pub fn docker_engine(state: &AppState) -> Option<berth_core::EngineId> {
    if let crate::state::EngineScope::One(id) = &*state.engine_scope.peek() {
        return Some(id.clone());
    }
    state
        .engines
        .peek()
        .iter()
        .find(|e| e.state == berth_core::EngineState::Running)
        .map(|e| e.id.clone())
}

/// ⌘T and the row's Terminal button: the first selected container gets a
/// shell, a selected project gets one per running service. Called from
/// `keys::dispatch`; a no-op without the contexts (scenes, tests).
pub fn dispatch_open(state: &mut AppState) {
    let (Some(core), Some(mut view)) = (
        try_consume_context::<ExecCore>(),
        try_consume_context::<TerminalView>(),
    ) else {
        return;
    };
    let actions = try_consume_context::<ActionsService>();
    let selection = state.selection.peek().clone();
    let result = match selection {
        crate::state::Selection::Containers(ids) => {
            let found = ids.first().and_then(|id| {
                state
                    .containers
                    .peek()
                    .iter()
                    .find(|c| c.id == *id)
                    .cloned()
            });
            match found {
                Some(container) => {
                    open_container_terminal(state, &mut view, &core, actions.as_ref(), &container)
                        .map(|_| ())
                }
                None => return,
            }
        }
        crate::state::Selection::Project(id) => {
            open_project_terminals(state, &mut view, &core, &id).map(|_| ())
        }
        _ => return,
    };
    if let Err(reason) = result {
        tracing::debug!(%reason, "terminal refused");
    }
}

#[cfg(test)]
#[path = "exec_tests.rs"]
mod exec_tests;
