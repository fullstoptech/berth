//! `MetricTile` (components-spec): one live figure with a five-minute
//! sparkline, and `StatsPanel`, the four tiles of a container's Stats tab
//! (CPU, memory with its limit, network in and out). The tile is pure
//! markup over [`MetricTileData`]; [`panel_model`] builds the data from a
//! [`History`], so everything but the thin `StatsPanel` host is tested
//! without a window.

use berth_core::{ContainerId, ContainerState, LiveStats};
use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;

use crate::services::stats::{History, StatsKey, StatsService, HISTORY_WINDOW_S};
use crate::state::AppState;

/// The tiles' CSS. Rendered with the panel; colours come from `tokens.css`.
pub const STATS_CSS: &str = include_str!("../../assets/css/stats.css");

/// Sparkline drawing box (the SVG scales to its tile).
const SPARK_W: f64 = 160.0;
const SPARK_H: f64 = 36.0;
/// A hole in the samples wider than this many seconds breaks the line, so
/// time the container was off screen is not drawn as a straight ramp.
const GAP_S: f64 = 5.0;

/// Everything a tile shows.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricTileData {
    pub label: &'static str,
    /// The figure, e.g. "23.7"; `None` draws "—".
    pub value: Option<String>,
    /// Its unit, e.g. "%", "MB", "KB/s".
    pub unit: String,
    /// Under the sparkline, e.g. "of 512 MB · last 5 min", or the reason
    /// the tile has no figure.
    pub caption: String,
    /// Points as (x, y): x in 0..=1 over the five-minute window, y raw.
    pub spark: Vec<(f64, f64)>,
    /// The sparkline's full scale.
    pub y_max: f64,
    /// A fill under the figure (0..=1), for values with a limit.
    pub fraction: Option<f32>,
    /// 1..=5, picks `--color-series-N`.
    pub series: u8,
    /// Waiting for the first sample rather than unavailable.
    pub loading: bool,
}

impl MetricTileData {
    fn empty(label: &'static str, series: u8, caption: String, loading: bool) -> Self {
        Self {
            label,
            value: None,
            unit: String::new(),
            caption,
            spark: Vec::new(),
            y_max: 1.0,
            fraction: None,
            series,
            loading,
        }
    }
}

/// Map samples onto the window: x = 0 is five minutes ago, 1 is `now`.
#[allow(clippy::cast_precision_loss)]
pub fn spark_points(series: &[(DateTime<Utc>, f64)], now: DateTime<Utc>) -> Vec<(f64, f64)> {
    let window_ms = (HISTORY_WINDOW_S * 1000) as f64;
    series
        .iter()
        .filter_map(|(at, y)| {
            let age_ms = now.signed_duration_since(*at).num_milliseconds() as f64;
            let x = 1.0 - age_ms / window_ms;
            (0.0..=1.0).contains(&x).then_some((x, *y))
        })
        .collect()
}

/// The SVG path of a sparkline: `M x y L x y …`, a new `M` after a gap.
/// `y_max` is the full scale; values above it are clipped to the top.
pub fn sparkline_path(points: &[(f64, f64)], width: f64, height: f64, y_max: f64) -> String {
    use std::fmt::Write as _;
    let scale = if y_max > 0.0 { y_max } else { 1.0 };
    let gap_x = GAP_S / f64::from(i32::try_from(HISTORY_WINDOW_S).unwrap_or(300));
    let mut path = String::new();
    let mut prev_x: Option<f64> = None;
    for &(x, y) in points {
        let px = x.clamp(0.0, 1.0) * width;
        let py = height - (y / scale).clamp(0.0, 1.0) * height;
        let command = if prev_x.is_some_and(|p| x - p <= gap_x) {
            "L"
        } else {
            "M"
        };
        if !path.is_empty() {
            path.push(' ');
        }
        let _ = write!(path, "{command}{px:.1} {py:.1}");
        prev_x = Some(x);
    }
    path
}

/// A full scale that leaves headroom over the peak and never collapses.
pub fn scale_for(peak: f64, floor: f64) -> f64 {
    (peak * 1.25).max(floor)
}

/// "184 MB" into ("184", "MB").
fn split_unit(text: &str) -> (String, String) {
    match text.split_once(' ') {
        Some((value, unit)) => (value.to_string(), unit.to_string()),
        None => (text.to_string(), String::new()),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn rate_text(bytes_per_s: f64) -> (String, String) {
    let (value, unit) = split_unit(&berth_core::fmt::bytes(ByteSize::b(
        bytes_per_s.max(0.0) as u64
    )));
    (value, format!("{unit}/s"))
}

fn peak_of(series: &[(DateTime<Utc>, f64)]) -> f64 {
    series.iter().map(|(_, y)| *y).fold(0.0, f64::max)
}

/// Why a container shows no stats.
pub fn unavailable_reason(state: ContainerState) -> Option<&'static str> {
    match state {
        ContainerState::Running | ContainerState::Paused => None,
        ContainerState::Restarting => Some("Stats resume when the restart settles"),
        ContainerState::Created | ContainerState::Exited | ContainerState::Dead => {
            Some("Stats show while the container runs")
        }
        ContainerState::Removing => Some("The container is being removed"),
    }
}

/// The four tiles for a container. `reason` is why stats are unavailable
/// (stopped container, no service); otherwise an empty history reads as
/// "waiting for the first sample" and a stale one as unavailable.
pub fn panel_model(
    history: &History,
    now: DateTime<Utc>,
    reason: Option<&str>,
) -> Vec<MetricTileData> {
    let labels: [(&'static str, u8); 4] = [
        ("CPU", 1),
        ("Memory", 2),
        ("Network in", 3),
        ("Network out", 4),
    ];
    if let Some(reason) = reason {
        return labels
            .iter()
            .map(|&(label, series)| MetricTileData::empty(label, series, reason.to_string(), false))
            .collect();
    }
    let Some(fresh) = history.fresh(now) else {
        let waiting = history.is_empty();
        let caption = if waiting {
            "Waiting for the first sample"
        } else {
            "No recent samples"
        };
        return labels
            .iter()
            .map(|&(label, series)| {
                MetricTileData::empty(label, series, caption.to_string(), waiting)
            })
            .collect();
    };
    vec![
        cpu_tile(history, now, fresh),
        memory_tile(history, now, fresh),
        net_tile(history, now, fresh, Direction::In),
        net_tile(history, now, fresh, Direction::Out),
    ]
}

fn cpu_tile(history: &History, now: DateTime<Utc>, fresh: &LiveStats) -> MetricTileData {
    let series = history.series(|s| f64::from(s.cpu_pct));
    let peak = peak_of(&series);
    MetricTileData {
        label: "CPU",
        value: Some(format!("{:.1}", fresh.cpu_pct)),
        unit: "%".to_string(),
        caption: format!("peak {peak:.1}% · last 5 min"),
        spark: spark_points(&series, now),
        y_max: scale_for(peak, 10.0),
        fraction: None,
        series: 1,
        loading: false,
    }
}

#[allow(clippy::cast_precision_loss)]
fn memory_tile(history: &History, now: DateTime<Utc>, fresh: &LiveStats) -> MetricTileData {
    let series = history.series(|s| s.mem_used.as_u64() as f64);
    let peak = peak_of(&series);
    let (value, unit) = split_unit(&berth_core::fmt::bytes(fresh.mem_used));
    let limit = fresh.mem_limit.as_u64();
    let (caption, fraction) = if limit == 0 {
        ("no limit · last 5 min".to_string(), None)
    } else {
        (
            format!(
                "of {} · last 5 min",
                berth_core::fmt::bytes(fresh.mem_limit)
            ),
            Some((fresh.mem_used.as_u64() as f32 / limit as f32).clamp(0.0, 1.0)),
        )
    };
    MetricTileData {
        label: "Memory",
        value: Some(value),
        unit,
        caption,
        spark: spark_points(&series, now),
        y_max: scale_for(peak, 1.0),
        fraction,
        series: 2,
        loading: false,
    }
}

#[derive(Clone, Copy)]
enum Direction {
    In,
    Out,
}

fn net_tile(
    history: &History,
    now: DateTime<Utc>,
    fresh: &LiveStats,
    direction: Direction,
) -> MetricTileData {
    let (label, series_no, total) = match direction {
        Direction::In => ("Network in", 3, fresh.net_rx),
        Direction::Out => ("Network out", 4, fresh.net_tx),
    };
    let rates = history.rates(match direction {
        Direction::In => |s: &LiveStats| s.net_rx.as_u64(),
        Direction::Out => |s: &LiveStats| s.net_tx.as_u64(),
    });
    let peak = peak_of(&rates);
    let current = rates.last().map_or(0.0, |(_, r)| *r);
    let (value, unit) = rate_text(current);
    MetricTileData {
        label,
        // A rate needs two samples; one sample shows only the total.
        value: if rates.is_empty() { None } else { Some(value) },
        unit: if rates.is_empty() {
            String::new()
        } else {
            unit
        },
        caption: format!("{} total · last 5 min", berth_core::fmt::bytes(total)),
        spark: spark_points(&rates, now),
        y_max: scale_for(peak, 1024.0),
        fraction: None,
        series: series_no,
        loading: false,
    }
}

#[component]
pub fn MetricTile(data: MetricTileData) -> Element {
    let path = sparkline_path(&data.spark, SPARK_W, SPARK_H, data.y_max);
    let class = if data.loading {
        "mt-tile mt-loading"
    } else if data.value.is_none() {
        "mt-tile mt-unavailable"
    } else {
        "mt-tile"
    };
    let figure = data.value.clone().unwrap_or_else(|| "—".to_string());
    let fill = data.fraction.map(|f| format!("width: {:.1}%;", f * 100.0));
    rsx! {
        div { class, role: "group", "aria-label": data.label,
            div { class: "mt-label", "{data.label}" }
            div { class: "mt-figure",
                span { class: "mt-value", "{figure}" }
                if data.value.is_some() && !data.unit.is_empty() {
                    span { class: "mt-unit", "{data.unit}" }
                }
            }
            if let Some(style) = fill {
                span { class: "mt-bar", "aria-hidden": "true",
                    span { class: "mt-bar-fill", style }
                }
            }
            svg {
                class: "mt-spark mt-s{data.series}",
                view_box: "0 0 {SPARK_W} {SPARK_H}",
                preserve_aspect_ratio: "none",
                "aria-hidden": "true",
                if !path.is_empty() {
                    path { d: "{path}" }
                }
            }
            div { class: "mt-caption", "{data.caption}" }
        }
    }
}

/// The Stats tab body: four tiles, fed live. Mount it with a container id;
/// it holds the container's stats stream open while it is on screen and
/// releases it on drop. Needs `AppState` and, for live data, a
/// `StatsService` in context (without one every tile reads unavailable).
#[component]
pub fn StatsPanel(id: ContainerId) -> Element {
    let state = use_context::<AppState>();
    let service = try_consume_context::<StatsService>();
    let owner = use_hook(|| service.as_ref().map(StatsService::new_owner));

    let found = state
        .containers
        .read()
        .iter()
        .find(|c| c.id == id)
        .map(|c| (c.engine.clone(), c.state));
    let reason = match (&found, &service) {
        (None, _) => Some("This container was removed"),
        (_, None) => Some("Live stats are not connected"),
        (Some((_, container_state)), Some(_)) => unavailable_reason(*container_state),
    };
    let key = found
        .as_ref()
        .filter(|_| reason.is_none())
        .map(|(engine, _)| StatsKey::new(engine, &id));
    if let (Some(service), Some(owner)) = (&service, owner) {
        service.watch_panel(owner, key.clone());
    }
    let release = service.clone();
    use_drop(move || {
        if let (Some(service), Some(owner)) = (release, owner) {
            service.release(owner);
        }
    });

    let now = Utc::now();
    let history = match (&service, &key) {
        (Some(service), Some(key)) => service.history(key),
        _ => History::default(),
    };
    let tiles = panel_model(&history, now, reason);
    rsx! {
        style { "{STATS_CSS}" }
        section { class: "stats-panel", "aria-label": "Stats",
            for tile in tiles {
                MetricTile { key: "{tile.label}", data: tile }
            }
        }
    }
}

/// The "Stats / panel" dev scene: the panel at the inspector's width,
/// under the container's name.
#[component]
pub fn StatsPanelScene(id: ContainerId) -> Element {
    let state = use_context::<AppState>();
    let name = state
        .containers
        .read()
        .iter()
        .find(|c| c.id == id)
        .map_or_else(|| id.0.clone(), |c| c.name.clone());
    rsx! {
        style { "{STATS_CSS}" }
        div { class: "dev-stats-scene",
            h2 { "Stats / panel" }
            p { class: "dev-stats-name", "{name}" }
            aside { class: "dev-stats-card", StatsPanel { id } }
        }
    }
}

#[cfg(test)]
#[path = "metric_tile_tests.rs"]
mod tests;
