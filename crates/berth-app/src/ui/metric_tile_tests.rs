use berth_core::{ContainerId, LiveStats};
use bytesize::ByteSize;
use chrono::{DateTime, TimeZone, Utc};

use super::*;
use crate::services::stats::History;
use crate::state::use_app_state;

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_792_000_000, 0)
        .single()
        .unwrap_or_default()
}

fn at(seconds_ago: i64) -> DateTime<Utc> {
    now() - chrono::Duration::seconds(seconds_ago)
}

fn live(cpu: f32, mem_mb: u64, rx: u64, tx: u64) -> LiveStats {
    LiveStats {
        cpu_pct: cpu,
        mem_used: ByteSize::mb(mem_mb),
        mem_limit: ByteSize::mb(512),
        net_rx: ByteSize::b(rx),
        net_tx: ByteSize::b(tx),
        at: now(),
    }
}

fn history(samples: &[(i64, LiveStats)]) -> History {
    let mut history = History::default();
    for (ago, stats) in samples {
        history.push(at(*ago), stats.clone());
    }
    history
}

#[test]
fn stats_sparkline_path_flips_y_and_spans_the_width() {
    // 3 s apart on a 100-unit-wide box.
    let path = sparkline_path(&[(0.0, 0.0), (0.01, 5.0), (0.02, 10.0)], 100.0, 40.0, 10.0);
    assert_eq!(path, "M0.0 40.0 L1.0 20.0 L2.0 0.0");
}

#[test]
fn stats_sparkline_path_clips_values_above_the_scale() {
    let path = sparkline_path(&[(0.0, 50.0), (0.01, -3.0)], 100.0, 40.0, 10.0);
    assert_eq!(path, "M0.0 0.0 L1.0 40.0");
}

#[test]
fn stats_sparkline_path_breaks_the_line_at_a_gap() {
    // 1 s apart, then a minute of nothing, then 1 s apart again.
    let x = |seconds: f64| seconds / 300.0;
    let points = [(x(0.0), 1.0), (x(1.0), 1.0), (x(61.0), 1.0), (x(62.0), 1.0)];
    let path = sparkline_path(&points, 300.0, 10.0, 2.0);
    assert_eq!(path.matches('M').count(), 2, "{path}");
    assert_eq!(path.matches('L').count(), 2, "{path}");
}

#[test]
fn stats_sparkline_path_of_nothing_is_empty() {
    assert_eq!(sparkline_path(&[], 100.0, 40.0, 10.0), "");
}

#[test]
fn stats_spark_points_place_samples_inside_the_five_minute_window() {
    let series = vec![(at(400), 1.0), (at(300), 2.0), (at(150), 3.0), (at(0), 4.0)];
    let points = spark_points(&series, now());
    // The sample from 400 s ago is outside the window and dropped.
    assert_eq!(points.len(), 3);
    assert!((points[0].0 - 0.0).abs() < 1e-9);
    assert!((points[1].0 - 0.5).abs() < 1e-9);
    assert!((points[2].0 - 1.0).abs() < 1e-9);
}

#[test]
fn stats_panel_model_reads_cpu_memory_and_the_limit() {
    let h = history(&[
        (2, live(2.0, 180, 1_000, 500)),
        (1, live(5.5, 182, 3_000, 700)),
        (0, live(3.2, 184, 5_000, 900)),
    ]);
    let tiles = panel_model(&h, now(), None);
    let labels: Vec<&str> = tiles.iter().map(|t| t.label).collect();
    assert_eq!(labels, ["CPU", "Memory", "Network in", "Network out"]);

    let cpu = &tiles[0];
    assert_eq!(cpu.value.as_deref(), Some("3.2"));
    assert_eq!(cpu.unit, "%");
    assert_eq!(cpu.caption, "peak 5.5% · last 5 min");
    assert_eq!(cpu.spark.len(), 3);

    let memory = &tiles[1];
    assert_eq!(memory.value.as_deref(), Some("184"));
    assert_eq!(memory.unit, "MB");
    assert_eq!(memory.caption, "of 512 MB · last 5 min");
    let fraction = memory.fraction.unwrap_or(-1.0);
    assert!((fraction - 184.0 / 512.0).abs() < 0.001, "{fraction}");

    // 2 000 B in the last second.
    let rx = &tiles[2];
    assert_eq!(rx.value.as_deref(), Some("2"));
    assert_eq!(rx.unit, "KB/s");
    assert_eq!(rx.caption, "5 KB total · last 5 min");
    assert_eq!(rx.spark.len(), 2);
}

#[test]
fn stats_panel_model_with_one_sample_shows_totals_but_no_rate() {
    let h = history(&[(0, live(1.0, 100, 4_000, 2_000))]);
    let tiles = panel_model(&h, now(), None);
    assert_eq!(tiles[0].value.as_deref(), Some("1.0"));
    assert!(tiles[2].value.is_none(), "a rate needs two samples");
    assert_eq!(tiles[2].caption, "4 KB total · last 5 min");
}

#[test]
fn stats_panel_model_without_samples_waits_and_never_shows_zero() {
    let tiles = panel_model(&History::default(), now(), None);
    assert_eq!(tiles.len(), 4);
    for tile in &tiles {
        assert!(tile.value.is_none(), "{} shows a figure", tile.label);
        assert!(tile.loading);
        assert_eq!(tile.caption, "Waiting for the first sample");
    }
}

#[test]
fn stats_panel_model_with_only_old_samples_is_unavailable() {
    let h = history(&[(60, live(9.0, 100, 1, 1))]);
    let tiles = panel_model(&h, now(), None);
    for tile in &tiles {
        assert!(tile.value.is_none());
        assert!(!tile.loading);
        assert_eq!(tile.caption, "No recent samples");
    }
}

#[test]
fn stats_panel_model_gives_the_reason_a_container_has_no_stats() {
    let reason = unavailable_reason(berth_core::ContainerState::Exited);
    assert_eq!(reason, Some("Stats show while the container runs"));
    assert!(unavailable_reason(berth_core::ContainerState::Running).is_none());
    assert!(unavailable_reason(berth_core::ContainerState::Paused).is_none());
    let h = history(&[(0, live(9.0, 100, 1, 1))]);
    let tiles = panel_model(&h, now(), reason);
    assert!(tiles.iter().all(|t| t.value.is_none()));
    assert!(tiles
        .iter()
        .all(|t| t.caption == "Stats show while the container runs"));
}

#[component]
fn TileHost(data: MetricTileData) -> Element {
    rsx! { MetricTile { data } }
}

fn render_tile(data: MetricTileData) -> String {
    let mut dom = VirtualDom::new_with_props(TileHost, TileHostProps { data });
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

#[test]
fn stats_tile_draws_figure_unit_bar_and_sparkline() {
    let h = history(&[(1, live(2.0, 180, 1, 1)), (0, live(3.0, 184, 2, 2))]);
    let tiles = panel_model(&h, now(), None);
    let html = render_tile(tiles[1].clone());
    assert!(html.contains(">184<"), "{html}");
    assert!(html.contains(">MB<"), "{html}");
    assert!(html.contains("of 512 MB"), "{html}");
    assert!(html.contains("mt-bar-fill"), "{html}");
    assert!(html.contains("<path d=\"M"), "{html}");
    assert!(html.contains("mt-s2"), "{html}");
}

#[test]
fn stats_tile_without_a_figure_draws_a_dash_and_its_reason() {
    let tiles = panel_model(
        &History::default(),
        now(),
        Some("Stats show while the container runs"),
    );
    let html = render_tile(tiles[0].clone());
    assert!(html.contains(">—<"), "{html}");
    assert!(html.contains("mt-unavailable"), "{html}");
    assert!(
        html.contains("Stats show while the container runs"),
        "{html}"
    );
    assert!(!html.contains("<path"), "{html}");
    assert!(!html.contains(">0"), "never a zero: {html}");
}

#[component]
fn PanelHost(id: ContainerId, with_service: bool) -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    let mut containers = state.containers;
    use_hook(move || {
        let world = crate::services::mock_world_at(Utc::now());
        containers.set(world.containers);
    });
    if with_service {
        use_context_provider(|| {
            StatsService::new(
                crate::services::engines::client_map(),
                tokio_util::sync::CancellationToken::new(),
            )
        });
    }
    rsx! { StatsPanel { id } }
}

fn render_panel(name: &str, with_service: bool) -> String {
    let world = crate::services::mock_world_at(Utc::now());
    let id = world
        .containers
        .iter()
        .find(|c| c.name == name)
        .map_or_else(|| ContainerId("missing".to_string()), |c| c.id.clone());
    let mut dom = VirtualDom::new_with_props(PanelHost, PanelHostProps { id, with_service });
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

#[test]
fn stats_panel_for_a_running_container_waits_for_its_first_sample() {
    let html = render_panel("shop-api-api-1", true);
    for label in ["CPU", "Memory", "Network in", "Network out"] {
        assert!(html.contains(label), "{label}: {html}");
    }
    assert!(html.contains("Waiting for the first sample"), "{html}");
}

#[test]
fn stats_panel_for_an_exited_container_says_why_there_is_nothing() {
    let html = render_panel("shop-api-db-1", true);
    assert!(
        html.contains("Stats show while the container runs"),
        "{html}"
    );
    assert!(!html.contains("Waiting for the first sample"), "{html}");
}

#[test]
fn stats_panel_without_a_service_reads_unavailable() {
    let html = render_panel("shop-api-api-1", false);
    assert!(html.contains("Live stats are not connected"), "{html}");
}

#[test]
fn stats_panel_for_a_removed_container_says_so() {
    let mut dom = VirtualDom::new_with_props(
        PanelHost,
        PanelHostProps {
            id: ContainerId("gone".to_string()),
            with_service: true,
        },
    );
    let _ = dom.rebuild_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("This container was removed"), "{html}");
}
