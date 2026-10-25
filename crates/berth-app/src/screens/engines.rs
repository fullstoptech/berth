//! Engines (P4-T6, frames "Engines / dashboard / 1440" and "Engines /
//! starting / 1440"): the list of engines on the left, the dashboard of the
//! selected one on the right ([`super::engine_dashboard`]).
//!
//! A row says what the engine is (Colima version, Docker version, VM size,
//! containers running), and offers the one action that fits its state:
//! Stop (asks first, naming the containers that stop), Start, or Retry for
//! an engine that does not answer. Colima commands go through the actions
//! service and `ColimaControl`; a running one draws its progress in the row.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;

use berth_core::{Engine, EngineId, EngineKind, EngineState, RemoteTransport};

use crate::services::actions::{with_view, ActionsService};
use crate::services::colima::{self, RunView};
use crate::services::confirm;
use crate::services::engine_dashboard::DashboardStore;
use crate::services::engines::EngineLink;
use crate::state::{AppState, Overlay, PendingKey, PendingState, Selection, Toast, ToastTone};
use crate::ui::engine_switcher::engine_dot;
use crate::ui::sidebar_item::Dot;
use crate::ui::{Button, ButtonVariant, ControlSize, EmptyState};

/// The screen's CSS, rendered with it (tokens only).
pub const ENGINES_CSS: &str = include_str!("../../assets/css/engines.css");

/// A frame scene's Colima run: scenes run no service, so the run the frame
/// shows ("Starting Colima 40%") is handed to the screen instead.
#[derive(Clone, PartialEq)]
pub struct SceneRun {
    pub engine: EngineId,
    pub view: RunView,
}

// ---------------------------------------------------------------------
// Row model (pure)
// ---------------------------------------------------------------------

/// The one button of a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowAction {
    /// "Stop engine…": asks first.
    Stop,
    Start,
    /// "Retry" on an engine that does not answer.
    Retry,
    /// A Colima command is running; the button says what, disabled.
    Busy(&'static str),
    /// Nothing to offer (not supported, or not a Colima engine that runs).
    None,
}

/// What a row says, line by line.
#[derive(Debug, Clone, PartialEq)]
pub struct RowModel {
    pub name: String,
    /// "Colima 0.10.3 · Docker 28.4.0 · aarch64", "SSH · deploy@host".
    pub meta: String,
    /// "15 of 23 running", "7 of 9 running · last seen 4 min ago".
    pub counts: Option<String>,
    /// "4 CPU · 8 GB · 60 GB".
    pub sizes: Option<String>,
    /// Why the engine does not answer (danger tone).
    pub problem: Option<String>,
    pub action: RowAction,
}

/// "aarch64" as Colima writes it.
#[must_use]
pub fn arch_text(arch: &berth_core::Arch) -> String {
    match arch {
        berth_core::Arch::Aarch64 => "aarch64".to_string(),
        berth_core::Arch::X86_64 => "x86_64".to_string(),
        berth_core::Arch::Other(other) => other.clone(),
    }
}

fn runtime_text(runtime: berth_core::Runtime) -> &'static str {
    match runtime {
        berth_core::Runtime::Docker => "docker",
        berth_core::Runtime::Containerd => "containerd",
        berth_core::Runtime::Incus => "incus",
    }
}

/// A VM size as Colima was configured with it: a whole number of GiB reads
/// "60 GB" (the way the frames and `colima list` say it), anything else is
/// written out in decimal units.
#[must_use]
pub fn size_text(size: bytesize::ByteSize) -> String {
    const GIB: u64 = 1 << 30;
    let bytes = size.as_u64();
    if bytes > 0 && bytes.is_multiple_of(GIB) {
        format!("{} GB", bytes / GIB)
    } else {
        berth_core::fmt::bytes(size)
    }
}

/// "4 CPU · 8 GB · 60 GB" of a VM.
#[must_use]
pub fn sizes_text(vm: &berth_core::VmResources) -> String {
    format!(
        "{} CPU \u{b7} {} \u{b7} {}",
        vm.cpus,
        size_text(vm.memory),
        size_text(vm.disk)
    )
}

fn transport_text(transport: &RemoteTransport) -> String {
    match transport {
        RemoteTransport::Ssh { host, user, port } => {
            let who = user.as_ref().map_or_else(String::new, |u| format!("{u}@"));
            let at = port.map_or_else(String::new, |p| format!(":{p}"));
            format!("SSH \u{b7} {who}{host}{at}")
        }
        RemoteTransport::Tcp { url, .. } => format!("TCP \u{b7} {}", url.authority()),
    }
}

fn meta_text(engine: &Engine, colima_version: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();
    match &engine.kind {
        EngineKind::Colima { .. } => {
            parts.push(colima_version.map_or_else(
                || "Colima".to_string(),
                |version| format!("Colima {version}"),
            ));
            match &engine.state {
                EngineState::Starting => parts.push("starting".to_string()),
                EngineState::Stopping => parts.push("stopping".to_string()),
                _ => {
                    if let Some(docker) = &engine.docker_version {
                        parts.push(format!("Docker {docker}"));
                    }
                    if let Some(vm) = &engine.resources {
                        let unusual = vm.runtime != berth_core::Runtime::Docker;
                        parts.push(if unusual {
                            runtime_text(vm.runtime).to_string()
                        } else {
                            arch_text(&vm.arch)
                        });
                    }
                }
            }
        }
        EngineKind::Remote { transport } => {
            parts.push(transport_text(transport));
        }
        EngineKind::Socket { path, context } => {
            parts.push(format!("Socket \u{b7} {}", path.display()));
            if let Some(context) = context {
                parts.push(format!("context {context}"));
            }
        }
    }
    parts.join(" \u{b7} ")
}

fn running_text(engine: &Engine) -> String {
    format!(
        "{} of {} running",
        engine.counts.containers_running, engine.counts.containers_total
    )
}

/// What a row of `engine` says and offers. `now` ages "last seen".
#[must_use]
pub fn row_model(engine: &Engine, colima_version: Option<&str>, now: DateTime<Utc>) -> RowModel {
    let colima = matches!(engine.kind, EngineKind::Colima { .. });
    let sizes = engine.resources.as_ref().map(sizes_text);
    let meta = meta_text(engine, colima_version);
    let (counts, problem, action, sizes) = match &engine.state {
        EngineState::Running => (
            Some(running_text(engine)),
            None,
            if colima {
                RowAction::Stop
            } else {
                RowAction::None
            },
            sizes,
        ),
        EngineState::Stopped => (
            Some(format!("0 of {} running", engine.counts.containers_total)),
            None,
            RowAction::Start,
            sizes,
        ),
        EngineState::Starting => (None, None, RowAction::Busy("Starting\u{2026}"), sizes),
        EngineState::Stopping => (None, None, RowAction::Busy("Stopping\u{2026}"), sizes),
        EngineState::Unreachable { reason, last_seen } => {
            let seen = last_seen.map_or_else(
                || "not reached yet".to_string(),
                |at| format!("last seen {}", berth_core::fmt::age(at, now)),
            );
            let counts = if engine.counts.containers_total > 0 {
                format!("{} \u{b7} {seen}", running_text(engine))
            } else {
                seen
            };
            (Some(counts), Some(reason.clone()), RowAction::Retry, None)
        }
        EngineState::Unsupported { reason } => (Some(reason.clone()), None, RowAction::None, sizes),
    };
    RowModel {
        name: engine.name.clone(),
        meta,
        counts,
        sizes,
        problem,
        action,
    }
}

/// The engine shown on the right: the selected one, else the scoped one,
/// else the first.
#[must_use]
pub fn selected_engine(
    engines: &[Engine],
    selection: &Selection,
    scope: &crate::state::EngineScope,
) -> Option<EngineId> {
    if let Selection::Engine(id) = selection {
        if engines.iter().any(|e| e.id == *id) {
            return Some(id.clone());
        }
    }
    if let crate::state::EngineScope::One(id) = scope {
        if engines.iter().any(|e| e.id == *id) {
            return Some(id.clone());
        }
    }
    engines.first().map(|e| e.id.clone())
}

/// Is a Colima command (or a retry) running for any engine?
#[must_use]
pub fn any_running(pending: &std::collections::BTreeMap<PendingKey, PendingState>) -> bool {
    pending.iter().any(|(key, value)| {
        matches!(key.subject, berth_core::Subject::Engine(_))
            && matches!(value, PendingState::Running { .. })
    })
}

// ---------------------------------------------------------------------
// Actions (shared with the dashboard)
// ---------------------------------------------------------------------

fn toast(state: &mut AppState, text: String) {
    state.toasts.write().push(Toast {
        id: uuid::Uuid::now_v7(),
        text,
        tone: ToastTone::Danger,
        action: None,
    });
}

/// Start a stopped Colima engine (nothing stops, so no confirm).
pub fn start_engine(state: &mut AppState, actions: Option<&ActionsService>, engine: &Engine) {
    let Some(actions) = actions else { return };
    if let Err(reason) = actions.engine(state, engine, berth_core::ColimaOp::Start) {
        toast(state, reason);
    }
}

/// Ask before stopping or restarting a running engine: the dialog names the
/// containers that stop.
pub fn ask(state: &mut AppState, engine: &Engine, op: berth_core::ColimaOp) {
    let running: Vec<berth_core::Container> = state
        .containers
        .peek()
        .iter()
        .filter(|c| c.engine == engine.id && c.state == berth_core::ContainerState::Running)
        .cloned()
        .collect();
    let refs: Vec<&berth_core::Container> = running.iter().collect();
    match confirm::colima_model(engine, op, &refs, &[]) {
        Some(model) => state.overlays.write().push(Overlay::Confirm(model)),
        None => toast(state, "Only Colima engines can be stopped here".to_string()),
    }
}

/// Try an engine that did not answer again. The row says "Retrying…" until
/// the engine's next answer or failure arrives (the applier clears it).
pub fn retry(state: &mut AppState, link: Option<&EngineLink>, engine: &Engine) {
    let key = PendingKey {
        subject: berth_core::Subject::Engine(engine.id.clone()),
        action: "engine.retry",
    };
    let Some(link) = link else {
        toast(state, "Engines cannot be retried here".to_string());
        return;
    };
    state.pending.write().insert(
        key.clone(),
        PendingState::Running {
            label: "Retrying\u{2026}".to_string(),
        },
    );
    link.connect(engine.clone(), None);
}

fn run_of(
    scene: Option<&SceneRun>,
    actions: Option<&ActionsService>,
    id: &EngineId,
) -> Option<RunView> {
    scene
        .filter(|s| s.engine == *id)
        .map(|s| s.view.clone())
        .or_else(|| actions.and_then(|a| a.engine_run(id)))
}

// ---------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------

/// Is the app on mock data (no real Colima to ask)?
fn on_mock() -> bool {
    std::env::var("BERTH_MOCK").is_ok_and(|v| v == "1") || std::env::var_os("BERTH_SCENE").is_some()
}

#[component]
pub fn EnginesScreen() -> Element {
    let state = use_context::<AppState>();
    let store = try_use_context::<DashboardStore>();
    use_hook(|| {
        if let Some(store) = store {
            store.ensure_version(on_mock());
        }
    });
    // Elapsed seconds move while a Colima command runs.
    let mut tick = use_signal(|| 0_u32);
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if any_running(&state.pending.peek()) {
                tick += 1;
            }
        }
    });
    let _ = tick.read();

    let engines = state.engines.read().clone();
    let selected = selected_engine(
        &engines,
        &state.selection.read(),
        &state.engine_scope.read(),
    );
    let version = store.and_then(|s| s.colima_version.read().clone());
    let now = Utc::now();
    let mut add_state = state;
    rsx! {
        style { "{ENGINES_CSS}" }
        style { "{crate::screens::projects::PROJECTS_CSS}" }
        section { class: "engines-screen", "aria-label": "Engines",
            aside { class: "engines-list-col",
                div { class: "engines-list-head",
                    h1 { class: "engines-title", "Engines" }
                    Button {
                        label: "Add engine".to_string(),
                        variant: ButtonVariant::Secondary,
                        size: ControlSize::Md,
                        onclick: move |_| add_state.overlays.write().push(Overlay::AddEngine),
                    }
                }
                if engines.is_empty() {
                    EmptyState {
                        title: "No engines found".to_string(),
                        body: "Install Colima or start Docker, and Berth finds the engine."
                            .to_string(),
                        icon: "server",
                    }
                }
                ul { class: "engines-list", role: "list",
                    for engine in engines.iter() {
                        li { key: "{engine.id.0}",
                            EngineRow {
                                engine: engine.clone(),
                                selected: selected.as_ref() == Some(&engine.id),
                                colima_version: version.clone(),
                                now,
                            }
                        }
                    }
                }
            }
            div { class: "engines-detail",
                if let Some(id) = selected {
                    super::engine_dashboard::EngineDashboard { key: "{id.0}", id }
                }
            }
        }
    }
}

#[component]
fn EngineRow(
    engine: Engine,
    selected: bool,
    colima_version: Option<String>,
    now: DateTime<Utc>,
) -> Element {
    let mut state = use_context::<AppState>();
    let actions = try_use_context::<ActionsService>();
    let link = try_use_context::<Option<EngineLink>>().flatten();
    let scene = try_use_context::<SceneRun>();
    let model = row_model(&engine, colima_version.as_deref(), now);
    let dot = engine_dot(&engine.state);
    let busy_run = match engine.state {
        EngineState::Starting | EngineState::Stopping => {
            run_of(scene.as_ref(), actions.as_ref(), &engine.id)
        }
        _ => None,
    };
    let retrying = state.pending.read().iter().any(|(key, value)| {
        key.subject == berth_core::Subject::Engine(engine.id.clone())
            && key.action == "engine.retry"
            && matches!(value, PendingState::Running { .. })
    });
    let class = if selected {
        "engine-row engine-row-selected"
    } else {
        "engine-row"
    };
    let select_id = engine.id.clone();
    let act_engine = engine.clone();
    let (label, variant, disabled, loading, reason) = match &model.action {
        RowAction::Stop => (
            "Stop engine\u{2026}",
            ButtonVariant::Secondary,
            false,
            false,
            with_view(&state, |view| {
                crate::services::actions::disabled_reason(
                    &berth_core::Subject::Engine(engine.id.clone()),
                    "engine.stop",
                    view,
                )
            }),
        ),
        RowAction::Start => (
            "Start engine",
            ButtonVariant::Secondary,
            false,
            false,
            with_view(&state, |view| {
                crate::services::actions::disabled_reason(
                    &berth_core::Subject::Engine(engine.id.clone()),
                    "engine.start",
                    view,
                )
            }),
        ),
        RowAction::Retry => ("Retry", ButtonVariant::Secondary, retrying, retrying, None),
        RowAction::Busy(text) => (*text, ButtonVariant::Secondary, true, false, None),
        RowAction::None => ("", ButtonVariant::Secondary, true, false, None),
    };
    let show_button = model.action != RowAction::None;
    let action = model.action.clone();
    rsx! {
        div {
            class,
            role: "button",
            tabindex: "0",
            "aria-pressed": selected.to_string(),
            "aria-label": "Engine {model.name}",
            onclick: move |_| state.selection.set(Selection::Engine(select_id.clone())),
            span { class: "engine-row-dot",
                Dot { spec: dot }
            }
            div { class: "engine-row-text",
                div { class: "engine-row-name", "{model.name}" }
                div { class: "engine-row-line", "{model.meta}" }
                if let Some(run) = &busy_run {
                    div { class: "engine-row-line",
                        "{running_label(&engine.state)} {run.elapsed_s.floor()} s"
                    }
                } else if let Some(counts) = &model.counts {
                    div { class: "engine-row-line", "{counts}" }
                }
                if let Some(sizes) = &model.sizes {
                    if busy_run.is_none() {
                        div { class: "engine-row-line", "{sizes}" }
                    }
                }
                if let Some(problem) = &model.problem {
                    div { class: "engine-row-line engine-row-problem", "{problem}" }
                }
                if let Some(run) = busy_run {
                    RunMeter { run, compact: true }
                }
            }
            if show_button {
                div {
                    class: "engine-row-action",
                    onclick: move |e| e.stop_propagation(),
                    Button {
                        label: label.to_string(),
                        variant,
                        size: ControlSize::Sm,
                        disabled,
                        loading,
                        disabled_reason: reason,
                        onclick: move |_| match action {
                            RowAction::Stop => ask(&mut state, &act_engine, berth_core::ColimaOp::Stop),
                            RowAction::Start => start_engine(&mut state, actions.as_ref(), &act_engine),
                            RowAction::Retry => retry(&mut state, link.as_ref(), &act_engine),
                            RowAction::Busy(_) | RowAction::None => {}
                        },
                    }
                }
            }
        }
    }
}

fn running_label(state: &EngineState) -> &'static str {
    match state {
        EngineState::Stopping => "Stopping\u{2026}",
        _ => "Starting\u{2026}",
    }
}

/// "Starting Colima        40%", the bar, and "8 s of about 20 s" (the row
/// form) or "Step 3 of 5 · 8 s of about 20 s" (the dashboard form).
#[component]
pub fn RunMeter(run: RunView, #[props(default)] compact: bool) -> Element {
    let percent = run.percent();
    let class = if compact {
        "run-meter run-meter-compact"
    } else {
        "run-meter"
    };
    let text = if compact {
        format!(
            "{} s of about {} s",
            run.elapsed_s.floor(),
            colima::expected_secs(run.action)
        )
    } else {
        run.progress_text()
    };
    rsx! {
        div { class,
            div { class: "run-meter-head",
                span { class: "run-meter-title", "{colima::run_title(run.action)}" }
                span { class: "run-meter-percent", "{percent}%" }
            }
            div {
                class: "run-meter-bar",
                role: "progressbar",
                "aria-valuemin": "0",
                "aria-valuemax": "100",
                "aria-valuenow": "{percent}",
                "aria-label": "{colima::run_title(run.action)}",
                span { class: "run-meter-fill", style: "width: {percent}%;" }
            }
            div { class: "run-meter-text", "{text}" }
        }
    }
}

#[cfg(test)]
#[path = "engines_tests.rs"]
mod tests;
