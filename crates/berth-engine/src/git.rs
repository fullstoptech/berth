//! Git through the `git` CLI (D49, no libgit2).
//!
//! Runs `git -C <dir> status --porcelain=v2 --branch`,
//! `git -C <dir> remote get-url origin`, and the deploy-from-Git helpers
//! (P8-T5: `clone --progress`, `fetch`, `log --oneline HEAD..@{u}`,
//! `pull --ff-only`, `rev-parse --short HEAD`) through
//! `berth_sys::run_capture` with the caller's `CancellationToken`, the
//! binary resolved through `berth_sys::SearchPath`. The network-shaped
//! calls (clone, fetch, pull) set `GIT_TERMINAL_PROMPT=0` so a missing
//! credential fails fast instead of hanging on a prompt nobody can see,
//! and run without a wall-clock timeout: the token ends them (AD29).

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

/// Pure argument builder for `git clone --progress` (D49). The `--`
/// keeps a URL starting with `-` from being read as a flag.
#[must_use]
pub fn clone_args(
    url: &str,
    dir: &std::path::Path,
    branch: Option<&str>,
) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec!["clone".into(), "--progress".into()];
    if let Some(branch) = branch {
        args.push("--branch".into());
        args.push(branch.into());
    }
    args.push("--".into());
    args.push(url.into());
    args.push(dir.as_os_str().to_os_string());
    args
}

/// Pure argument builder for `git fetch` (D49).
#[must_use]
pub fn fetch_args(dir: &std::path::Path) -> Vec<std::ffi::OsString> {
    vec!["-C".into(), dir.as_os_str().to_os_string(), "fetch".into()]
}

/// Pure argument builder for `git pull --ff-only` (D49).
#[must_use]
pub fn pull_args(dir: &std::path::Path) -> Vec<std::ffi::OsString> {
    vec![
        "-C".into(),
        dir.as_os_str().to_os_string(),
        "pull".into(),
        "--ff-only".into(),
    ]
}

/// Pure argument builder for the incoming-commits listing (D49):
/// commits on the upstream branch that HEAD does not have yet.
#[must_use]
pub fn incoming_args(dir: &std::path::Path) -> Vec<std::ffi::OsString> {
    vec![
        "-C".into(),
        dir.as_os_str().to_os_string(),
        "log".into(),
        "--oneline".into(),
        "HEAD..@{u}".into(),
    ]
}

/// Pure argument builder for the current commit's short hash.
#[must_use]
pub fn head_args(dir: &std::path::Path) -> Vec<std::ffi::OsString> {
    vec![
        "-C".into(),
        dir.as_os_str().to_os_string(),
        "rev-parse".into(),
        "--short".into(),
        "HEAD".into(),
    ]
}

/// One line of `git log --oneline`: the abbreviated hash and the subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingCommit {
    pub hash: String,
    pub subject: String,
}

/// Parse `git log --oneline` output, newest first (git's order). Blank
/// lines are skipped; a line with no space is all hash, empty subject.
#[must_use]
pub fn parse_incoming(oneline_log: &str) -> Vec<IncomingCommit> {
    oneline_log
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| match line.split_once(' ') {
            Some((hash, subject)) => IncomingCommit {
                hash: hash.to_string(),
                subject: subject.trim().to_string(),
            },
            None => IncomingCommit {
                hash: line.trim().to_string(),
                subject: String::new(),
            },
        })
        .collect()
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
        self.run_with(args, Some(RUN_TIMEOUT), false, cancel).await
    }

    /// `network` calls (clone, fetch, pull) get no wall-clock timeout —
    /// the token ends them — and never prompt for credentials.
    async fn run_with(
        &self,
        args: &[std::ffi::OsString],
        timeout: Option<std::time::Duration>,
        network: bool,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<berth_sys::Output, EngineError> {
        let binary = self.binary().ok_or_else(Self::not_found)?;
        let env: Vec<(std::ffi::OsString, std::ffi::OsString)> = if network {
            vec![("GIT_TERMINAL_PROMPT".into(), "0".into())]
        } else {
            Vec::new()
        };
        berth_sys::run_capture(
            &binary,
            args,
            berth_sys::SpawnOpts {
                path: Some(self.search.path_var()),
                env,
                timeout,
                ..Default::default()
            },
            cancel,
        )
        .await
        .map_err(EngineError::from)
    }

    async fn run_network(
        &self,
        args: &[std::ffi::OsString],
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), EngineError> {
        let output = self.run_with(args, None, true, cancel).await?;
        if output.code != Some(0) {
            return Err(Self::output_err(&output));
        }
        Ok(())
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

    /// Clone `url` into `dir` (which git creates), optionally checking
    /// out `branch`. Ends when the token fires; a clone that git rejects
    /// (bad URL, auth) is an error carrying git's message.
    pub async fn clone_repo(
        &self,
        url: &str,
        dir: &std::path::Path,
        branch: Option<&str>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), EngineError> {
        self.run_network(&clone_args(url, dir, branch), cancel)
            .await
    }

    /// Fetch from the configured remotes so `@{u}` is current.
    pub async fn fetch(
        &self,
        dir: &std::path::Path,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), EngineError> {
        self.run_network(&fetch_args(dir), cancel).await
    }

    /// Fast-forward to the upstream branch; an error when that is not
    /// possible (diverged, dirty), never a merge.
    pub async fn pull_ff_only(
        &self,
        dir: &std::path::Path,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), EngineError> {
        self.run_network(&pull_args(dir), cancel).await
    }

    /// Commits the upstream branch has that HEAD lacks, newest first —
    /// the "N new commits" of D72. A branch with no upstream has no
    /// incoming commits to report, not an error.
    pub async fn incoming(
        &self,
        dir: &std::path::Path,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<Vec<IncomingCommit>, EngineError> {
        let output = self.run(&incoming_args(dir), cancel).await?;
        if output.code != Some(0) {
            if output.stderr.contains("no upstream") || output.stderr.contains("unknown revision") {
                return Ok(Vec::new());
            }
            return Err(Self::output_err(&output));
        }
        Ok(parse_incoming(&output.stdout))
    }

    /// The current commit's short hash (the project row's commit), or
    /// `None` outside a repository or before the first commit.
    pub async fn current_commit(
        &self,
        dir: &std::path::Path,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<Option<String>, EngineError> {
        let output = self.run(&head_args(dir), cancel).await?;
        if output.code != Some(0) {
            return Ok(None);
        }
        let hash = output.stdout.trim();
        Ok((!hash.is_empty()).then(|| hash.to_string()))
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

    #[test]
    fn git_clone_args_are_exactly_d49() {
        assert_eq!(
            as_strings(&clone_args(
                "https://example.com/shop.git",
                std::path::Path::new("/x/shop"),
                None
            )),
            [
                "clone",
                "--progress",
                "--",
                "https://example.com/shop.git",
                "/x/shop"
            ]
        );
        assert_eq!(
            as_strings(&clone_args(
                "git@example.com:shop.git",
                std::path::Path::new("/x/shop"),
                Some("dev")
            )),
            [
                "clone",
                "--progress",
                "--branch",
                "dev",
                "--",
                "git@example.com:shop.git",
                "/x/shop"
            ]
        );
    }

    #[test]
    fn git_deploy_args_are_exactly_d49() {
        let dir = std::path::Path::new("/x/shop");
        assert_eq!(as_strings(&fetch_args(dir)), ["-C", "/x/shop", "fetch"]);
        assert_eq!(
            as_strings(&pull_args(dir)),
            ["-C", "/x/shop", "pull", "--ff-only"]
        );
        assert_eq!(
            as_strings(&incoming_args(dir)),
            ["-C", "/x/shop", "log", "--oneline", "HEAD..@{u}"]
        );
        assert_eq!(
            as_strings(&head_args(dir)),
            ["-C", "/x/shop", "rev-parse", "--short", "HEAD"]
        );
    }

    #[test]
    fn parse_incoming_splits_hash_and_subject_newest_first() {
        assert_eq!(
            parse_incoming("d544a3d berthfx-app: note the fixture build\nb09ecba initial commit\n"),
            [
                IncomingCommit {
                    hash: "d544a3d".into(),
                    subject: "berthfx-app: note the fixture build".into()
                },
                IncomingCommit {
                    hash: "b09ecba".into(),
                    subject: "initial commit".into()
                },
            ]
        );
        assert!(parse_incoming("").is_empty());
        assert!(parse_incoming("\n").is_empty());
    }

    /// git with a fixed identity (and no signing), a 30 s bound and no
    /// credential prompts; `dir` None runs without `-C`.
    async fn git_ok(cli: &GitCli, dir: Option<&std::path::Path>, args: &[&str]) -> String {
        let binary = cli.binary().expect("git binary");
        let mut os: Vec<std::ffi::OsString> = Vec::new();
        if let Some(dir) = dir {
            os.push("-C".into());
            os.push(dir.as_os_str().to_os_string());
        }
        os.extend(
            [
                "-c",
                "user.name=Berth Fixtures",
                "-c",
                "user.email=fixtures@berth.invalid",
                "-c",
                "commit.gpgsign=false",
            ]
            .iter()
            .map(std::ffi::OsString::from),
        );
        os.extend(args.iter().map(std::ffi::OsString::from));
        let out = berth_sys::run_capture(
            &binary,
            &os,
            berth_sys::SpawnOpts {
                env: vec![("GIT_TERMINAL_PROMPT".into(), "0".into())],
                timeout: Some(std::time::Duration::from_secs(30)),
                ..Default::default()
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("git runs");
        assert_eq!(out.code, Some(0), "git {args:?}: {}", out.stderr);
        out.stdout.trim().to_string()
    }

    /// A temp bare repo shaped like `scripts/fx-git.sh` output: a compose
    /// app, two commits on `main`. Returns (bare path, HEAD short hash).
    async fn make_bare_fixture(
        cli: &GitCli,
        tmp: &std::path::Path,
    ) -> (std::path::PathBuf, String) {
        let src = tmp.join("src");
        git_ok(
            cli,
            None,
            &["init", "-q", "-b", "main", src.to_str().expect("utf8")],
        )
        .await;
        std::fs::write(
            src.join("compose.yaml"),
            "name: berthfx-app\nservices:\n  web:\n    image: nginx:alpine\n",
        )
        .expect("write compose");
        std::fs::write(src.join("CHANGELOG.md"), "# changelog\n").expect("write changelog");
        git_ok(cli, Some(&src), &["add", "-A"]).await;
        git_ok(cli, Some(&src), &["commit", "-q", "-m", "initial commit"]).await;
        std::fs::write(src.join("CHANGELOG.md"), "# changelog\n\n- two\n").expect("edit changelog");
        git_ok(cli, Some(&src), &["commit", "-q", "-am", "second commit"]).await;
        let head = git_ok(cli, Some(&src), &["rev-parse", "--short", "HEAD"]).await;
        let bare = tmp.join("berthfx-app.git");
        git_ok(
            cli,
            None,
            &[
                "clone",
                "-q",
                "--bare",
                src.to_str().expect("utf8"),
                bare.to_str().expect("utf8"),
            ],
        )
        .await;
        (bare, head)
    }

    #[tokio::test]
    async fn git_deploy_clone_lands_at_remote_head_with_the_app_tree() {
        let Some(cli) = cli() else { return };
        let tmp = tempfile::tempdir().expect("tempdir");
        let (bare, head) = make_bare_fixture(&cli, tmp.path()).await;
        let clone_dir = tmp.path().join("deployed");
        let token = tokio_util::sync::CancellationToken::new;

        cli.clone_repo(
            bare.to_str().expect("utf8"),
            &clone_dir,
            Some("main"),
            token(),
        )
        .await
        .expect("clone");

        assert_eq!(
            cli.current_commit(&clone_dir, token()).await.expect("head"),
            Some(head),
            "clone checked out the remote HEAD"
        );
        assert!(
            clone_dir.join("compose.yaml").is_file(),
            "detection-ready tree: compose.yaml present"
        );
        let info = cli
            .git_info(&clone_dir, token())
            .await
            .expect("git info")
            .expect("clone is a repo");
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert!(!info.dirty);
        assert!(
            cli.incoming(&clone_dir, token())
                .await
                .expect("incoming")
                .is_empty(),
            "a fresh clone has nothing incoming"
        );
    }

    #[tokio::test]
    async fn git_deploy_incoming_after_a_push_is_exactly_one_commit() {
        let Some(cli) = cli() else { return };
        let tmp = tempfile::tempdir().expect("tempdir");
        let (bare, _) = make_bare_fixture(&cli, tmp.path()).await;
        let token = tokio_util::sync::CancellationToken::new;
        let deployed = tmp.path().join("deployed");
        cli.clone_repo(bare.to_str().expect("utf8"), &deployed, None, token())
            .await
            .expect("clone deployed");

        // Another clone pushes a third commit.
        let other = tmp.path().join("other");
        git_ok(
            &cli,
            None,
            &[
                "clone",
                "-q",
                bare.to_str().expect("utf8"),
                other.to_str().expect("utf8"),
            ],
        )
        .await;
        std::fs::write(other.join("CHANGELOG.md"), "# changelog\n\n- three\n")
            .expect("edit changelog");
        git_ok(&cli, Some(&other), &["commit", "-q", "-am", "third commit"]).await;
        let new_head = git_ok(&cli, Some(&other), &["rev-parse", "--short", "HEAD"]).await;
        git_ok(&cli, Some(&other), &["push", "-q", "origin", "main"]).await;

        // Before a fetch the deployed clone cannot see it yet.
        assert!(cli
            .incoming(&deployed, token())
            .await
            .expect("incoming before fetch")
            .is_empty());
        cli.fetch(&deployed, token()).await.expect("fetch");
        let incoming = cli.incoming(&deployed, token()).await.expect("incoming");
        assert_eq!(
            incoming,
            [IncomingCommit {
                hash: new_head.clone(),
                subject: "third commit".into()
            }],
            "exactly 1 new commit"
        );

        cli.pull_ff_only(&deployed, token()).await.expect("pull");
        assert!(cli
            .incoming(&deployed, token())
            .await
            .expect("incoming after pull")
            .is_empty());
        assert_eq!(
            cli.current_commit(&deployed, token())
                .await
                .expect("head after pull"),
            Some(new_head)
        );
    }

    #[tokio::test]
    async fn git_deploy_clone_of_a_missing_repo_errors() {
        let Some(cli) = cli() else { return };
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("no-such.git");
        let result = cli
            .clone_repo(
                missing.to_str().expect("utf8"),
                &tmp.path().join("out"),
                None,
                tokio_util::sync::CancellationToken::new(),
            )
            .await;
        assert!(result.is_err(), "cloning a missing repo must fail");
    }

    #[tokio::test]
    async fn git_deploy_current_commit_and_incoming_outside_a_repo() {
        let Some(cli) = cli() else { return };
        let tmp = tempfile::tempdir().expect("tempdir");
        let token = tokio_util::sync::CancellationToken::new;
        assert_eq!(
            cli.current_commit(tmp.path(), token())
                .await
                .expect("no error outside a repo"),
            None
        );
        // A repo whose branch has no upstream reports no incoming.
        run_git(&cli, tmp.path(), &["init", "-b", "main"]).await;
        std::fs::write(tmp.path().join("a.txt"), "a").expect("write");
        git_ok(&cli, Some(tmp.path()), &["add", "-A"]).await;
        git_ok(&cli, Some(tmp.path()), &["commit", "-q", "-m", "one"]).await;
        assert!(cli
            .incoming(tmp.path(), token())
            .await
            .expect("incoming without upstream")
            .is_empty());
        assert!(cli
            .current_commit(tmp.path(), token())
            .await
            .expect("head")
            .is_some());
    }
}
