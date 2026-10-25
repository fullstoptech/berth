//! Git status through the `git` CLI (D49, no libgit2).
//!
//! Runs `git -C <dir> status --porcelain=v2 --branch` and
//! `git -C <dir> remote get-url origin` through `berth_sys::run_capture`
//! with the caller's `CancellationToken`, the binary resolved through
//! `berth_sys::SearchPath`.

use crate::projects::parse_git_status;
use crate::EngineError;

const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Pure argument builder for the status call.
#[must_use]
pub fn status_args(dir: &std::path::Path) -> Vec<std::ffi::OsString> {
    vec![
        "-C".into(),
        dir.as_os_str().to_os_string(),
        "status".into(),
        "--porcelain=v2".into(),
        "--branch".into(),
    ]
}

/// Pure argument builder for the origin remote call.
#[must_use]
pub fn remote_args(dir: &std::path::Path) -> Vec<std::ffi::OsString> {
    vec![
        "-C".into(),
        dir.as_os_str().to_os_string(),
        "remote".into(),
        "get-url".into(),
        "origin".into(),
    ]
}

pub struct GitCli {
    search: berth_sys::SearchPath,
}

impl GitCli {
    pub fn new(search: berth_sys::SearchPath) -> Self {
        Self { search }
    }

    pub fn binary(&self) -> Option<std::path::PathBuf> {
        self.search.find("git")
    }

    fn not_found() -> EngineError {
        EngineError::Sys(berth_sys::SysError::NotFound {
            program: "git".to_string(),
        })
    }

    fn output_err(output: &berth_sys::Output) -> EngineError {
        let message = if output.stderr.trim().is_empty() {
            output.stdout.trim()
        } else {
            output.stderr.trim()
        };
        EngineError::Docker(message.to_string())
    }

    async fn run(
        &self,
        args: &[std::ffi::OsString],
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<berth_sys::Output, EngineError> {
        let binary = self.binary().ok_or_else(Self::not_found)?;
        berth_sys::run_capture(
            &binary,
            args,
            berth_sys::SpawnOpts {
                path: Some(self.search.path_var()),
                timeout: Some(RUN_TIMEOUT),
                ..Default::default()
            },
            cancel,
        )
        .await
        .map_err(EngineError::from)
    }

    /// Porcelain status of `dir`; an error when git itself fails (for a
    /// non-repository `git_info` is the friendlier entry point).
    pub async fn status(
        &self,
        dir: &std::path::Path,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<berth_core::GitInfo, EngineError> {
        let output = self.run(&status_args(dir), cancel).await?;
        if output.code != Some(0) {
            return Err(Self::output_err(&output));
        }
        Ok(parse_git_status(&output.stdout))
    }

    /// The `origin` URL, or `None` when the repo has no origin remote.
    pub async fn remote_url(
        &self,
        dir: &std::path::Path,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<Option<String>, EngineError> {
        let output = self.run(&remote_args(dir), cancel).await?;
        if output.code != Some(0) {
            return Ok(None);
        }
        let url = output.stdout.trim();
        Ok((!url.is_empty()).then(|| url.to_string()))
    }

    /// Status plus origin remote; `None` when `dir` is not in a git
    /// repository, an error on any other git failure.
    pub async fn git_info(
        &self,
        dir: &std::path::Path,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<Option<berth_core::GitInfo>, EngineError> {
        let output = self.run(&status_args(dir), cancel.clone()).await?;
        if output.code != Some(0) {
            if output.stderr.contains("not a git repository") {
                return Ok(None);
            }
            return Err(Self::output_err(&output));
        }
        let mut info = parse_git_status(&output.stdout);
        info.remote = self.remote_url(dir, cancel).await?;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_strings(args: &[std::ffi::OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn git_status_args_are_exactly_d49() {
        assert_eq!(
            as_strings(&status_args(std::path::Path::new("/x/shop"))),
            ["-C", "/x/shop", "status", "--porcelain=v2", "--branch"]
        );
    }

    #[test]
    fn git_remote_args_are_exactly_d49() {
        assert_eq!(
            as_strings(&remote_args(std::path::Path::new("/x/shop"))),
            ["-C", "/x/shop", "remote", "get-url", "origin"]
        );
    }

    fn cli() -> Option<GitCli> {
        let cli = GitCli::new(berth_sys::SearchPath::from_process());
        cli.binary().map(|_| cli)
    }

    async fn run_git(cli: &GitCli, dir: &std::path::Path, args: &[&str]) {
        let binary = cli.binary().expect("git binary");
        let os: Vec<std::ffi::OsString> = std::iter::once(std::ffi::OsString::from("-C"))
            .chain(std::iter::once(dir.as_os_str().to_os_string()))
            .chain(args.iter().map(std::ffi::OsString::from))
            .collect();
        let out = berth_sys::run_capture(
            &binary,
            &os,
            berth_sys::SpawnOpts::default(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("git runs");
        assert_eq!(out.code, Some(0), "git {args:?}: {}", out.stderr);
    }

    #[tokio::test]
    async fn git_info_reads_branch_dirty_and_remote_from_a_real_repo() {
        let Some(cli) = cli() else { return };
        let dir = tempfile::tempdir().expect("tempdir");
        run_git(&cli, dir.path(), &["init", "-b", "main"]).await;
        run_git(
            &cli,
            dir.path(),
            &["remote", "add", "origin", "https://example.com/shop.git"],
        )
        .await;
        std::fs::write(dir.path().join("new.txt"), "hi").expect("write file");

        let info = cli
            .git_info(dir.path(), tokio_util::sync::CancellationToken::new())
            .await
            .expect("git info")
            .expect("a repo gives info");
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert!(info.dirty, "untracked file marks the tree dirty");
        assert_eq!(info.remote.as_deref(), Some("https://example.com/shop.git"));
    }

    #[tokio::test]
    async fn git_info_of_a_plain_folder_is_none() {
        let Some(cli) = cli() else { return };
        let dir = tempfile::tempdir().expect("tempdir");
        let info = cli
            .git_info(dir.path(), tokio_util::sync::CancellationToken::new())
            .await
            .expect("no error outside a repo");
        assert_eq!(info, None);
    }

    #[tokio::test]
    async fn git_remote_url_without_origin_is_none() {
        let Some(cli) = cli() else { return };
        let dir = tempfile::tempdir().expect("tempdir");
        run_git(&cli, dir.path(), &["init", "-b", "main"]).await;
        let url = cli
            .remote_url(dir.path(), tokio_util::sync::CancellationToken::new())
            .await
            .expect("remote call");
        assert_eq!(url, None);
    }
}
