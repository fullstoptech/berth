use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use berth_core::{ContainerId, EngineId, LiveStats};
use bytesize::ByteSize;
use chrono::{DateTime, TimeZone, Utc};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::ui::visible_range;

fn key(n: usize) -> StatsKey {
    StatsKey::new(
        &EngineId("colima:default".to_string()),
        &ContainerId(format!("c{n:04}")),
    )
}

fn sample(cpu: f32, rx: u64) -> LiveStats {
    LiveStats {
        cpu_pct: cpu,
        mem_used: ByteSize::mb(184),
        mem_limit: ByteSize::mb(512),
        net_rx: ByteSize::b(rx),
        net_tx: ByteSize::b(rx / 4),
        at: Utc
            .timestamp_opt(1_792_000_000, 0)
            .single()
            .unwrap_or_default(),
    }
}

fn t0() -> DateTime<Utc> {
    Utc.timestamp_opt(1_792_000_000, 0)
        .single()
        .unwrap_or_default()
}

/// Counts what the service opens and what is still alive.
#[derive(Default)]
struct Probe {
    opened: AtomicUsize,
    alive: AtomicUsize,
}

struct AliveGuard(Arc<Probe>);

impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A stream that yields one sample at once, then waits to be cancelled.
fn probe_opener(probe: &Arc<Probe>) -> Opener {
    let probe = Arc::clone(probe);
    Arc::new(move |_key, cancel| {
        probe.opened.fetch_add(1, Ordering::SeqCst);
        probe.alive.fetch_add(1, Ordering::SeqCst);
        let guard = AliveGuard(Arc::clone(&probe));
        Some(
            futures::stream::unfold((cancel, guard, false), |(cancel, guard, sent)| async move {
                if sent {
                    cancel.cancelled().await;
                    return None;
                }
                Some((Ok(sample(3.2, 1_000)), (cancel, guard, true)))
            })
            .boxed(),
        )
    })
}

fn service(probe: &Arc<Probe>) -> StatsService {
    StatsService::with_opener(probe_opener(probe), CancellationToken::new())
}

/// Wait (briefly, in real time) until `check` holds.
async fn until(mut check: impl FnMut() -> bool) {
    for _ in 0..400 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("condition not reached in 2 s");
}

/// A stand-in for the table: one owner per rendered row index, mounted and
/// dropped as the visible range moves, exactly as `VirtualTable` mounts and
/// drops row components.
struct FakeTable {
    total: usize,
    mounted: std::collections::HashMap<usize, OwnerId>,
}

impl FakeTable {
    fn new(total: usize) -> Self {
        Self {
            total,
            mounted: std::collections::HashMap::new(),
        }
    }

    fn scroll(&mut self, service: &StatsService, scroll_top: f64, viewport: f64) {
        let range = visible_range(scroll_top, viewport, 28, self.total, 10);
        let gone: Vec<usize> = self
            .mounted
            .keys()
            .copied()
            .filter(|i| !range.contains(i))
            .collect();
        for i in gone {
            if let Some(owner) = self.mounted.remove(&i) {
                service.release(owner);
            }
        }
        for i in range {
            let owner = *self.mounted.entry(i).or_insert_with(|| service.new_owner());
            service.watch_row(owner, Some(key(i)));
        }
    }
}

#[tokio::test]
async fn stats_scrolling_a_1000_row_table_keeps_at_most_60_subscriptions() {
    let probe = Arc::new(Probe::default());
    let service = service(&probe);
    let mut table = FakeTable::new(1000);
    let mut peak = 0;
    // The Containers table: 1100 px of rows, 10 rows of overscan each way.
    for step in 0..=330 {
        table.scroll(&service, f64::from(step) * 84.0, 1100.0);
        service.reconcile();
        let open = service.subscription_count();
        assert!(open <= MAX_SUBSCRIPTIONS, "{open} open at step {step}");
        peak = peak.max(open);
    }
    assert!(peak >= 50, "the table should use its window: peak {peak}");
    // Only rows in the final window are open; the rest were closed.
    until(|| probe.alive.load(Ordering::SeqCst) == service.subscription_count()).await;
    assert!(
        probe.opened.load(Ordering::SeqCst) > 300,
        "scrolling opened new rows"
    );
}

#[tokio::test]
async fn stats_a_viewport_taller_than_the_cap_is_trimmed_to_60() {
    let probe = Arc::new(Probe::default());
    let service = service(&probe);
    let mut table = FakeTable::new(1000);
    // 4000 px = 143 rows + overscan: far more than the cap allows.
    table.scroll(&service, 5600.0, 4000.0);
    let report = service.reconcile();
    assert_eq!(report.opened.len(), MAX_SUBSCRIPTIONS);
    assert_eq!(service.subscription_count(), MAX_SUBSCRIPTIONS);
}

#[tokio::test]
async fn stats_hiding_the_window_closes_every_stream_and_showing_it_reopens() {
    let probe = Arc::new(Probe::default());
    let service = service(&probe);
    let mut table = FakeTable::new(1000);
    table.scroll(&service, 0.0, 560.0);
    service.reconcile();
    let before = service.subscription_count();
    assert_eq!(before, 30, "20 visible rows plus 10 of overscan");

    service.set_window_visible(false);
    assert_eq!(service.subscription_count(), 0);
    until(|| probe.alive.load(Ordering::SeqCst) == 0).await;
    // Hidden: a row that scrolls into view opens nothing.
    table.scroll(&service, 2800.0, 560.0);
    service.reconcile();
    assert_eq!(service.subscription_count(), 0);

    service.set_window_visible(true);
    assert_eq!(service.subscription_count(), 40);
    until(|| probe.alive.load(Ordering::SeqCst) == 40).await;
}

#[tokio::test]
async fn stats_scrolling_back_reopens_the_rows_that_scrolled_out() {
    let probe = Arc::new(Probe::default());
    let service = service(&probe);
    let mut table = FakeTable::new(1000);
    table.scroll(&service, 0.0, 560.0);
    service.reconcile();
    assert!(service.subscriptions().contains(&key(0)));

    table.scroll(&service, 14_000.0, 560.0);
    let away = service.reconcile();
    assert!(!service.subscriptions().contains(&key(0)));
    assert!(away.closed.contains(&key(0)));
    assert!(service.subscriptions().contains(&key(500)));

    table.scroll(&service, 0.0, 560.0);
    let back = service.reconcile();
    assert!(back.opened.contains(&key(0)));
    assert!(service.subscriptions().contains(&key(0)));
    assert!(!service.subscriptions().contains(&key(500)));
}

#[tokio::test]
async fn stats_an_open_panel_outranks_rows_when_the_cap_is_reached() {
    let probe = Arc::new(Probe::default());
    let service = service(&probe);
    let panel = service.new_owner();
    service.watch_panel(panel, Some(key(9999)));
    for i in 0..100 {
        let owner = service.new_owner();
        service.watch_row(owner, Some(key(i)));
    }
    service.reconcile();
    assert_eq!(service.subscription_count(), MAX_SUBSCRIPTIONS);
    assert!(service.subscriptions().contains(&key(9999)));
    // Among the rows, the ones that became visible last win.
    assert!(service.subscriptions().contains(&key(99)));
    assert!(!service.subscriptions().contains(&key(0)));
}

#[tokio::test]
async fn stats_a_row_and_a_panel_for_one_container_share_one_stream() {
    let probe = Arc::new(Probe::default());
    let service = service(&probe);
    let (row, panel) = (service.new_owner(), service.new_owner());
    service.watch_row(row, Some(key(1)));
    service.watch_panel(panel, Some(key(1)));
    service.reconcile();
    assert_eq!(service.subscription_count(), 1);
    until(|| probe.opened.load(Ordering::SeqCst) >= 1).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(probe.opened.load(Ordering::SeqCst), 1);

    // Closing the panel keeps the row's stream; closing both ends it.
    service.release(panel);
    service.reconcile();
    assert_eq!(service.subscription_count(), 1);
    service.release(row);
    service.reconcile();
    assert_eq!(service.subscription_count(), 0);
    until(|| probe.alive.load(Ordering::SeqCst) == 0).await;
}

#[tokio::test]
async fn stats_a_row_that_stops_running_closes_its_stream() {
    let probe = Arc::new(Probe::default());
    let service = service(&probe);
    let row = service.new_owner();
    service.watch_row(row, Some(key(1)));
    service.reconcile();
    assert_eq!(service.subscription_count(), 1);
    service.watch_row(row, None);
    service.reconcile();
    assert_eq!(service.subscription_count(), 0);
}

#[tokio::test]
async fn stats_samples_from_the_stream_reach_the_history() {
    let probe = Arc::new(Probe::default());
    let service = service(&probe);
    let row = service.new_owner();
    service.watch_row(row, Some(key(1)));
    assert!(
        service.latest(&key(1), Utc::now()).is_none(),
        "no stats yet is None"
    );
    service.reconcile();
    until(|| service.latest(&key(1), Utc::now()).is_some()).await;
    let latest = service.latest(&key(1), Utc::now());
    assert!((latest.map_or(0.0, |s| s.cpu_pct) - 3.2).abs() < 0.01);
    assert_eq!(service.history(&key(1)).len(), 1);
}

#[tokio::test]
async fn stats_a_stream_that_ends_is_not_reopened_inside_the_cooldown() {
    let opened = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&opened);
    let opener: Opener = Arc::new(move |_key, _cancel| {
        counter.fetch_add(1, Ordering::SeqCst);
        Some(futures::stream::empty().boxed())
    });
    let service = StatsService::with_opener(opener, CancellationToken::new());
    let row = service.new_owner();
    service.watch_row(row, Some(key(1)));
    service.reconcile();
    until(|| service.subscription_count() == 0).await;
    for _ in 0..5 {
        service.reconcile();
    }
    assert_eq!(opened.load(Ordering::SeqCst), 1, "no hot reopen loop");
}

#[tokio::test]
async fn stats_an_engine_that_is_not_connected_opens_nothing() {
    let service = StatsService::new(
        crate::services::engines::client_map(),
        CancellationToken::new(),
    );
    let row = service.new_owner();
    service.watch_row(row, Some(key(1)));
    service.reconcile();
    until(|| service.subscription_count() == 0).await;
    assert!(service.latest(&key(1), Utc::now()).is_none());
}

#[tokio::test]
async fn stats_cancelling_the_root_token_ends_every_stream() {
    let probe = Arc::new(Probe::default());
    let cancel = CancellationToken::new();
    let service = StatsService::with_opener(probe_opener(&probe), cancel.clone());
    for i in 0..5 {
        let owner = service.new_owner();
        service.watch_row(owner, Some(key(i)));
    }
    service.reconcile();
    until(|| probe.alive.load(Ordering::SeqCst) == 5).await;
    cancel.cancel();
    until(|| probe.alive.load(Ordering::SeqCst) == 0).await;
}

#[tokio::test]
async fn stats_the_mock_engine_feeds_the_service_through_the_client_map() {
    let engine = EngineId("colima:default".to_string());
    let world = crate::services::mock_world_at(Utc::now());
    let api = world
        .containers
        .iter()
        .find(|c| c.name == "shop-api-api-1")
        .map(|c| c.id.clone())
        .unwrap_or_else(|| ContainerId("missing".to_string()));
    let clients = crate::services::engines::client_map();
    if let Ok(mock) = berth_engine::MockEngine::from_world(world, &engine) {
        if let Ok(mut map) = clients.lock() {
            map.insert(engine.clone(), Arc::new(mock));
        }
    }
    let service = StatsService::new(clients, CancellationToken::new());
    let row = service.new_owner();
    let wanted = StatsKey::new(&engine, &api);
    service.watch_row(row, Some(wanted.clone()));
    service.reconcile();
    until(|| service.history(&wanted).len() >= 3).await;
    let shown = service.latest(&wanted, Utc::now());
    assert!(shown.is_some_and(|s| s.mem_used.as_u64() == 184_000_000));
}

#[test]
fn stats_history_is_capped_at_five_minutes() {
    let mut history = History::default();
    // One sample a second for 15 minutes.
    for s in 0..900 {
        history.push(t0() + chrono::Duration::seconds(s), sample(1.0, 0));
    }
    let first = history.points().next().map(|p| p.at);
    let last = history.latest().map(|p| p.at);
    let (Some(first), Some(last)) = (first, last) else {
        panic!("history is empty");
    };
    assert_eq!(last - first, chrono::Duration::seconds(HISTORY_WINDOW_S));
    assert_eq!(history.len(), 301);
}

#[test]
fn stats_history_never_holds_more_than_the_point_cap() {
    let mut history = History::default();
    // Ten samples a second for 100 s: inside the window, over the cap.
    for tenth in 0..1000 {
        history.push(
            t0() + chrono::Duration::milliseconds(tenth * 100),
            sample(1.0, 0),
        );
    }
    assert_eq!(history.len(), HISTORY_MAX_POINTS);
}

#[test]
fn stats_a_missing_or_stale_sample_reads_none_never_zero() {
    let mut history = History::default();
    assert!(history.fresh(t0()).is_none());
    history.push(t0(), sample(7.5, 0));
    assert!(history.fresh(t0() + chrono::Duration::seconds(2)).is_some());
    assert!(history
        .fresh(t0() + chrono::Duration::seconds(FRESH_FOR_S + 1))
        .is_none());
}

#[test]
fn stats_rates_are_bytes_per_second_and_skip_counter_resets() {
    let mut history = History::default();
    history.push(t0(), sample(0.0, 1_000));
    history.push(t0() + chrono::Duration::seconds(2), sample(0.0, 5_000));
    // The container restarted: its counter fell back to zero.
    history.push(t0() + chrono::Duration::seconds(3), sample(0.0, 200));
    history.push(t0() + chrono::Duration::seconds(4), sample(0.0, 1_200));
    let rates: Vec<f64> = history
        .rates(|s| s.net_rx.as_u64())
        .into_iter()
        .map(|(_, r)| r)
        .collect();
    assert_eq!(rates, vec![2_000.0, 1_000.0]);
}

#[tokio::test]
async fn stats_pruning_drops_histories_nothing_feeds_any_more() {
    let probe = Arc::new(Probe::default());
    let service = service(&probe);
    service.seed(&key(1), [(t0(), sample(1.0, 0))]);
    service.seed(
        &key(2),
        [(t0() + chrono::Duration::seconds(400), sample(1.0, 0))],
    );
    service.prune(t0() + chrono::Duration::seconds(400));
    assert!(service.history(&key(1)).is_empty());
    assert_eq!(service.history(&key(2)).len(), 1);
}
