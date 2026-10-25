use futures::StreamExt;

use crate::EngineError;

const CAPTURE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[async_trait::async_trait]
pub trait ComposeRunner: Send + Sync {
    fn run(
        &self,
        target: &berth_core::ComposeTarget,
        cmd: berth_core::ComposeCommand,
        cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_core::ComposeEvent, EngineError>>;
    async fn config(
        &self,
        target: &berth_core::ComposeTarget,
    ) -> Result<serde_json::Value, EngineError>;
    async fn validate(
        &self,
        folder: &std::path::Path,
        files: &[std::path::PathBuf],
    ) -> Result<(), EngineError>;
    /// Validates compose text that is not on disk yet (`config -q -f -`, the text on stdin), so Review
    /// changes can check a proposed file before Write (AD33, ADR 0012). `env` holds the `.env` lines the
    /// plan adds; they reach the child as process environment, never as a file, and are not recorded.
    /// Relative paths resolve against `folder`. A `Docker(message)` error carries Compose's own words.
    ///
    /// `env_file: .env` pointing at a `.env` that is not on disk passes when `env` is not empty: a plan
    /// that adds `.env` lines creates that file. Any other missing `env_file` is an error.
    /// There is no cancellation token (the trait has none for `validate`); dropping the future kills the
    /// child, and the call gives up after 30 s.
    async fn validate_text(
        &self,
        folder: &std::path::Path,
        compose_text: &str,
        env: &[(String, String)],
    ) -> Result<(), EngineError>;
}

fn push_base(
    args: &mut Vec<String>,
    folder: &std::path::Path,
    files: &[std::path::PathBuf],
    project_name: Option<&str>,
) {
    args.push("compose".to_string());
    args.push("--project-directory".to_string());
    args.push(folder.to_string_lossy().into_owned());
    for file in files {
        args.push("-f".to_string());
        args.push(file.to_string_lossy().into_owned());
    }
    if let Some(name) = project_name {
        args.push("-p".to_string());
        args.push(name.to_string());
    }
}

/// Arguments of `docker compose` that validate the compose text on stdin: `-f -` and `config -q`.
pub fn validate_text_args(folder: &std::path::Path) -> Vec<String> {
    let mut args = Vec::new();
    push_base(&mut args, folder, &[std::path::PathBuf::from("-")], None);
    args.push("config".to_string());
    args.push("-q".to_string());
    args
}

/// The file Compose names in `env file <path> not found: ...`, if that is what `stderr` says.
fn missing_env_file(stderr: &str) -> Option<std::path::PathBuf> {
    stderr.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("env file ")?;
        let end = rest.find(" not found")?;
        Some(std::path::PathBuf::from(&rest[..end]))
    })
}

/// Whether `missing` is the project's own `.env`.
fn is_project_dotenv(missing: &std::path::Path, folder: &std::path::Path) -> bool {
    if missing.file_name().and_then(|n| n.to_str()) != Some(".env") {
        return false;
    }
    let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    missing
        .parent()
        .is_some_and(|parent| canon(parent) == canon(folder))
}

/// Pure argument builder for a Compose command (D27). The first item is the
/// `compose` subcommand of the `docker` binary; progress always streams as
/// JSON so `parse_progress_json` can map each line to a `ComposeEvent`.
pub fn compose_args(
    target: &berth_core::ComposeTarget,
    cmd: &berth_core::ComposeCommand,
) -> Vec<String> {
    let mut args = Vec::new();
    push_base(
        &mut args,
        &target.folder,
        &target.files,
        Some(&target.project_name),
    );
    args.push("--progress".to_string());
    args.push("json".to_string());
    match cmd {
        berth_core::ComposeCommand::Up { services, build } => {
            args.push("up".to_string());
            args.push("-d".to_string());
            if *build {
                args.push("--build".to_string());
            }
            args.extend(services.iter().cloned());
        }
        berth_core::ComposeCommand::Down {
            volumes,
            remove_images,
        } => {
            args.push("down".to_string());
            if *volumes {
                args.push("-v".to_string());
            }
            if *remove_images {
                args.push("--rmi".to_string());
                args.push("all".to_string());
            }
        }
        berth_core::ComposeCommand::Start { services } => {
            args.push("start".to_string());
            args.extend(services.iter().cloned());
        }
        berth_core::ComposeCommand::Stop { services } => {
            args.push("stop".to_string());
            args.extend(services.iter().cloned());
        }
        berth_core::ComposeCommand::Restart { services } => {
            args.push("restart".to_string());
            args.extend(services.iter().cloned());
        }
        berth_core::ComposeCommand::Build { services, pull } => {
            args.push("build".to_string());
            if *pull {
                args.push("--pull".to_string());
            }
            args.extend(services.iter().cloned());
        }
        berth_core::ComposeCommand::Pull { services } => {
            args.push("pull".to_string());
            args.extend(services.iter().cloned());
        }
    }
    args
}

/// Pure parser for one `--progress json` line. A progress object carries
/// string `id` and `status` fields (and usually `text`); anything else
/// (plain output, `level`/`msg` log lines, malformed JSON) is `None` so the
/// caller can surface it as a raw line instead.
pub fn parse_progress_json(line: &str) -> Option<berth_core::ComposeEvent> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let id = value.get("id")?.as_str()?.to_string();
    let status = value.get("status")?.as_str()?.to_string();
    let text = value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    Some(berth_core::ComposeEvent::Progress { id, status, text })
}

/// The containers the engine's classic builder is running for a Compose build, read from its own
/// output (`---> Running in 45f1f3e9c4cd`, then `Removing intermediate container 45f1f3e9c4cd` once
/// the step passes).
///
/// A step that fails is not removed: the engine keeps it, stopped, under a random name
/// (`eager_satoshi`) and nothing marks it as a build's. Compose offers no `--force-rm` and the
/// BuildKit route needs the `buildx` plugin, which Colima setups often lack, so Berth removes
/// exactly the containers its own build named and never failed to clean up.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BuildLeftovers {
    open: Vec<String>,
}

fn container_id_after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let rest = line.trim().split_once(marker)?.1;
    let id = rest.split_whitespace().next()?;
    ((12..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())).then_some(id)
}

impl BuildLeftovers {
    /// Learn from one output line.
    pub fn feed(&mut self, line: &str) {
        if let Some(id) = container_id_after(line, "---> Running in ") {
            self.open.push(id.to_string());
        } else if let Some(id) = container_id_after(line, "Removing intermediate container ") {
            self.open.retain(|open| open != id);
        }
    }

    /// The containers a failed step left, and forget them.
    pub fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.open)
    }
}

/// `docker rm <ids>`: without `-f`, so a container that is still running is refused, not killed.
#[must_use]
pub fn remove_leftovers_args(ids: &[String]) -> Vec<String> {
    std::iter::once("rm".to_string())
        .chain(ids.iter().cloned())
        .collect()
}

fn line_event(stream: berth_core::Stream, text: String) -> berth_core::ComposeEvent {
    parse_progress_json(&text).unwrap_or(berth_core::ComposeEvent::Line { stream, text })
}

fn os_args(parts: &[String]) -> Vec<std::ffi::OsString> {
    parts.iter().map(std::ffi::OsString::from).collect()
}

fn docker_host_env(target: &berth_core::ComposeTarget) -> (std::ffi::OsString, std::ffi::OsString) {
    (
        std::ffi::OsString::from("DOCKER_HOST"),
        std::ffi::OsString::from(&target.docker_host),
    )
}

fn note_build_line(leftovers: &std::sync::Mutex<BuildLeftovers>, text: &str) {
    if let Ok(mut l) = leftovers.lock() {
        l.feed(text);
    }
}

/// Best effort: a container that is already gone, or an engine that went away, changes nothing.
async fn remove_leftovers(
    binary: &std::path::Path,
    env: &[(std::ffi::OsString, std::ffi::OsString)],
    path_var: std::ffi::OsString,
    ids: &[String],
) {
    let _ = berth_sys::run_capture(
        binary,
        &os_args(&remove_leftovers_args(ids)),
        berth_sys::SpawnOpts {
            env: env.to_vec(),
            path: Some(path_var),
            timeout: Some(CAPTURE_TIMEOUT),
            ..Default::default()
        },
        tokio_util::sync::CancellationToken::new(),
    )
    .await;
}

pub struct CliCompose {
    search: berth_sys::SearchPath,
}

impl CliCompose {
    pub fn new(search: berth_sys::SearchPath) -> Self {
        Self { search }
    }

    fn binary(&self) -> Option<std::path::PathBuf> {
        self.search.find("docker")
    }

    fn not_found() -> EngineError {
        EngineError::Sys(berth_sys::SysError::NotFound {
            program: "docker".to_string(),
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

    async fn capture(
        &self,
        args: &[String],
        env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
        stdin: Option<Vec<u8>>,
    ) -> Result<berth_sys::Output, EngineError> {
        let binary = self.binary().ok_or_else(Self::not_found)?;
        berth_sys::run_capture(
            &binary,
            &os_args(args),
            berth_sys::SpawnOpts {
                env,
                path: Some(self.search.path_var()),
                stdin,
                timeout: Some(CAPTURE_TIMEOUT),
                ..Default::default()
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .map_err(EngineError::from)
    }
}

#[async_trait::async_trait]
impl ComposeRunner for CliCompose {
    fn run(
        &self,
        target: &berth_core::ComposeTarget,
        cmd: berth_core::ComposeCommand,
        cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_core::ComposeEvent, EngineError>> {
        let Some(binary) = self.binary() else {
            return futures::stream::once(async { Err(Self::not_found()) }).boxed();
        };
        let args = compose_args(target, &cmd);
        let builds = matches!(
            cmd,
            berth_core::ComposeCommand::Up { build: true, .. }
                | berth_core::ComposeCommand::Build { .. }
        );
        let leftovers = std::sync::Arc::new(std::sync::Mutex::new(BuildLeftovers::default()));
        let rm_env = vec![docker_host_env(target)];
        let path_var = self.search.path_var();
        let rm_binary = binary.clone();
        berth_sys::spawn_lines(
            &binary,
            &os_args(&args),
            berth_sys::SpawnOpts {
                env: vec![docker_host_env(target)],
                path: Some(path_var.clone()),
                ..Default::default()
            },
            cancel,
        )
        .then(move |item| {
            let leftovers = std::sync::Arc::clone(&leftovers);
            let (rm_binary, rm_env, path_var) =
                (rm_binary.clone(), rm_env.clone(), path_var.clone());
            async move {
                let event = match item {
                    Ok(berth_sys::ProcessLine::Stdout(text)) => {
                        if builds {
                            note_build_line(&leftovers, &text);
                        }
                        Ok(line_event(berth_core::Stream::Stdout, text))
                    }
                    Ok(berth_sys::ProcessLine::Stderr(text)) => {
                        if builds {
                            note_build_line(&leftovers, &text);
                        }
                        Ok(line_event(berth_core::Stream::Stderr, text))
                    }
                    Ok(berth_sys::ProcessLine::Exit(code)) => {
                        let left = leftovers.lock().map(|mut l| l.take()).unwrap_or_default();
                        if !left.is_empty() {
                            remove_leftovers(&rm_binary, &rm_env, path_var, &left).await;
                        }
                        Ok(berth_core::ComposeEvent::Done {
                            code: code.unwrap_or(-1),
                        })
                    }
                    Err(err) => Err(EngineError::from(err)),
                };
                event
            }
        })
        .boxed()
    }

    async fn config(
        &self,
        target: &berth_core::ComposeTarget,
    ) -> Result<serde_json::Value, EngineError> {
        let mut args = Vec::new();
        push_base(
            &mut args,
            &target.folder,
            &target.files,
            Some(&target.project_name),
        );
        args.push("config".to_string());
        args.push("--format".to_string());
        args.push("json".to_string());
        let output = self
            .capture(&args, vec![docker_host_env(target)], None)
            .await?;
        if output.code != Some(0) {
            return Err(Self::output_err(&output));
        }
        serde_json::from_str(&output.stdout).map_err(|err| EngineError::Parse {
            what: "compose config".to_string(),
            message: err.to_string(),
        })
    }

    async fn validate(
        &self,
        folder: &std::path::Path,
        files: &[std::path::PathBuf],
    ) -> Result<(), EngineError> {
        let mut args = Vec::new();
        push_base(&mut args, folder, files, None);
        args.push("config".to_string());
        args.push("-q".to_string());
        let output = self.capture(&args, Vec::new(), None).await?;
        if output.code == Some(0) {
            Ok(())
        } else {
            Err(Self::output_err(&output))
        }
    }

    async fn validate_text(
        &self,
        folder: &std::path::Path,
        compose_text: &str,
        env: &[(String, String)],
    ) -> Result<(), EngineError> {
        let process_env: Vec<_> = env
            .iter()
            .map(|(k, v)| (std::ffi::OsString::from(k), std::ffi::OsString::from(v)))
            .collect();
        let stdin = Some(compose_text.as_bytes().to_vec());
        let args = validate_text_args(folder);
        let output = self
            .capture(&args, process_env.clone(), stdin.clone())
            .await?;
        if output.code == Some(0) {
            return Ok(());
        }
        let excused = !env.is_empty()
            && missing_env_file(&output.stderr)
                .is_some_and(|missing| is_project_dotenv(&missing, folder));
        if !excused {
            return Err(Self::output_err(&output));
        }
        // The plan creates `.env`. Check the rest of the file against a mirror of the folder that has one.
        let mirror = DotenvMirror::new(folder)?;
        let args = validate_text_args(mirror.path());
        let output = self.capture(&args, process_env, stdin).await?;
        if output.code == Some(0) {
            Ok(())
        } else {
            Err(Self::output_err(&output))
        }
    }
}

/// A temporary project directory that links to everything in the real folder and holds an empty `.env`,
/// so Compose can resolve an `env_file` the plan has yet to create. Removed on drop.
struct DotenvMirror {
    dir: std::path::PathBuf,
}

impl DotenvMirror {
    fn new(folder: &std::path::Path) -> Result<Self, EngineError> {
        let io = |err: std::io::Error| EngineError::Docker(err.to_string());
        let dir =
            std::env::temp_dir().join(format!("berth-validate-{}", uuid::Uuid::now_v7().simple()));
        std::fs::create_dir(&dir).map_err(io)?;
        let mirror = Self { dir };
        #[cfg(unix)]
        for entry in std::fs::read_dir(folder).map_err(io)?.flatten() {
            let name = entry.file_name();
            if name != ".env" {
                // A link that fails to appear only means a different error later; nothing to report here.
                let _ = std::os::unix::fs::symlink(entry.path(), mirror.dir.join(&name));
            }
        }
        std::fs::write(mirror.dir.join(".env"), "").map_err(io)?;
        Ok(mirror)
    }

    fn path(&self) -> &std::path::Path {
        &self.dir
    }
}

impl Drop for DotenvMirror {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub use crate::mock::compose::MockCompose;

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> berth_core::ComposeTarget {
        berth_core::ComposeTarget {
            engine: berth_core::EngineId("colima:default".to_string()),
            docker_host: "unix:///tmp/docker.sock".to_string(),
            project_name: "shop".to_string(),
            folder: std::path::PathBuf::from("/work/shop"),
            files: vec![
                std::path::PathBuf::from("/work/shop/compose.yaml"),
                std::path::PathBuf::from("/work/shop/compose.override.yaml"),
            ],
        }
    }

    fn strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn compose_args_base_carries_folder_files_project_and_progress() {
        let args = compose_args(
            &target(),
            &berth_core::ComposeCommand::Pull { services: vec![] },
        );
        assert_eq!(
            args,
            strings(&[
                "compose",
                "--project-directory",
                "/work/shop",
                "-f",
                "/work/shop/compose.yaml",
                "-f",
                "/work/shop/compose.override.yaml",
                "-p",
                "shop",
                "--progress",
                "json",
                "pull",
            ])
        );
    }

    #[test]
    fn compose_args_up_is_detached_with_build_and_services() {
        let args = compose_args(
            &target(),
            &berth_core::ComposeCommand::Up {
                services: vec!["web".to_string(), "db".to_string()],
                build: true,
            },
        );
        assert_eq!(
            &args[args.len() - 5..],
            strings(&["up", "-d", "--build", "web", "db"]).as_slice()
        );
    }

    #[test]
    fn compose_args_up_without_build_or_services() {
        let args = compose_args(
            &target(),
            &berth_core::ComposeCommand::Up {
                services: vec![],
                build: false,
            },
        );
        assert_eq!(&args[args.len() - 2..], strings(&["up", "-d"]).as_slice());
        assert!(!args.iter().any(|a| a == "--build"));
    }

    #[test]
    fn compose_args_down_with_volumes_and_images() {
        let args = compose_args(
            &target(),
            &berth_core::ComposeCommand::Down {
                volumes: true,
                remove_images: true,
            },
        );
        assert_eq!(
            &args[args.len() - 4..],
            strings(&["down", "-v", "--rmi", "all"]).as_slice()
        );
    }

    #[test]
    fn compose_args_down_plain_has_no_flags() {
        let args = compose_args(
            &target(),
            &berth_core::ComposeCommand::Down {
                volumes: false,
                remove_images: false,
            },
        );
        assert_eq!(args.last().map(String::as_str), Some("down"));
        assert!(!args.iter().any(|a| a == "-v" || a == "--rmi"));
    }

    #[test]
    fn compose_args_start_stop_restart_carry_services() {
        let services = || vec!["web".to_string()];
        for (cmd, verb) in [
            (
                berth_core::ComposeCommand::Start {
                    services: services(),
                },
                "start",
            ),
            (
                berth_core::ComposeCommand::Stop {
                    services: services(),
                },
                "stop",
            ),
            (
                berth_core::ComposeCommand::Restart {
                    services: services(),
                },
                "restart",
            ),
        ] {
            let args = compose_args(&target(), &cmd);
            assert_eq!(
                &args[args.len() - 2..],
                strings(&[verb, "web"]).as_slice(),
                "{cmd:?}"
            );
        }
    }

    #[test]
    fn compose_args_build_pull_flag_and_services() {
        let args = compose_args(
            &target(),
            &berth_core::ComposeCommand::Build {
                services: vec!["api".to_string()],
                pull: true,
            },
        );
        assert_eq!(
            &args[args.len() - 3..],
            strings(&["build", "--pull", "api"]).as_slice()
        );
        let no_pull = compose_args(
            &target(),
            &berth_core::ComposeCommand::Build {
                services: vec![],
                pull: false,
            },
        );
        assert_eq!(no_pull.last().map(String::as_str), Some("build"));
        assert!(!no_pull.iter().any(|a| a == "--pull"));
    }

    #[test]
    fn compose_parse_progress_working_line_from_recording() {
        let event = parse_progress_json(
            r#"{"id":"Container berthfx-web-1","status":"Working","text":"Starting"}"#,
        )
        .expect("progress line parses");
        assert_eq!(
            event,
            berth_core::ComposeEvent::Progress {
                id: "Container berthfx-web-1".to_string(),
                status: "Working".to_string(),
                text: Some("Starting".to_string()),
            }
        );
    }

    #[test]
    fn compose_parse_progress_done_line_from_recording() {
        let event =
            parse_progress_json(r#"{"id":"Image nginx:alpine","status":"Done","text":"Pulled"}"#)
                .expect("progress line parses");
        assert_eq!(
            event,
            berth_core::ComposeEvent::Progress {
                id: "Image nginx:alpine".to_string(),
                status: "Done".to_string(),
                text: Some("Pulled".to_string()),
            }
        );
    }

    #[test]
    fn compose_parse_progress_without_text_gives_none_text() {
        let event = parse_progress_json(r#"{"id":"Container x-1","status":"Working"}"#)
            .expect("progress line parses");
        assert_eq!(
            event,
            berth_core::ComposeEvent::Progress {
                id: "Container x-1".to_string(),
                status: "Working".to_string(),
                text: None,
            }
        );
    }

    #[test]
    fn compose_parse_progress_rejects_non_progress_lines() {
        // Real non-progress line: `build` with nothing to build logs level/msg.
        assert_eq!(
            parse_progress_json(
                r#"{"level":"warning","msg":"No services to build","time":"2026-10-24T14:46:13+02:00"}"#
            ),
            None
        );
        assert_eq!(parse_progress_json("Container berthfx-web-1 Started"), None);
        assert_eq!(parse_progress_json(""), None);
        assert_eq!(parse_progress_json("{\"id\":\"x\"}"), None);
        assert_eq!(parse_progress_json("{\"status\":\"Working\"}"), None);
        assert_eq!(parse_progress_json("not json"), None);
        assert_eq!(parse_progress_json(r#"{"id":7,"status":"Working"}"#), None);
    }

    #[test]
    fn compose_parse_progress_reads_every_recorded_line() {
        for (name, text) in [
            (
                "up",
                include_str!("../../../fixtures/compose-progress/up.jsonl"),
            ),
            (
                "down",
                include_str!("../../../fixtures/compose-progress/down.jsonl"),
            ),
            (
                "pull",
                include_str!("../../../fixtures/compose-progress/pull.jsonl"),
            ),
        ] {
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            assert!(!lines.is_empty(), "{name}.jsonl is empty");
            for line in lines {
                assert!(
                    matches!(
                        parse_progress_json(line),
                        Some(berth_core::ComposeEvent::Progress { .. })
                    ),
                    "{name}.jsonl line did not parse as progress: {line}"
                );
            }
        }
    }

    #[tokio::test]
    async fn compose_mock_run_streams_progress_then_done_zero() {
        let mock = MockCompose::new();
        let items: Vec<_> = mock
            .run(
                &target(),
                berth_core::ComposeCommand::Restart {
                    services: vec!["web".to_string()],
                },
                tokio_util::sync::CancellationToken::new(),
            )
            .collect()
            .await;
        let events: Vec<_> = items
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("mock run succeeds");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, berth_core::ComposeEvent::Progress { .. })),
            "{events:?}"
        );
        assert_eq!(
            events.last(),
            Some(&berth_core::ComposeEvent::Done { code: 0 })
        );
    }

    #[tokio::test]
    async fn compose_mock_config_and_validate_succeed() {
        let mock = MockCompose::new();
        let config = mock.config(&target()).await.expect("mock config");
        assert!(config.get("services").is_some(), "{config}");
        mock.validate(std::path::Path::new("/work/shop"), &target().files)
            .await
            .expect("mock validate");
    }

    #[test]
    fn validate_text_args_read_the_file_from_stdin_and_only_check() {
        assert_eq!(
            validate_text_args(std::path::Path::new("/work/shop")),
            strings(&[
                "compose",
                "--project-directory",
                "/work/shop",
                "-f",
                "-",
                "config",
                "-q",
            ])
        );
    }

    #[test]
    fn missing_env_file_reads_the_path_from_composes_message() {
        let stderr = "level=warning msg=\"The PORT variable is not set.\"\n\
            env file /work/my shop/.env not found: stat /work/my shop/.env: no such file or directory\n";
        assert_eq!(
            missing_env_file(stderr),
            Some(std::path::PathBuf::from("/work/my shop/.env"))
        );
        assert_eq!(
            missing_env_file("services.web.image must be a string"),
            None
        );
        assert_eq!(missing_env_file(""), None);
    }

    #[test]
    fn only_the_project_dotenv_counts_as_created_by_the_plan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let folder = dir.path();
        assert!(is_project_dotenv(&folder.join(".env"), folder));
        // Another file in the folder, or a `.env` somewhere else, is the user's mistake.
        assert!(!is_project_dotenv(&folder.join(".env.local"), folder));
        assert!(!is_project_dotenv(&folder.join("api/.env"), folder));
        assert!(!is_project_dotenv(
            std::path::Path::new("/elsewhere/.env"),
            folder
        ));
    }

    #[cfg(unix)]
    #[test]
    fn dotenv_mirror_links_the_folder_adds_an_empty_dotenv_and_cleans_up() {
        let folder = tempfile::tempdir().expect("tempdir");
        std::fs::write(folder.path().join("compose.yaml"), "services: {}\n").expect("write");
        std::fs::create_dir(folder.path().join("api")).expect("mkdir");
        std::fs::write(folder.path().join("api/prod.env"), "A=1\n").expect("write");

        let mirror = DotenvMirror::new(folder.path()).expect("mirror");
        let dir = mirror.path().to_path_buf();
        assert_eq!(std::fs::read_to_string(dir.join(".env")).expect(".env"), "");
        assert_eq!(
            std::fs::read_to_string(dir.join("compose.yaml")).expect("linked file"),
            "services: {}\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("api/prod.env")).expect("file in linked dir"),
            "A=1\n"
        );
        // The real folder is untouched.
        assert!(!folder.path().join(".env").exists());

        drop(mirror);
        assert!(!dir.exists(), "the mirror is removed");
        assert!(
            folder.path().join("api/prod.env").exists(),
            "removing the mirror must not follow its links into the real folder"
        );
    }

    // --- Classic-builder leftovers ---

    const FAILED_BUILD: &[&str] = &[
        "Step 2/4 : RUN echo layer1 > /a",
        " ---> Running in 11aa22bb33cc",
        "Removing intermediate container 11aa22bb33cc",
        " ---> 73b62c093331",
        "Step 3/4 : RUN echo failing && exit 3",
        " ---> Running in 45f1f3e9c4cd",
        "failing",
    ];

    #[test]
    fn compose_build_leftovers_are_the_containers_of_steps_that_did_not_finish() {
        let mut left = BuildLeftovers::default();
        for line in FAILED_BUILD {
            left.feed(line);
        }
        assert_eq!(left.take(), ["45f1f3e9c4cd"]);
        assert!(left.take().is_empty(), "taken once");
    }

    #[test]
    fn compose_build_leftovers_are_none_when_every_step_passed() {
        let mut left = BuildLeftovers::default();
        for line in &FAILED_BUILD[..4] {
            left.feed(line);
        }
        assert!(left.take().is_empty());
    }

    #[test]
    fn compose_build_leftovers_ignore_lines_that_only_look_like_the_marker() {
        let mut left = BuildLeftovers::default();
        left.feed("Running in production mode");
        left.feed(" ---> Running in not-a-container");
        left.feed(" ---> Running in abc");
        left.feed("#7 [app 3/4] RUN pnpm install");
        assert!(left.take().is_empty());
    }

    #[test]
    fn compose_leftovers_are_removed_without_force() {
        let args = remove_leftovers_args(&["45f1f3e9c4cd".to_string(), "aa00bb11cc22".to_string()]);
        assert_eq!(args, ["rm", "45f1f3e9c4cd", "aa00bb11cc22"]);
        assert!(!args.iter().any(|a| a.starts_with('-')));
    }
}
