//! The login shell's search path, resolved once (P6-T3, AD34). Asking `$SHELL -ilc` for its
//! PATH costs 100 ms to 3 s depending on the user's rc files, and seven services used to
//! start one each at launch. They all share this one answer instead.
//!
//! Only this module may call `SearchPath::with_login_shell` (a test reads the sources to
//! keep it that way). A path that cannot change at run time (`BERTH_SEARCH_PATH`) is
//! answered without a shell, as `with_login_shell` already does.

use tokio::sync::OnceCell;

static SHARED: OnceCell<berth_sys::SearchPath> = OnceCell::const_new();

/// `PATH`, the well-known dirs and whatever the login shell adds. The first caller pays for
/// the shell; the others wait for that same answer.
pub async fn search_path() -> berth_sys::SearchPath {
    SHARED
        .get_or_init(|| async {
            berth_sys::SearchPath::from_process()
                .with_login_shell()
                .await
        })
        .await
        .clone()
}

/// Start the shell now, on its own thread, so the first caller finds the answer ready and
/// the window and webview are built meanwhile. Call it first thing in `main`.
pub fn prewarm() {
    std::thread::spawn(|| {
        if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            runtime.block_on(search_path());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A login shell that records each run in a file and reports a PATH of its own.
    fn counting_shell(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let counter = dir.join("runs");
        let shell = dir.join("fake-shell");
        let script = format!(
            "#!/bin/sh\necho run >> '{}'\nprintf '%s' '__BERTH_PATH_BEGIN__/fake/login/bin__BERTH_PATH_END__'\n",
            counter.display()
        );
        std::fs::write(&shell, script).expect("write fake shell");
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake shell");
        (shell, counter)
    }

    #[tokio::test]
    async fn every_caller_shares_one_login_shell_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (shell, counter) = counting_shell(dir.path());
        std::env::remove_var("BERTH_SEARCH_PATH");
        std::env::remove_var("BERTH_SHELL_PATH");
        std::env::set_var("SHELL", &shell);

        // Seven services asked at start-up; ask the way they do, all at once.
        let answers = futures::future::join_all((0..7).map(|_| search_path())).await;
        let again = search_path().await;

        let runs = std::fs::read_to_string(&counter).unwrap_or_default();
        assert_eq!(runs.lines().count(), 1, "login shell ran: {runs:?}");
        for answer in answers.iter().chain([&again]) {
            assert!(
                answer
                    .dirs()
                    .iter()
                    .any(|d| d == std::path::Path::new("/fake/login/bin")),
                "{:?}",
                answer.dirs()
            );
        }
    }

    /// Walk the crate's sources for `needle`, skipping this file and the test files (live
    /// tests may start their own shell).
    fn files_mentioning(needle: &str) -> Vec<String> {
        fn walk(dir: &std::path::Path, needle: &str, out: &mut Vec<String>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, needle, out);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && !path.ends_with("services/login_path.rs")
                    && !path.to_string_lossy().ends_with("tests.rs")
                    && std::fs::read_to_string(&path).is_ok_and(|text| text.contains(needle))
                {
                    out.push(path.display().to_string());
                }
            }
        }
        let mut out = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            needle,
            &mut out,
        );
        out
    }

    #[test]
    fn no_other_code_starts_its_own_login_shell() {
        let offenders = files_mentioning("with_login_shell");
        assert!(
            offenders.is_empty(),
            "use services::login_path::search_path() instead: {offenders:?}"
        );
    }
}
