use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};

use crate::SysError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessLine {
    Stdout(String),
    Stderr(String),
    Exit(Option<i32>),
}

#[derive(Debug, Clone, Default)]
pub struct SpawnOpts {
    pub cwd: Option<std::path::PathBuf>,
    pub env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    pub path: Option<std::ffi::OsString>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: Option<std::time::Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

fn program_name(program: &std::path::Path) -> String {
    program
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_string_lossy().into_owned())
}

fn spawn_error(program: &std::path::Path, err: &std::io::Error) -> SysError {
    if err.kind() == std::io::ErrorKind::NotFound {
        SysError::NotFound {
            program: program_name(program),
        }
    } else {
        SysError::Spawn {
            program: program_name(program),
            message: err.to_string(),
        }
    }
}

fn spawn_reader<R>(reader: R, tx: tokio::sync::mpsc::UnboundedSender<ProcessLine>, is_stdout: bool)
where
    R: AsyncRead + Send + Unpin + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let item = if is_stdout {
                        ProcessLine::Stdout(line)
                    } else {
                        ProcessLine::Stderr(line)
                    };
                    if tx.send(item).is_err() {
                        return;
                    }
                }
                _ => return,
            }
        }
    });
}

/// Streams stdout and stderr line by line, then Exit. Kills the child when
/// `cancel` fires, when `opts.timeout` runs out, or the stream drops.
pub fn spawn_lines(
    program: &std::path::Path,
    args: &[std::ffi::OsString],
    opts: SpawnOpts,
    cancel: tokio_util::sync::CancellationToken,
) -> futures::stream::BoxStream<'static, Result<ProcessLine, SysError>> {
    let name = program_name(program);
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .kill_on_drop(true)
        .stdin(if opts.stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(cwd) = &opts.cwd {
        command.current_dir(cwd);
    }
    for (key, value) in &opts.env {
        command.env(key, value);
    }
    if let Some(path) = &opts.path {
        command.env("PATH", path);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            let err = spawn_error(program, &err);
            return futures::stream::once(async move { Err(err) }).boxed();
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let mut stdin = child.stdin.take();
    let timeout = opts.timeout;

    let (out_tx, out_rx) = futures::channel::mpsc::channel::<Result<ProcessLine, SysError>>(64);
    tokio::spawn(async move {
        let mut out_tx = out_tx;
        let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<ProcessLine>();
        if let Some(stdout) = stdout {
            spawn_reader(stdout, line_tx.clone(), true);
        }
        if let Some(stderr) = stderr {
            spawn_reader(stderr, line_tx.clone(), false);
        }
        drop(line_tx);

        if let Some(bytes) = opts.stdin {
            if let Some(input) = stdin.as_mut() {
                let _ = input.write_all(&bytes).await;
                let _ = input.shutdown().await;
            }
        }
        drop(stdin);

        let timeout_fut = async move {
            match timeout {
                Some(duration) => tokio::time::sleep(duration).await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(timeout_fut);
        let mut exit_code: Option<Option<i32>> = None;
        let mut lines_open = true;

        loop {
            if exit_code.is_some() && !lines_open {
                let _ = out_tx
                    .send(Ok(ProcessLine::Exit(exit_code.flatten())))
                    .await;
                return;
            }
            tokio::select! {
                line = line_rx.recv(), if lines_open => {
                    match line {
                        Some(line) => {
                            if out_tx.send(Ok(line)).await.is_err() {
                                let _ = child.kill().await;
                                return;
                            }
                        }
                        None => lines_open = false,
                    }
                }
                // Child::wait is cancel safe, so recreating it each pass is fine.
                status = child.wait(), if exit_code.is_none() => {
                    exit_code = Some(status.ok().and_then(|s| s.code()));
                }
                () = cancel.cancelled() => {
                    let _ = child.kill().await;
                    let _ = out_tx.send(Err(SysError::Cancelled)).await;
                    return;
                }
                () = &mut timeout_fut => {
                    let _ = child.kill().await;
                    let secs = timeout.map(|d| d.as_secs()).unwrap_or(0);
                    let _ = out_tx.send(Err(SysError::Timeout { program: name.clone(), secs })).await;
                    return;
                }
            }
        }
    });
    out_rx.boxed()
}

pub async fn run_capture(
    program: &std::path::Path,
    args: &[std::ffi::OsString],
    opts: SpawnOpts,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<Output, SysError> {
    let mut stream = spawn_lines(program, args, opts, cancel);
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut code = None;
    while let Some(item) = stream.next().await {
        match item? {
            ProcessLine::Stdout(line) => {
                stdout.push_str(&line);
                stdout.push('\n');
            }
            ProcessLine::Stderr(line) => {
                stderr.push_str(&line);
                stderr.push('\n');
            }
            ProcessLine::Exit(c) => code = c,
        }
    }
    Ok(Output {
        code,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh_args(script: &str) -> Vec<std::ffi::OsString> {
        vec!["-c".into(), script.into()]
    }
    fn token() -> tokio_util::sync::CancellationToken {
        tokio_util::sync::CancellationToken::new()
    }
    async fn collect(
        mut stream: futures::stream::BoxStream<'static, Result<ProcessLine, SysError>>,
    ) -> Vec<Result<ProcessLine, SysError>> {
        let mut out = Vec::new();
        while let Some(item) = stream.next().await {
            out.push(item);
        }
        out
    }

    #[tokio::test]
    async fn lines_keep_order_per_stream_and_exit_last() {
        let items = collect(spawn_lines(
            std::path::Path::new("/bin/sh"),
            &sh_args("echo out1; echo err1 >&2; echo out2; echo err2 >&2"),
            SpawnOpts::default(),
            token(),
        ))
        .await;
        let lines: Vec<&ProcessLine> = items.iter().filter_map(|i| i.as_ref().ok()).collect();
        let stdout: Vec<&str> = lines
            .iter()
            .filter_map(|l| match l {
                ProcessLine::Stdout(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        let stderr: Vec<&str> = lines
            .iter()
            .filter_map(|l| match l {
                ProcessLine::Stderr(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(stdout, ["out1", "out2"]);
        assert_eq!(stderr, ["err1", "err2"]);
        assert_eq!(lines.last(), Some(&&ProcessLine::Exit(Some(0))));
    }

    #[tokio::test]
    async fn nonzero_exit_code_is_reported() {
        let output = run_capture(
            std::path::Path::new("/bin/sh"),
            &sh_args("echo nope >&2; exit 3"),
            SpawnOpts::default(),
            token(),
        )
        .await
        .expect("capture runs");
        assert_eq!(output.code, Some(3));
        assert_eq!(output.stderr, "nope\n");
    }

    #[tokio::test]
    async fn missing_program_is_not_found() {
        let err = run_capture(
            std::path::Path::new("/nonexistent/berth-nope"),
            &[],
            SpawnOpts::default(),
            token(),
        )
        .await
        .expect_err("missing program fails");
        assert!(matches!(err, SysError::NotFound { .. }), "{err}");
    }

    /// Cancels `sleep 30` after 100 ms and returns how long the stream took to end, with what it
    /// yielded. The stream ending at all before the outer 5 s timeout proves the child was killed
    /// (it would sleep for 30 s).
    async fn cancel_a_sleeper() -> (std::time::Duration, Vec<Result<ProcessLine, SysError>>) {
        let cancel = token();
        let stream = spawn_lines(
            std::path::Path::new("/bin/sh"),
            &sh_args("sleep 30"),
            SpawnOpts::default(),
            cancel.clone(),
        );
        let collected = tokio::spawn(collect(stream));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let started = std::time::Instant::now();
        cancel.cancel();
        let items = tokio::time::timeout(std::time::Duration::from_secs(5), collected)
            .await
            .expect("stream ends")
            .expect("task joins");
        (started.elapsed(), items)
    }

    #[tokio::test]
    async fn cancel_kills_sleep_and_ends_the_stream_with_cancelled() {
        let (_, items) = cancel_a_sleeper().await;
        assert!(
            items.iter().any(|i| matches!(i, Err(SysError::Cancelled))),
            "{items:?}"
        );
    }

    #[tokio::test]
    #[ignore = "timing; run by scripts/perf.sh"]
    async fn perf_cancel_kills_sleep_within_500ms() {
        let (took, _) = cancel_a_sleeper().await;
        assert!(
            took < std::time::Duration::from_millis(500),
            "took {took:?}"
        );
    }

    #[tokio::test]
    async fn timeout_kills_child() {
        let err = run_capture(
            std::path::Path::new("/bin/sh"),
            &sh_args("sleep 30"),
            SpawnOpts {
                timeout: Some(std::time::Duration::from_millis(200)),
                ..Default::default()
            },
            token(),
        )
        .await
        .expect_err("times out");
        assert!(matches!(err, SysError::Timeout { .. }), "{err}");
    }

    #[tokio::test]
    async fn stdin_is_written_to_the_child() {
        let output = run_capture(
            std::path::Path::new("/bin/sh"),
            &sh_args("cat"),
            SpawnOpts {
                stdin: Some(b"hello stdin".to_vec()),
                ..Default::default()
            },
            token(),
        )
        .await
        .expect("cat runs");
        assert_eq!(output.stdout, "hello stdin\n");
        assert_eq!(output.code, Some(0));
    }

    #[tokio::test]
    async fn cwd_and_env_are_applied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output = run_capture(
            std::path::Path::new("/bin/sh"),
            &sh_args("pwd; echo \"v=$BERTH_TEST_V\""),
            SpawnOpts {
                cwd: Some(dir.path().to_path_buf()),
                env: vec![("BERTH_TEST_V".into(), "42".into())],
                ..Default::default()
            },
            token(),
        )
        .await
        .expect("runs in cwd");
        let canonical = dir.path().canonicalize().expect("canonical");
        assert!(
            output
                .stdout
                .contains(&canonical.to_string_lossy().into_owned()),
            "{}",
            output.stdout
        );
        assert!(output.stdout.contains("v=42"));
    }
}
