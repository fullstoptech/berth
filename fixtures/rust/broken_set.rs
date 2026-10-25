//! Shared by the live tests that need the `berthfx-broken` project (berth-rules `problems_it`,
//! berth-app `problems_it`): `#[path]`-included, so it uses std only and finds the repo through
//! the including crate's manifest dir (`crates/<name>`).
//!
//! The broken set holds containers named `berthfx-broken-*`, which the other live tests count
//! under their "berthfx" filter, so a test that needs it brings it up and takes it down itself.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

const BROKEN_PROJECT: &str = "berthfx-broken";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Takes `berthfx-broken` down (containers, volumes, orphans) when dropped, so a failing test
/// leaves nothing behind.
pub struct BrokenSet(());

/// Brings `berthfx-broken` up as `scripts/fx-up.sh --broken-only` does: the reset, then each
/// service on its own, failures tolerated (they are the point of the set).
pub fn broken_up() -> BrokenSet {
    // Built first so that a failed bring-up still tears down what it did create.
    let guard = BrokenSet(());
    let status = Command::new(repo_root().join("scripts/fx-up.sh"))
        .arg("--broken-only")
        .status()
        .unwrap_or_else(|e| panic!("scripts/fx-up.sh runs: {e}"));
    assert!(status.success(), "scripts/fx-up.sh --broken-only failed");
    guard
}

impl Drop for BrokenSet {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args([
                "compose",
                "-p",
                BROKEN_PROJECT,
                "down",
                "-v",
                "--remove-orphans",
            ])
            .output();
    }
}

/// For tests that count "berthfx" containers: panics with a clear message when the broken
/// project is up.
pub fn assert_broken_down() {
    let out = Command::new("docker")
        .args([
            "ps",
            "-aq",
            "--filter",
            &format!("label=com.docker.compose.project={BROKEN_PROJECT}"),
        ])
        .output()
        .unwrap_or_else(|e| panic!("docker runs: {e}"));
    assert!(
        String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        "{BROKEN_PROJECT} is up; tear it down (docker compose -p {BROKEN_PROJECT} down -v --remove-orphans)"
    );
}
