// markers around the PATH the login shell prints (rc files may print banners)
const BEGIN: &str = "__BERTH_PATH_BEGIN__";
const END: &str = "__BERTH_PATH_END__";
const SHELL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct SearchPath {
    dirs: Vec<std::path::PathBuf>,
}

fn push_unseen(
    dirs: &mut Vec<std::path::PathBuf>,
    candidates: impl IntoIterator<Item = std::path::PathBuf>,
) {
    for dir in candidates {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
}

impl SearchPath {
    fn from_parts(
        process_path: Option<&std::ffi::OsStr>,
        home: &std::path::Path,
        extra: Option<&std::ffi::OsStr>,
    ) -> Self {
        let mut dirs = Vec::new();
        if let Some(path) = process_path {
            push_unseen(&mut dirs, std::env::split_paths(path));
        }
        push_unseen(
            &mut dirs,
            [
                home.join(".local/bin"),
                std::path::PathBuf::from("/opt/homebrew/bin"),
                std::path::PathBuf::from("/usr/local/bin"),
            ],
        );
        if let Some(extra) = extra {
            push_unseen(&mut dirs, std::env::split_paths(extra));
        }
        Self { dirs }
    }

    pub fn from_process() -> Self {
        Self::from_parts(
            std::env::var_os("PATH").as_deref(),
            &crate::paths::home(),
            std::env::var_os("BERTH_EXTRA_PATH").as_deref(),
        )
    }

    pub async fn with_login_shell(self) -> Self {
        let enabled = std::env::var("BERTH_SHELL_PATH").map_or(true, |v| v != "0");
        let shell = std::env::var_os("SHELL")
            .filter(|v| !v.is_empty())
            .map_or_else(
                || std::path::PathBuf::from("/bin/sh"),
                std::path::PathBuf::from,
            );
        self.with_shell(Some(&shell), enabled, SHELL_TIMEOUT).await
    }

    async fn with_shell(
        mut self,
        shell: Option<&std::path::Path>,
        enabled: bool,
        timeout: std::time::Duration,
    ) -> Self {
        if !enabled || !cfg!(unix) {
            return self;
        }
        let Some(shell) = shell else { return self };
        if let Some(shell_path) = read_shell_path(shell, timeout).await {
            push_unseen(&mut self.dirs, std::env::split_paths(&shell_path));
        }
        self
    }

    pub fn find(&self, program: &str) -> Option<std::path::PathBuf> {
        if program.contains('/') {
            let candidate = std::path::PathBuf::from(program);
            return is_executable(&candidate).then_some(candidate);
        }
        let path_var = std::env::join_paths(&self.dirs).ok()?;
        let cwd = std::env::current_dir().ok()?;
        which::which_in(program, Some(path_var), cwd).ok()
    }

    pub fn path_var(&self) -> std::ffi::OsString {
        std::env::join_paths(&self.dirs).unwrap_or_default()
    }

    pub fn dirs(&self) -> &[std::path::PathBuf] {
        &self.dirs
    }
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}
#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

fn parse_shell_path(stdout: &str) -> Option<std::ffi::OsString> {
    if let Some(start) = stdout.find(BEGIN) {
        if let Some(end) = stdout[start..].find(END) {
            let inner = &stdout[start + BEGIN.len()..start + end];
            if !inner.is_empty() {
                return Some(std::ffi::OsString::from(inner));
            }
        }
    }
    // Fallback for shells that ran something else: the last non-empty line
    // that looks like a PATH (absolute entries separated by colons).
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| {
            line.starts_with('/') && line.contains(':') && !line.contains(char::is_whitespace)
        })
        .map(std::ffi::OsString::from)
}

/// Run `shell -ilc` and read the PATH it ends up with. Any failure gives None;
/// the child is killed when the timeout runs out.
pub async fn read_shell_path(
    shell: &std::path::Path,
    timeout: std::time::Duration,
) -> Option<std::ffi::OsString> {
    let script = format!("printf '%s' \"{BEGIN}${{PATH}}{END}\"");
    let args: Vec<std::ffi::OsString> = vec!["-ilc".into(), script.into()];
    let output = crate::process::run_capture(
        shell,
        &args,
        crate::process::SpawnOpts {
            timeout: Some(timeout),
            ..Default::default()
        },
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .ok()?;
    if output.code != Some(0) {
        return None;
    }
    parse_shell_path(&output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> std::path::PathBuf {
        std::path::PathBuf::from("/Users/tester")
    }

    #[test]
    fn from_parts_appends_known_dirs_and_dedupes() {
        let sp = SearchPath::from_parts(
            Some(std::ffi::OsStr::new("/usr/bin:/bin:/usr/bin")),
            &home(),
            Some(std::ffi::OsStr::new("/extra:/bin")),
        );
        let dirs: Vec<&str> = sp
            .dirs()
            .iter()
            .map(|d| d.to_str().expect("utf8"))
            .collect();
        assert_eq!(
            dirs,
            [
                "/usr/bin",
                "/bin",
                "/Users/tester/.local/bin",
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/extra"
            ]
        );
    }

    #[test]
    fn path_var_joins_dirs_in_order() {
        let sp = SearchPath::from_parts(Some(std::ffi::OsStr::new("/a:/b")), &home(), None);
        assert!(sp.path_var().to_string_lossy().starts_with("/a:/b:"));
    }

    #[cfg(unix)]
    fn write_executable(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    #[cfg(unix)]
    #[test]
    fn find_locates_executable_in_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let exe = write_executable(dir.path(), "berth-fake-tool", "#!/bin/sh\nexit 0\n");
        let sp = SearchPath {
            dirs: vec![dir.path().to_path_buf()],
        };
        assert_eq!(sp.find("berth-fake-tool"), Some(exe));
        assert_eq!(sp.find("berth-definitely-missing"), None);
    }

    #[test]
    fn find_rejects_missing_slash_path() {
        let sp = SearchPath { dirs: vec![] };
        assert_eq!(sp.find("/nonexistent/tool"), None);
    }

    #[test]
    fn parse_shell_path_reads_between_markers_past_a_banner() {
        let stdout =
            format!("rc banner here\nmore noise {BEGIN}/home/u/.local/bin:/usr/bin{END} trailing");
        assert_eq!(
            parse_shell_path(&stdout),
            Some(std::ffi::OsString::from("/home/u/.local/bin:/usr/bin"))
        );
    }

    #[test]
    fn parse_shell_path_falls_back_to_last_path_shaped_line() {
        assert_eq!(
            parse_shell_path("banner\n/usr/bin:/bin:/opt/x/bin\n"),
            Some(std::ffi::OsString::from("/usr/bin:/bin:/opt/x/bin"))
        );
        assert_eq!(parse_shell_path("just a banner\n"), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_shell_supplies_extra_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = format!(
            "#!/bin/sh\necho 'rc file banner'\nprintf '%s' \"{BEGIN}/from-shell/bin:/usr/bin{END}\"\n"
        );
        let shell = write_executable(dir.path(), "fake-shell", &script);
        let sp = SearchPath::from_parts(Some(std::ffi::OsStr::new("/usr/bin")), &home(), None)
            .with_shell(Some(&shell), true, std::time::Duration::from_secs(5))
            .await;
        assert!(
            sp.dirs()
                .contains(&std::path::PathBuf::from("/from-shell/bin")),
            "{:?}",
            sp.dirs()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hanging_shell_is_killed_by_the_timeout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let shell = write_executable(dir.path(), "hang-shell", "#!/bin/sh\nsleep 30\n");
        let base = SearchPath::from_parts(Some(std::ffi::OsStr::new("/usr/bin")), &home(), None);
        let before = base.dirs().to_vec();
        let started = std::time::Instant::now();
        let sp = base
            .with_shell(Some(&shell), true, std::time::Duration::from_millis(300))
            .await;
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
        assert_eq!(sp.dirs(), before.as_slice());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn disabled_lookup_skips_the_shell() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("shell-ran");
        let shell = write_executable(
            dir.path(),
            "marker-shell",
            &format!("#!/bin/sh\ntouch {}\n", marker.display()),
        );
        let base = SearchPath::from_parts(Some(std::ffi::OsStr::new("/usr/bin")), &home(), None);
        let _ = base
            .with_shell(Some(&shell), false, std::time::Duration::from_secs(1))
            .await;
        assert!(!marker.exists(), "shell ran while disabled");
    }

    #[tokio::test]
    async fn berth_shell_path_zero_disables_public_lookup() {
        // The only test that reads BERTH_SHELL_PATH through the public API;
        // other tests pass parameters explicitly, so this cannot race them.
        std::env::set_var("BERTH_SHELL_PATH", "0");
        let before = SearchPath::from_process().dirs().to_vec();
        let after = SearchPath::from_process().with_login_shell().await;
        std::env::remove_var("BERTH_SHELL_PATH");
        assert_eq!(after.dirs(), before.as_slice());
    }
}
