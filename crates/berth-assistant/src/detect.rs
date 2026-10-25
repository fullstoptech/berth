//! Finding the coding harnesses installed on this Mac (PLAN §6.9, D43).
//!
//! Only `--version` is ever run; no prompt reaches a harness here.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use berth_core::{Harness, HarnessId, HarnessTransport};
use berth_sys::{SearchPath, SpawnOpts};
use tokio_util::sync::CancellationToken;

/// A hung `--version` must not hold up the Settings screen or first run.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

struct Known {
    id: &'static str,
    name: &'static str,
    program: &'static str,
    transport: HarnessTransport,
}

const KNOWN: [Known; 2] = [
    Known {
        id: "claude-code",
        name: "Claude Code",
        program: "claude",
        transport: HarnessTransport::ClaudeStreamJson,
    },
    Known {
        id: "muse",
        name: "Muse Code",
        program: "muse",
        transport: HarnessTransport::MuseMsp,
    },
];

/// One entry per known harness, in a fixed order (Claude Code, then Muse).
/// A harness whose binary cannot be found is returned with `available: false`
/// and `binary` set to the bare program name (or the user's override).
///
/// `overrides` maps a harness id (`"claude-code"`, `"muse"`) to a binary the
/// user chose; it wins over the search path, even when the file has gone, so
/// Settings can show the broken choice instead of silently using another.
pub async fn detect(path: &SearchPath, overrides: &BTreeMap<String, PathBuf>) -> Vec<Harness> {
    detect_with_timeout(path, overrides, VERSION_TIMEOUT).await
}

async fn detect_with_timeout(
    path: &SearchPath,
    overrides: &BTreeMap<String, PathBuf>,
    timeout: Duration,
) -> Vec<Harness> {
    let [claude, muse] = &KNOWN;
    let (first, second) = tokio::join!(
        detect_one(path, overrides, claude, timeout),
        detect_one(path, overrides, muse, timeout)
    );
    vec![first, second]
}

async fn detect_one(
    path: &SearchPath,
    overrides: &BTreeMap<String, PathBuf>,
    known: &Known,
    timeout: Duration,
) -> Harness {
    let wanted = overrides
        .get(known.id)
        .cloned()
        .unwrap_or_else(|| PathBuf::from(known.program));
    let found = path.find(&wanted.to_string_lossy());
    let version = match &found {
        Some(binary) => read_version(binary, path, timeout).await,
        None => None,
    };
    Harness {
        id: HarnessId(known.id.to_string()),
        name: known.name.to_string(),
        binary: found.clone().unwrap_or(wanted),
        version,
        transport: known.transport,
        available: found.is_some(),
    }
}

async fn read_version(
    binary: &std::path::Path,
    path: &SearchPath,
    timeout: Duration,
) -> Option<String> {
    let output = berth_sys::run_capture(
        binary,
        &["--version".into()],
        SpawnOpts {
            // Node-script harnesses start through `env node`.
            path: Some(path.path_var()),
            timeout: Some(timeout),
            ..SpawnOpts::default()
        },
        CancellationToken::new(),
    )
    .await
    .ok()?;
    if output.code != Some(0) {
        return None;
    }
    parse_version(&output.stdout)
}

/// The first whitespace-separated token that starts with a digit:
/// `2.1.309 (Claude Code)` gives `2.1.309`, `Muse Code 1.4.12 (1.4.12-R6996.1)`
/// gives `1.4.12`.
fn parse_version(stdout: &str) -> Option<String> {
    stdout
        .split_whitespace()
        .find(|token| token.starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT: Duration = Duration::from_millis(400);

    #[test]
    fn parse_version_reads_both_real_output_shapes() {
        assert_eq!(
            parse_version("2.1.309 (Claude Code)\n").as_deref(),
            Some("2.1.309")
        );
        assert_eq!(
            parse_version("Muse Code 1.4.12 (1.4.12-R6996.1)\n").as_deref(),
            Some("1.4.12")
        );
        assert_eq!(parse_version("no numbers here\n"), None);
        assert_eq!(parse_version(""), None);
    }

    #[cfg(unix)]
    fn script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    #[cfg(unix)]
    fn fake_path(dir: &tempfile::TempDir) -> SearchPath {
        SearchPath::from_dirs(vec![dir.path().to_path_buf()])
    }

    #[cfg(unix)]
    fn by_id<'a>(found: &'a [Harness], id: &str) -> &'a Harness {
        found.iter().find(|h| h.id.0 == id).expect("harness listed")
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn finds_both_harnesses_with_their_versions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let claude = script(dir.path(), "claude", "echo '2.1.309 (Claude Code)'");
        let muse = script(
            dir.path(),
            "muse",
            "echo 'Muse Code 1.4.12 (1.4.12-R6996.1)'",
        );
        let found = detect_with_timeout(&fake_path(&dir), &BTreeMap::new(), SHORT).await;
        assert_eq!(found.len(), 2);
        let c = by_id(&found, "claude-code");
        assert_eq!(c.name, "Claude Code");
        assert_eq!(c.binary, claude);
        assert_eq!(c.version.as_deref(), Some("2.1.309"));
        assert_eq!(c.transport, HarnessTransport::ClaudeStreamJson);
        assert!(c.available);
        let m = by_id(&found, "muse");
        assert_eq!(m.binary, muse);
        assert_eq!(m.version.as_deref(), Some("1.4.12"));
        assert_eq!(m.transport, HarnessTransport::MuseMsp);
        assert!(m.available);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_missing_harness_is_listed_unavailable() {
        let dir = tempfile::tempdir().expect("tempdir");
        script(dir.path(), "muse", "echo 'Muse Code 1.4.12'");
        let found = detect_with_timeout(&fake_path(&dir), &BTreeMap::new(), SHORT).await;
        let claude = by_id(&found, "claude-code");
        assert!(!claude.available);
        assert_eq!(claude.version, None);
        assert_eq!(claude.binary, PathBuf::from("claude"));
        assert!(by_id(&found, "muse").available);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_user_chosen_binary_wins_over_the_search_path() {
        let on_path = tempfile::tempdir().expect("tempdir");
        script(on_path.path(), "claude", "echo '1.0.0 (on path)'");
        let elsewhere = tempfile::tempdir().expect("tempdir");
        let chosen = script(elsewhere.path(), "my-claude", "echo '9.9.9 (chosen)'");
        let overrides = BTreeMap::from([("claude-code".to_string(), chosen.clone())]);
        let found = detect_with_timeout(&fake_path(&on_path), &overrides, SHORT).await;
        let claude = by_id(&found, "claude-code");
        assert_eq!(claude.binary, chosen);
        assert_eq!(claude.version.as_deref(), Some("9.9.9"));
        assert!(claude.available);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_override_that_no_longer_exists_is_unavailable_not_replaced() {
        let on_path = tempfile::tempdir().expect("tempdir");
        script(on_path.path(), "claude", "echo '1.0.0'");
        let gone = PathBuf::from("/nonexistent/berth/claude");
        let overrides = BTreeMap::from([("claude-code".to_string(), gone.clone())]);
        let found = detect_with_timeout(&fake_path(&on_path), &overrides, SHORT).await;
        let claude = by_id(&found, "claude-code");
        assert!(!claude.available);
        assert_eq!(claude.binary, gone);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failing_version_command_keeps_the_binary_but_has_no_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        script(dir.path(), "claude", "echo '2.1.309' ; exit 3");
        let found = detect_with_timeout(&fake_path(&dir), &BTreeMap::new(), SHORT).await;
        let claude = by_id(&found, "claude-code");
        assert!(claude.available);
        assert_eq!(claude.version, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_hanging_binary_is_killed_by_the_timeout() {
        let dir = tempfile::tempdir().expect("tempdir");
        script(dir.path(), "claude", "exec /bin/sleep 30");
        script(dir.path(), "muse", "echo 'Muse Code 1.4.12'");
        let started = std::time::Instant::now();
        let found = detect_with_timeout(&fake_path(&dir), &BTreeMap::new(), SHORT).await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
        let claude = by_id(&found, "claude-code");
        assert!(claude.available);
        assert_eq!(claude.version, None);
        // The hang did not stop the other harness from being read.
        assert_eq!(by_id(&found, "muse").version.as_deref(), Some("1.4.12"));
    }

    /// Prints what this Mac really has. Run with
    /// `cargo nextest run -p berth-assistant --run-ignored only live_detect --no-capture`.
    /// Only `--version` runs; nothing is sent to a harness.
    #[ignore = "reads the real installed harnesses"]
    #[tokio::test]
    async fn live_detect_prints_the_installed_harnesses() {
        let path = SearchPath::from_process().with_login_shell().await;
        let found = detect(&path, &BTreeMap::new()).await;
        for harness in &found {
            println!(
                "{}: available={} version={:?} binary={}",
                harness.id.0,
                harness.available,
                harness.version,
                harness.binary.display()
            );
        }
        assert_eq!(found.len(), 2);
    }
}
