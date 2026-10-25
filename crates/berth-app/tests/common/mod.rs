#![allow(clippy::expect_used, dead_code)]
//! Shared by the live-engine tests (`*_it.rs`). nextest runs those one at a
//! time (`.config/nextest.toml`), but each leaves the berthfx containers
//! restarted with healthchecks still `starting`. Call [`settle_fixture`] first.

use std::time::{Duration, Instant};

const FIXTURE: [&str; 4] = [
    "berthfx-web-1",
    "berthfx-db-1",
    "berthfx-cache-1",
    "berthfx-logger-1",
];

fn docker(bin: &std::path::Path, host: &str, args: &[&str]) -> String {
    let out = std::process::Command::new(bin)
        .env("DOCKER_HOST", host)
        .args(args)
        .output()
        .expect("docker CLI runs");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Wait up to 30 s until all four berthfx containers are running and every
/// one that has a healthcheck reports `healthy`. Containers found stopped are
/// started (they are fixtures; nothing else is touched). Panics on timeout so
/// a sick fixture fails here, not in an unrelated assertion.
pub async fn settle_fixture() {
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima_cli = berth_engine::ColimaCli::new(search.clone());
    let engines = berth_engine::discovery::discover(&colima_cli, &[]).await;
    let host = engines
        .iter()
        .find(|e| e.id.0 == "colima:default" && e.state == berth_core::EngineState::Running)
        .and_then(|e| e.docker_host.clone())
        .expect("colima:default running");
    let bin = search
        .find("docker")
        .unwrap_or_else(|| std::path::PathBuf::from("docker"));

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        // One line per container: "<name> <status> <health or none>".
        let mut args = vec![
            "inspect",
            "-f",
            "{{.Name}} {{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}",
        ];
        args.extend(FIXTURE);
        let last = docker(&bin, &host, &args);
        let rows: Vec<Vec<&str>> = last
            .lines()
            .map(|l| l.split_whitespace().collect())
            .collect();
        let settled = rows.len() == FIXTURE.len()
            && rows.iter().all(|r| {
                r.len() == 3 && r[1] == "running" && (r[2] == "none" || r[2] == "healthy")
            });
        if settled {
            return;
        }
        for row in &rows {
            if row.len() == 3 && matches!(row[1], "exited" | "created") {
                docker(&bin, &host, &["start", row[0].trim_start_matches('/')]);
            }
        }
        assert!(
            Instant::now() < deadline,
            "berthfx fixture did not settle within 30 s:\n{last}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
