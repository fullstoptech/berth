//! The dashboard of one engine (P4-T6, frame "Engines / dashboard / 1440"):
//! the CPU and memory tiles, the engine info list, the disk bar from
//! `disk_usage`, and the engine's recent activity. While Colima starts or
//! stops it shows the run instead (frame "Engines / starting / 1440"): the
//! progress bar and the command's output. A stopped, unreachable or
//! unsupported engine says so and offers the one thing that helps.
//!
//! Reads signals and the dashboard store; every Colima command goes
//! through the actions service (`ColimaControl`), and every one that
//! changes a running engine asks first.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;

use berth_core::{Engine, EngineId, EngineKind, EngineState};

use super::engines::{arch_text, ask, retry, size_text, start_engine, RunMeter, SceneRun};
use crate::services::actions::{with_view, ActionsService};
use crate::services::colima::{self, RunView};
use crate::services::engine_dashboard::{DashboardStore, EngineSeries, UsageEntry};
use crate::services::engines::EngineLink;
use crate::services::stats::{StatsKey, StatsService};
use crate::services::Services;
use crate::sheets::edit_resources;
use crate::state::{AppState, Overlay, PendingKey, PendingState};
use crate::ui::metric_tile::{scale_for, spark_points, MetricTile, MetricTileData, STATS_CSS};
use crate::ui::usage_bar::{segments_of, used_bytes, used_of_text};
use crate::ui::{Banner, BannerTone, Button, ButtonVariant, ControlSize, EmptyState, UsageBar};

// ---------------------------------------------------------------------
// Models (pure)
// ---------------------------------------------------------------------

/// "Colima 0.10.3 · Docker 28.4.0 · 4 CPU · 8 GB · 60 GB" under the title.
#[must_use]
pub fn header_sub(engine: &Engine, colima_version: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if matches!(engine.kind, EngineKind::Colima { .. }) {
        parts.push(colima_version.map_or_else(
            || "Colima".to_string(),
            |version| format!("Colima {version}"),
        ));
    }
    if engine.state == EngineState::Running {
        if let Some(docker) = &engine.docker_version {
            parts.push(format!("Docker {docker}"));
        }
        if let Some(vm) = &engine.resources {
            parts.push(format!("{} CPU", vm.cpus));
            parts.push(size_text(vm.memory));
            parts.push(size_text(vm.disk));
        }
    }
    parts.join(" \u{b7} ")
}

/// The word in the state badge.
#[must_use]
pub fn state_word(state: &EngineState) -> &'static str {
    match state {
        EngineState::Running => "Running",
        EngineState::Stopped => "Stopped",
        EngineState::Starting => "Starting\u{2026}",
        EngineState::Stopping => "Stopping\u{2026}",
        EngineState::Unreachable { .. } => "Unreachable",
        EngineState::Unsupported { .. } => "Not supported",
    }
}

/// The label/value pairs of the Engine info list.
#[must_use]
pub fn info_rows(engine: &Engine) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    if let Some(vm) = &engine.resources {
        rows.push(("VM type", edit_resources::vm_id(&vm.vm_type)));
        rows.push(("Rosetta", if vm.rosetta { "on" } else { "off" }.to_string()));
        rows.push(("Architecture", arch_text(&vm.arch)));
    }
    if let Some(docker) = &engine.docker_version {
        rows.push(("Docker", docker.clone()));
    }
    if engine.resources.is_none() {
        if let Some(host) = &engine.docker_host {
            rows.push(("Host", host.clone()));
        }
    }
    rows
}

/// "22.5 of 60 GB used" and "5.7 GB reclaimable" ("0 B reclaimable" when
/// nothing is) from the usage and the VM disk (the usage's own total wins
/// when it has one).
#[must_use]
pub fn disk_summary(
    usage: &berth_core::DiskUsage,
    vm_disk: Option<bytesize::ByteSize>,
) -> (String, Option<String>) {
    let used = bytesize::ByteSize(used_bytes(&segments_of(usage)));
    let total = usage.disk_total.or(vm_disk);
    let head = match total {
        Some(total) => format!(
            "{} used",
            used_of_text(&berth_core::fmt::bytes(used), &size_text(total))
        ),
        None => format!("{} used", berth_core::fmt::bytes(used)),
    };
    let reclaimable = bytesize::ByteSize(
        usage.images.reclaimable.as_u64()
            + usage.volumes.reclaimable.as_u64()
            + usage.build_cache.reclaimable.as_u64()
            + usage.containers.reclaimable.as_u64(),
    );
    // "0 B reclaimable" stays on screen after a clean up: it says the run
    // worked.
    let tail = Some(format!(
        "{} reclaimable",
        berth_core::fmt::bytes(reclaimable)
    ));
    (head, tail)
}

fn split_unit(text: &str) -> (String, String) {
    match text.split_once(' ') {
        Some((value, unit)) => (value.to_string(), unit.to_string()),
        None => (text.to_string(), String::new()),
    }
}

/// The CPU and memory tiles from the engine's summed load. `reason` is why
/// there is nothing to show (engine not running).
#[must_use]
pub fn load_tiles(
    series: Option<&EngineSeries>,
    engine: &Engine,
    now: DateTime<Utc>,
    reason: Option<&str>,
) -> Vec<MetricTileData> {
    let cores = engine.resources.as_ref().map_or(0, |vm| u32::from(vm.cpus));
    let empty =
        |label: &'static str, series_no: u8, caption: String, loading: bool| MetricTileData {
            label,
            value: None,
            unit: String::new(),
            caption,
            spark: Vec::new(),
            y_max: 1.0,
            fraction: None,
            series: series_no,
            loading,
        };
    if let Some(reason) = reason {
        return vec![
            empty("CPU", 1, reason.to_string(), false),
            empty("Memory", 2, reason.to_string(), false),
        ];
    }
    let Some(series) = series.filter(|s| s.latest().is_some()) else {
        let waiting = "Waiting for the first sample".to_string();
        return vec![
            empty("CPU", 1, waiting.clone(), true),
            empty("Memory", 2, waiting, true),
        ];
    };
    let (cpu_now, mem_now) = series.latest().unwrap_or((0.0, 0.0));
    let peak = |points: &[(DateTime<Utc>, f64)]| points.iter().map(|(_, y)| *y).fold(0.0, f64::max);
    let cpu_caption = if cores > 0 {
        format!("of {cores} cores \u{b7} last 5 min")
    } else {
        "last 5 min".to_string()
    };
    let memory_total = engine.resources.as_ref().map(|vm| size_text(vm.memory));
    let mem_caption = memory_total.map_or_else(
        || "last 5 min".to_string(),
        |total| format!("of {total} \u{b7} last 5 min"),
    );
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (mem_value, mem_unit) = split_unit(&berth_core::fmt::bytes(bytesize::ByteSize(
        mem_now.max(0.0) as u64,
    )));
    vec![
        MetricTileData {
            label: "CPU",
            value: Some(format!("{cpu_now:.1}")),
            unit: "%".to_string(),
            caption: cpu_caption,
            spark: spark_points(&series.cpu, now),
            y_max: scale_for(peak(&series.cpu), 10.0),
            fraction: None,
            series: 1,
            loading: false,
        },
        MetricTileData {
            label: "Memory",
            value: Some(mem_value),
            unit: mem_unit,
            caption: mem_caption,
            spark: spark_points(&series.mem, now),
            y_max: scale_for(peak(&series.mem), 1.0),
            fraction: None,
            series: 2,
            loading: false,
        },
    ]
}

/// The Colima run the engine's pending actions describe, if any.
fn is_colima_busy(state: &EngineState) -> bool {
    matches!(state, EngineState::Starting | EngineState::Stopping)
}

fn is_scene() -> bool {
    std::env::var_os("BERTH_SCENE").is_some()
}

// ---------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------

/// One running container's stats stream, held open while the dashboard is
/// on screen so the engine's load can be summed.
#[component]
fn LoadWatch(key_for: StatsKey) -> Element {
    let service = try_use_context::<StatsService>();
    crate::services::stats::use_row_stats(service.as_ref(), Some(key_for));
    rsx! {}
}

/// Is the disk usage older than `USAGE_EVERY` (or never read)? Measured on the clock, so the
/// first tick after a long hidden spell refreshes it.
fn usage_due(last: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    last.is_none_or(|at| {
        now.saturating_duration_since(at) >= crate::services::engine_dashboard::USAGE_EVERY
    })
}

#[component]
pub fn EngineDashboard(id: EngineId, #[props(default)] tick: u32) -> Element {
    let _ = tick;
    let mut state = use_context::<AppState>();
    let store = try_use_context::<DashboardStore>();
    let services = try_use_context::<Services>();
    let actions = try_use_context::<ActionsService>();
    let stats = try_use_context::<StatsService>();
    let link = try_use_context::<Option<EngineLink>>().flatten();
    let scene = try_use_context::<SceneRun>();

    // Disk usage and the engine's load, refreshed while the dashboard is
    // open and the engine runs.
    let watched = id.clone();
    let feed_store = store;
    let feed_services = services.clone();
    let visibility = try_use_context::<crate::services::visibility::Visibility>();
    use_future(move || {
        let watched = watched.clone();
        let services = feed_services.clone();
        let stats = stats.clone();
        // The clock parks while the window is hidden and ticks at once when it shows (AD32).
        let mut pacer = crate::services::visibility::Pacer::of(
            visibility.as_ref(),
            crate::services::engine_dashboard::SERIES_EVERY,
        );
        async move {
            let mut last_usage: Option<std::time::Instant> = None;
            loop {
                let running = state
                    .engines
                    .peek()
                    .iter()
                    .any(|e| e.id == watched && e.state == EngineState::Running);
                if running {
                    if let (Some(store), Some(services), false) =
                        (feed_store, services.as_ref(), is_scene())
                    {
                        if usage_due(last_usage, std::time::Instant::now()) {
                            store.refresh_usage(services, &watched);
                            last_usage = Some(std::time::Instant::now());
                        }
                    }
                    if let (Some(store), Some(stats)) = (feed_store, stats.as_ref()) {
                        push_load(&state, store, stats, &watched);
                    }
                }
                if !pacer.tick().await {
                    break;
                }
            }
        }
    });

    let engine = state.engines.read().iter().find(|e| e.id == id).cloned();
    let Some(engine) = engine else {
        return rsx! {
            div { class: "dash",
                EmptyState {
                    title: "This engine is gone".to_string(),
                    body: "It is no longer listed.".to_string(),
                    icon: "server",
                }
            }
        };
    };
    let version = store.and_then(|s| s.colima_version.read().clone());
    let run = scene
        .as_ref()
        .filter(|s| s.engine == id)
        .map(|s| s.view.clone())
        .or_else(|| actions.as_ref().and_then(|a| a.engine_run(&id)));
    let colima = matches!(engine.kind, EngineKind::Colima { .. });
    let sub = header_sub(&engine, version.as_deref());
    let busy = is_colima_busy(&engine.state);
    let failed = run.as_ref().filter(|r| r.failure.is_some()).cloned();
    let running_containers: Vec<StatsKey> = if engine.state == EngineState::Running {
        state
            .containers
            .read()
            .iter()
            .filter(|c| c.engine == id && c.state == berth_core::ContainerState::Running)
            .map(|c| StatsKey::new(&c.engine, &c.id))
            .collect()
    } else {
        Vec::new()
    };

    let badge_class = badge_class(&engine.state);
    let dot = crate::ui::engine_dot(&engine.state);
    rsx! {
        style { "{STATS_CSS}" }
        div { class: "dash",
            for key in running_containers {
                LoadWatch { key: "{key.id.0}", key_for: key.clone() }
            }
            div { class: "dash-head",
                h2 { class: "dash-title", "{engine.name}" }
                span { class: "{badge_class}",
                    crate::ui::sidebar_item::Dot { spec: dot }
                    "{state_word(&engine.state)}"
                }
                if !sub.is_empty() {
                    span { class: "dash-sub", "{sub}" }
                }
                span { class: "dash-spacer" }
                {header_buttons(&mut state, actions.as_ref(), link.as_ref(), &engine, colima)}
            }
            if let Some(run) = failed {
                FailedRun { engine: engine.clone(), run }
            }
            if busy {
                if let Some(run) = run.filter(|r| r.failure.is_none()) {
                    RunPanel { engine: engine.clone(), run }
                } else {
                    p { class: "dash-note", "{state_word(&engine.state)}" }
                }
            } else {
                {body(&mut state, store, &engine, version.as_deref())}
            }
        }
    }
}

fn badge_class(state: &EngineState) -> &'static str {
    match state {
        EngineState::Running => "eng-badge tone-success",
        EngineState::Stopped | EngineState::Unsupported { .. } => "eng-badge tone-stopped",
        EngineState::Starting | EngineState::Stopping => "eng-badge tone-warning",
        EngineState::Unreachable { .. } => "eng-badge tone-danger",
    }
}

/// Add the engine's load now: the sum over its running containers' newest
/// samples, one point per second.
fn push_load(state: &AppState, store: DashboardStore, stats: &StatsService, id: &EngineId) {
    let now = Utc::now();
    let (cores, keys): (u32, Vec<StatsKey>) = {
        let engines = state.engines.peek();
        let cores = engines
            .iter()
            .find(|e| e.id == *id)
            .and_then(|e| e.resources.as_ref())
            .map_or(1, |vm| u32::from(vm.cpus));
        let keys = state
            .containers
            .peek()
            .iter()
            .filter(|c| c.engine == *id && c.state == berth_core::ContainerState::Running)
            .map(|c| StatsKey::new(&c.engine, &c.id))
            .collect();
        (cores, keys)
    };
    let samples: Vec<berth_core::LiveStats> = keys
        .iter()
        .filter_map(|key| stats.latest(key, now))
        .collect();
    if let Some((cpu, mem)) = crate::services::engine_dashboard::sum_load(&samples, cores) {
        store.push_load(id, now, cpu, mem);
    }
}

/// The buttons at the right of the header, by state.
fn header_buttons(
    state: &mut AppState,
    actions: Option<&ActionsService>,
    link: Option<&EngineLink>,
    engine: &Engine,
    colima: bool,
) -> Element {
    let reason = |action: &'static str| {
        with_view(state, |view| {
            crate::services::actions::disabled_reason(
                &berth_core::Subject::Engine(engine.id.clone()),
                action,
                view,
            )
        })
    };
    let mut open = *state;
    let mut stop_state = *state;
    let mut restart_state = *state;
    let mut start_state = *state;
    let mut retry_state = *state;
    let stop_target = engine.clone();
    let restart_target = engine.clone();
    let start_target = engine.clone();
    let retry_target = engine.clone();
    let edit_id = engine.id.clone();
    let start_actions = actions.cloned();
    let retry_link = link.cloned();
    let retrying = state.pending.read().iter().any(|(key, value)| {
        key.subject == berth_core::Subject::Engine(engine.id.clone())
            && key.action == "engine.retry"
            && matches!(value, PendingState::Running { .. })
    });
    match &engine.state {
        EngineState::Running if colima => rsx! {
            div { class: "dash-actions",
                Button {
                    label: "Restart engine\u{2026}".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    disabled_reason: reason("engine.restart"),
                    disabled: reason("engine.restart").is_some(),
                    onclick: move |_| ask(&mut restart_state, &restart_target, berth_core::ColimaOp::Restart),
                }
                Button {
                    label: "Edit resources".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    disabled_reason: reason("engine.edit_resources"),
                    disabled: reason("engine.edit_resources").is_some(),
                    onclick: move |_| edit_resources::open_sheet(&mut open, &edit_id),
                }
                Button {
                    label: "Stop engine\u{2026}".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    disabled_reason: reason("engine.stop"),
                    disabled: reason("engine.stop").is_some(),
                    onclick: move |_| ask(&mut stop_state, &stop_target, berth_core::ColimaOp::Stop),
                }
            }
        },
        EngineState::Stopped if colima => rsx! {
            div { class: "dash-actions",
                Button {
                    label: "Edit resources".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    disabled_reason: reason("engine.edit_resources"),
                    disabled: reason("engine.edit_resources").is_some(),
                    onclick: move |_| edit_resources::open_sheet(&mut open, &edit_id),
                }
                Button {
                    label: "Start engine".to_string(),
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    disabled_reason: reason("engine.start"),
                    disabled: reason("engine.start").is_some(),
                    onclick: move |_| start_engine(&mut start_state, start_actions.as_ref(), &start_target),
                }
            }
        },
        EngineState::Unreachable { .. } => rsx! {
            div { class: "dash-actions",
                Button {
                    label: "Retry".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    disabled: retrying,
                    loading: retrying,
                    onclick: move |_| retry(&mut retry_state, retry_link.as_ref(), &retry_target),
                }
            }
        },
        _ => rsx! {},
    }
}

/// What sits under the header for an engine that is not mid-command.
fn body(
    state: &mut AppState,
    store: Option<DashboardStore>,
    engine: &Engine,
    colima_version: Option<&str>,
) -> Element {
    let _ = colima_version;
    match &engine.state {
        EngineState::Running => running_body(state, store, engine),
        EngineState::Stopped => {
            let colima = matches!(engine.kind, EngineKind::Colima { .. });
            rsx! {
                div { class: "dash-split",
                    div { class: "dash-main",
                        p { class: "dash-note",
                            if colima {
                                "The engine is stopped. Start it to see its load, disk and containers."
                            } else {
                                "The engine is stopped."
                            }
                        }
                        {disk_section(state, store, engine)}
                    }
                    {info_list(engine)}
                }
            }
        }
        EngineState::Unreachable { reason, last_seen } => {
            let seen = last_seen.map_or_else(
                || "Berth has not reached it yet.".to_string(),
                |at| format!("Last seen {}.", berth_core::fmt::age(at, Utc::now())),
            );
            rsx! {
                div { class: "dash-banner",
                    Banner {
                        message: format!("{reason}. {seen}"),
                        tone: BannerTone::Danger,
                    }
                }
                div { class: "dash-split",
                    div { class: "dash-main" }
                    {info_list(engine)}
                }
            }
        }
        EngineState::Unsupported { reason } => rsx! {
            div { class: "dash-banner",
                Banner { message: reason.clone(), tone: BannerTone::Info }
            }
            div { class: "dash-split",
                div { class: "dash-main" }
                {info_list(engine)}
            }
        },
        EngineState::Starting | EngineState::Stopping => rsx! {},
    }
}

fn info_list(engine: &Engine) -> Element {
    let rows = info_rows(engine);
    rsx! {
        section { class: "dash-info", "aria-label": "Engine info",
            h3 { class: "dash-section-title", "Engine info" }
            dl { class: "dash-info-list",
                for (label, value) in rows {
                    div { key: "{label}", class: "dash-info-row",
                        dt { "{label}" }
                        dd { "{value}" }
                    }
                }
            }
        }
    }
}

/// Why "Clean up\u{2026}" is off, if it is: the engine is not running, or
/// no group of the plan has anything in it.
#[must_use]
pub fn clean_up_blocker(
    state: &AppState,
    store: Option<DashboardStore>,
    engine: &Engine,
) -> Option<String> {
    if engine.state != EngineState::Running {
        return Some("Clean up needs a running engine".to_string());
    }
    crate::sheets::clean_up::initial_entries(state, store, &engine.id)
        .is_empty()
        .then(|| NOTHING_TO_CLEAN_UP.to_string())
}

/// The reason on a disabled "Clean up\u{2026}" when the plan is empty.
pub const NOTHING_TO_CLEAN_UP: &str = "Nothing to clean up";

fn disk_section(state: &mut AppState, store: Option<DashboardStore>, engine: &Engine) -> Element {
    let blocker = clean_up_blocker(state, store, engine);
    let mut open = *state;
    let id = engine.id.clone();
    let running = engine.state == EngineState::Running;
    let entry = store.and_then(|s| s.usage.read().get(&engine.id).cloned());
    let vm_disk = engine.resources.as_ref().map(|vm| vm.disk);
    let (head, tail, bar) = match (&entry, running) {
        (_, false) => (
            None,
            None,
            rsx! {
                UsageBar {
                    segments: Vec::new(),
                    total: 0_u64,
                    unavailable: "Disk usage shows while the engine runs.".to_string(),
                }
            },
        ),
        (Some(UsageEntry::Ready(usage)), true) => {
            let (head, tail) = disk_summary(usage, vm_disk);
            let total = usage.disk_total.or(vm_disk).map_or(0, |t| t.as_u64());
            (
                Some(head),
                tail,
                rsx! { UsageBar { segments: segments_of(usage), total } },
            )
        }
        (Some(UsageEntry::Failed(message)), true) => (
            None,
            None,
            rsx! {
                UsageBar {
                    segments: Vec::new(),
                    total: 0_u64,
                    unavailable: format!("Disk usage could not be read: {message}"),
                }
            },
        ),
        (None, true) => (
            None,
            None,
            rsx! { UsageBar { segments: Vec::new(), total: 0_u64, loading: true } },
        ),
    };
    rsx! {
        section { class: "dash-disk", "aria-label": "Disk",
            div { class: "dash-disk-head",
                h3 { class: "dash-section-title", "Disk" }
                if let Some(head) = head {
                    span { class: "dash-disk-summary", "{head}" }
                }
                if let Some(tail) = tail {
                    span { class: "dash-disk-summary", "\u{b7} {tail}" }
                }
                span { class: "dash-spacer" }
                Button {
                    label: "Clean up\u{2026}".to_string(),
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    disabled: blocker.is_some(),
                    disabled_reason: blocker,
                    onclick: move |_| open.overlays.write().push(Overlay::CleanUp(id.clone())),
                }
            }
            {bar}
        }
    }
}

fn running_body(state: &mut AppState, store: Option<DashboardStore>, engine: &Engine) -> Element {
    let colima = matches!(engine.kind, EngineKind::Colima { .. });
    let series = store.and_then(|s| s.series.read().get(&engine.id).cloned());
    let tiles = load_tiles(series.as_ref(), engine, Utc::now(), None);
    rsx! {
        if colima {
            p { class: "dash-notice", "{NOTICE}" }
        }
        div { class: "dash-split",
            div { class: "dash-main",
                div { class: "dash-tiles",
                    for tile in tiles {
                        MetricTile { key: "{tile.label}", data: tile }
                    }
                }
            }
            {info_list(engine)}
        }
        {disk_section(state, store, engine)}
        div { class: "dash-recent",
            crate::screens::recent_activity::RecentActivity {
                max: RECENT_LINES,
                engine: Some(engine.id.clone()),
            }
        }
    }
}

/// The frame's line under the title.
pub const NOTICE: &str =
    "Edit resources restarts Colima \u{b7} running containers stop for about 20 s.";

/// How many recent lines the frame shows.
pub const RECENT_LINES: usize = 13;

/// The run panel (frame "Engines / starting / 1440"): progress, the
/// command, and its output with the second each line came at.
#[component]
fn RunPanel(engine: Engine, run: RunView) -> Element {
    let profile = match &engine.kind {
        EngineKind::Colima { profile } => profile.clone(),
        _ => engine.name.clone(),
    };
    let command = colima::run_command(run.action, &profile);
    rsx! {
        div { class: "run-panel",
            RunMeter { run: run.clone() }
            h3 { class: "run-command", "{command}" }
            div { class: "run-log", role: "log", "aria-label": "{command}",
                for (index, line) in run.lines.iter().enumerate() {
                    div { key: "{index}", class: "run-line",
                        span { class: "run-line-at", "+{line.at_s:.1} s" }
                        span { class: "run-line-text", "{line.text}" }
                    }
                }
            }
            if run.action != "engine.stop" {
                p { class: "run-note", "Containers appear here once Docker answers." }
            }
        }
    }
}

/// A Colima command that failed: what it said, and a way to dismiss it.
#[component]
fn FailedRun(engine: Engine, run: RunView) -> Element {
    let actions = try_use_context::<ActionsService>();
    let id = engine.id.clone();
    let title = format!("{} failed", colima::run_title(run.action));
    let reason = run.failure.clone().unwrap_or_default();
    let pending_key = PendingKey {
        subject: berth_core::Subject::Engine(engine.id.clone()),
        action: run.action,
    };
    let mut state = use_context::<AppState>();
    rsx! {
        div { class: "dash-banner",
            Banner {
                message: format!("{title}: {reason}"),
                tone: BannerTone::Danger,
                action_label: Some("Dismiss".to_string()),
                onaction: move |()| {
                    if let Some(actions) = &actions {
                        actions.clear_engine_run(&id);
                    }
                    crate::services::actions::dismiss(&mut state, &pending_key);
                },
            }
        }
        if !run.lines.is_empty() {
            div { class: "run-log run-log-failed", role: "log", "aria-label": "Output",
                for (index, line) in run.lines.iter().enumerate() {
                    div { key: "{index}", class: "run-line",
                        span { class: "run-line-at", "+{line.at_s:.1} s" }
                        span { class: "run-line-text", "{line.text}" }
                    }
                }
            }
        }
    }
}
