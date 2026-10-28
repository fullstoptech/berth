#![allow(clippy::expect_used)]
//! Live registry login against the berthfx-registry fixture (P8-T2).
//! Run with: BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only registry_it
//!
//! The test brings the fixture project up and down itself (AD33:
//! `berthfx-*` project from the fixture folder, host port 56500).

use berth_registry::{RegistryClient, RegistryError};

/// The fixture password, read from the one place it may live
/// (`fixtures/`, PLAN secrets rule) instead of copied here.
fn fixture_password() -> String {
    let env_file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/compose/berthfx/.env"
    );
    let text = std::fs::read_to_string(env_file).expect("fixture .env");
    text.lines()
        .find_map(|line| line.strip_prefix("BERTHFX_DB_PASSWORD="))
        .expect("fixture password key")
        .trim()
        .to_string()
}

fn compose(args: &[&str]) {
    let file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/compose/berthfx-registry/compose.yaml"
    );
    let status = std::process::Command::new("docker")
        .args(["compose", "-f", file])
        .args(args)
        .status()
        .expect("docker compose runs");
    assert!(status.success(), "compose {args:?} failed");
}

#[tokio::test]
#[ignore = "needs Docker (scripts/it.sh); run with BERTH_IT=1"]
async fn registry_it_wrong_password_fails_and_fixture_password_logs_in() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    compose(&["up", "-d", "--wait"]);
    let url = url::Url::parse("http://127.0.0.1:56500").expect("url");
    let client = RegistryClient::new();
    let wrong = client
        .check_login(
            &url,
            "berthfx",
            &secrecy::SecretString::from("wrong".to_string()),
        )
        .await;
    assert_eq!(wrong, Err(RegistryError::Unauthorized));
    client
        .check_login(
            &url,
            "berthfx",
            &secrecy::SecretString::from(fixture_password()),
        )
        .await
        .expect("login with the fixture password");
    compose(&["down", "-v"]);
}
