//! Scenes of the dev screens.

use super::*;

/// Marks the "Stats / panel" dev scene (context provided by its setup, read
/// by the shell): one mock container's Stats panel on a synthetic feed.
#[derive(Clone)]
pub(crate) struct DevStatsPanel(pub berth_core::ContainerId);

/// A smooth, deterministic reading for the dev scene: a container at ~3 %
/// CPU with bursts, memory creeping up and sawing down, network counters
/// that only grow. A function of absolute time, so a live feed continues
/// the seeded history without a seam.
pub(crate) fn synthetic_sample(at: chrono::DateTime<chrono::Utc>) -> berth_core::LiveStats {
    // Seconds since the mock world's reference moment, so the counters
    // start small instead of at the Unix epoch's 1.7 billion seconds.
    #[allow(clippy::cast_precision_loss)]
    let since = (at - crate::services::mock_reference_now())
        .num_seconds()
        .max(0) as f64;
    #[allow(clippy::cast_precision_loss)]
    let t = at.timestamp() as f64;
    let burst = ((t / 90.0).sin().max(0.8) - 0.8) * 30.0;
    let cpu = 3.2 + 2.2 * (t / 17.0).sin() + 1.4 * (t / 5.0).sin() + burst;
    let mem = 150_000_000.0 + 30_000_000.0 * ((t % 300.0) / 300.0) + 4_000_000.0 * (t / 11.0).sin();
    let rx = 1_200_000.0 + 3_000.0 * since + 2_500.0 * (since + 10.0 * (t / 10.0).sin());
    let tx = 340_000.0 + 900.0 * since + 700.0 * (since + 7.0 * (t / 7.0).sin());
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    berth_core::LiveStats {
        cpu_pct: cpu.max(0.1) as f32,
        mem_used: bytesize::ByteSize::b(mem as u64),
        mem_limit: bytesize::ByteSize::mb(512),
        net_rx: bytesize::ByteSize::b(rx as u64),
        net_tx: bytesize::ByteSize::b(tx as u64),
        at,
    }
}

/// "Stats / panel": the Stats panel of `shop-api-api-1` with five minutes of
/// synthetic history and a new sample every second while the window runs.
fn stats_panel(state: &mut AppState) {
    use crate::services::stats::{StatsKey, StatsService};
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Settings);
    let Some(container) = state
        .containers
        .peek()
        .iter()
        .find(|c| c.name == "shop-api-api-1")
        .cloned()
    else {
        return;
    };
    let service = try_consume_context::<StatsService>().unwrap_or_else(|| {
        provide_context(StatsService::new(
            crate::services::engines::client_map(),
            tokio_util::sync::CancellationToken::new(),
        ))
    });
    let key = StatsKey::new(&container.engine, &container.id);
    let now = chrono::Utc::now();
    service.seed(
        &key,
        (0..150).rev().map(|i| {
            let at = now - chrono::Duration::seconds(i * 2);
            (at, synthetic_sample(at))
        }),
    );
    let feed = service.clone();
    let feed_key = key.clone();
    spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let at = chrono::Utc::now();
            feed.seed(&feed_key, [(at, synthetic_sample(at))]);
        }
    });
    provide_context(DevStatsPanel(container.id));
}

/// Marks the "Files / panel" dev scene (context provided by its setup, read
/// by the shell): one mock container's Files tab over an in-memory folder.
#[derive(Clone)]
pub(crate) struct DevFilesPanel(pub berth_core::ContainerId);

/// "Files / panel": the Files tab of `shop-api-api-1`, opened on its `/app`
/// mount, over a small in-memory Node project.
fn files_panel(state: &mut AppState) {
    use crate::services::files::{FilesBackend, MemoryFiles};
    load_mock_world(state);
    state.settings.write().theme = berth_store::Theme::Light;
    state.section.set(Section::Settings);
    let mut containers = state.containers.peek().clone();
    let Some(container) = containers.iter_mut().find(|c| c.name == "shop-api-api-1") else {
        return;
    };
    container.mounts.insert(
        0,
        berth_core::Mount {
            kind: berth_core::MountKind::Bind,
            source: "/Users/berth/Dev/clients/acme/shop-api".to_string(),
            destination: "/app".to_string(),
            read_only: false,
            volume_name: None,
        },
    );
    let id = container.id.clone();
    state.containers.set(containers);
    let now = chrono::Utc::now().timestamp();
    let day = 86_400;
    let files = MemoryFiles::new()
        .with_dir("/app/node_modules", now - 3 * day)
        .with_file(
            "/app/node_modules/.package-lock.json",
            &[b'x'; 41_000],
            now - 3 * day,
        )
        .with_dir("/app/src", now - 3_600)
        .with_file("/app/src/server.js", &[b'x'; 3_210], now - 3_600)
        .with_dir("/app/public", now - 5 * day)
        .with_file("/app/.env.example", &[b'x'; 212], now - 40 * day)
        .with_file("/app/Dockerfile", &[b'x'; 418], now - 40 * day)
        .with_file("/app/package.json", &[b'x'; 1_187], now - 2 * day)
        .with_file("/app/package-lock.json", &[b'x'; 284_300], now - 2 * day)
        .with_file("/app/README.md", &[b'x'; 5_940], now - 400 * day)
        .with_file("/app/server.log", &[b'x'; 12_582_912], now - 90);
    provide_context(FilesBackend(std::sync::Arc::new(files)));
    provide_context(DevFilesPanel(id));
}

fn components_primitives(state: &mut AppState) {
    state.dev_gallery.set(true);
}

pub(super) const SCENES: &[Scene] = &[
    Scene::dev("Components / primitives", components_primitives),
    Scene::dev("Stats / panel", stats_panel),
    Scene::dev("Files / panel", files_panel),
];
