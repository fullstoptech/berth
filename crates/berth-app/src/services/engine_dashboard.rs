//! What the engine dashboard shows beyond the engine record (P4-T6, D23):
//! the engine's disk usage (read with `disk_usage`, refreshed while the
//! dashboard is open), the Colima version (one read-only `colima version`),
//! and five minutes of the engine's load. Docker reports load per
//! container, so the engine's CPU and memory are the sums over its running
//! containers; the dashboard feeds [`EngineSeries`] from the stats service
//! once a second. The UI reads the store; this file owns the I/O.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use dioxus::prelude::*;

use berth_core::{DiskUsage, EngineId, LiveStats};

use crate::services::stats::{HISTORY_MAX_POINTS, HISTORY_WINDOW_S};
use crate::services::Services;

/// How often the dashboard asks for a fresh `disk_usage` and a new load point.
pub const USAGE_EVERY: std::time::Duration = std::time::Duration::from_secs(30);
pub const SERIES_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq)]
pub enum UsageEntry {
    Ready(DiskUsage),
    /// The engine said no (stopped, unreachable); the text says why.
    Failed(String),
}

/// Five minutes of an engine's summed load: CPU as a share of the whole VM
/// (0 to 100), memory in bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineSeries {
    pub cpu: Vec<(DateTime<Utc>, f64)>,
    pub mem: Vec<(DateTime<Utc>, f64)>,
}

impl EngineSeries {
    /// Add one reading and drop what fell out of the window.
    pub fn push(&mut self, at: DateTime<Utc>, cpu_pct: f64, mem_bytes: f64) {
        let keep_from = at - Duration::seconds(HISTORY_WINDOW_S);
        for series in [&mut self.cpu, &mut self.mem] {
            series.retain(|(t, _)| *t >= keep_from);
        }
        self.cpu.push((at, cpu_pct));
        self.mem.push((at, mem_bytes));
        for series in [&mut self.cpu, &mut self.mem] {
            if series.len() > HISTORY_MAX_POINTS {
                series.remove(0);
            }
        }
    }

    #[must_use]
    pub fn latest(&self) -> Option<(f64, f64)> {
        Some((self.cpu.last()?.1, self.mem.last()?.1))
    }
}

/// The engine's load from its containers' latest samples: CPU is the sum of
/// the containers' percentages (each is a share of one core) spread over the
/// VM's cores, memory the sum of what they use. `None` without a sample.
#[must_use]
pub fn sum_load(samples: &[LiveStats], cores: u32) -> Option<(f64, f64)> {
    if samples.is_empty() {
        return None;
    }
    let cpu: f64 = samples.iter().map(|s| f64::from(s.cpu_pct)).sum();
    #[allow(clippy::cast_precision_loss)]
    let mem: f64 = samples.iter().map(|s| s.mem_used.as_u64() as f64).sum();
    Some(((cpu / f64::from(cores.max(1))).clamp(0.0, 100.0), mem))
}

/// The dashboard's data, by engine. Provided as context from the App root;
/// scenes seed it.
#[derive(Clone, Copy)]
pub struct DashboardStore {
    pub usage: Signal<HashMap<EngineId, UsageEntry>>,
    pub series: Signal<HashMap<EngineId, EngineSeries>>,
    pub colima_version: Signal<Option<String>>,
    asked_version: Signal<bool>,
}

impl DashboardStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            usage: Signal::new(HashMap::new()),
            series: Signal::new(HashMap::new()),
            colima_version: Signal::new(None),
            asked_version: Signal::new(false),
        }
    }

    /// Put readings in as if they had been loaded (scenes and tests).
    pub fn seed(&self, engine: &EngineId, usage: Option<UsageEntry>, series: Option<EngineSeries>) {
        let mut usage_map = self.usage;
        let mut series_map = self.series;
        if let Some(entry) = usage {
            usage_map.write().insert(engine.clone(), entry);
        }
        if let Some(points) = series {
            series_map.write().insert(engine.clone(), points);
        }
    }

    pub fn seed_version(&self, version: &str) {
        let mut slot = self.colima_version;
        let mut asked = self.asked_version;
        slot.set(Some(version.to_string()));
        asked.set(true);
    }

    /// One load point for `engine`.
    pub fn push_load(&self, engine: &EngineId, at: DateTime<Utc>, cpu_pct: f64, mem_bytes: f64) {
        let mut series = self.series;
        series
            .write()
            .entry(engine.clone())
            .or_default()
            .push(at, cpu_pct, mem_bytes);
    }

    /// Read the engine's `disk_usage` (the client's own 60 s budget, AD29) and
    /// store it. Replaces nothing until the answer is in, so the bar does not
    /// flash. The store's task outlives the caller: Clean up asks for a
    /// fresh reading and closes its sheet in the same breath.
    pub fn refresh_usage(&self, services: &Services, engine: &EngineId) {
        let mut usage = self.usage;
        let Some(client) = services.client(engine) else {
            usage.write().insert(
                engine.clone(),
                UsageEntry::Failed("The engine is not connected".to_string()),
            );
            return;
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        crate::services::actions::spawn_io(async move {
            let result = client.disk_usage().await.map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
        let id = engine.clone();
        dioxus::core::spawn_forever(async move {
            let Ok(result) = rx.await else { return };
            let entry = match result {
                Ok(found) => UsageEntry::Ready(found),
                Err(message) => UsageEntry::Failed(message),
            };
            usage.write().insert(id, entry);
        });
    }

    /// Ask `colima version` once per run.
    pub fn ensure_version(&self, mock: bool) {
        let mut asked = self.asked_version;
        if *asked.peek() {
            return;
        }
        asked.set(true);
        let (tx, rx) = tokio::sync::oneshot::channel();
        crate::services::actions::spawn_io(async move {
            let _ = tx.send(crate::services::colima::version(mock).await);
        });
        let mut slot = self.colima_version;
        crate::services::actions::spawn_job(async move {
            if let Ok(version) = rx.await {
                slot.set(version);
            }
        });
    }
}

impl Default for DashboardStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Provide one [`DashboardStore`] from the App root.
pub fn provide_dashboard_store() -> DashboardStore {
    use_hook(|| provide_context(DashboardStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytesize::ByteSize;

    fn sample(cpu: f32, mb: u64) -> LiveStats {
        LiveStats {
            cpu_pct: cpu,
            mem_used: ByteSize::mb(mb),
            mem_limit: ByteSize::mb(512),
            net_rx: ByteSize(0),
            net_tx: ByteSize(0),
            at: Utc::now(),
        }
    }

    #[test]
    fn engines_load_sums_containers_and_spreads_cpu_over_cores() {
        let (cpu, mem) = sum_load(&[sample(40.0, 100), sample(55.0, 250)], 4).expect("two samples");
        assert!((cpu - 23.75).abs() < 0.001, "{cpu}");
        assert!((mem - 350_000_000.0).abs() < 1.0, "{mem}");
        assert!(sum_load(&[], 4).is_none());
        // A busier machine than cores cannot show more than all of it.
        assert!((sum_load(&[sample(900.0, 1)], 2).expect("one").0 - 100.0).abs() < 0.001);
    }

    #[test]
    fn engines_series_keeps_five_minutes() {
        let now = Utc::now();
        let mut series = EngineSeries::default();
        series.push(now - Duration::seconds(400), 1.0, 1.0);
        series.push(now - Duration::seconds(200), 2.0, 2.0);
        series.push(now, 3.0, 3.0);
        assert_eq!(series.cpu.len(), 2, "the 400 s old point is gone");
        assert_eq!(series.latest(), Some((3.0, 3.0)));
    }
}
