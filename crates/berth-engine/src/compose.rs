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
    ) -> Result<berth_sys::Output, EngineError> {
        let binary = self.binary().ok_or_else(Self::not_found)?;
        berth_sys::run_capture(
            &binary,
            &os_args(args),
            berth_sys::SpawnOpts {
                env,
                path: Some(self.search.path_var()),
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
        berth_sys::spawn_lines(
            &binary,
            &os_args(&args),
            berth_sys::SpawnOpts {
                env: vec![docker_host_env(target)],
                path: Some(self.search.path_var()),
                ..Default::default()
            },
            cancel,
        )
        .map(|item| match item {
            Ok(berth_sys::ProcessLine::Stdout(text)) => {
                Ok(line_event(berth_core::Stream::Stdout, text))
            }
            Ok(berth_sys::ProcessLine::Stderr(text)) => {
                Ok(line_event(berth_core::Stream::Stderr, text))
            }
            Ok(berth_sys::ProcessLine::Exit(code)) => Ok(berth_core::ComposeEvent::Done {
                code: code.unwrap_or(-1),
            }),
            Err(err) => Err(EngineError::from(err)),
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
        let output = self.capture(&args, vec![docker_host_env(target)]).await?;
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
        let output = self.capture(&args, Vec::new()).await?;
        if output.code == Some(0) {
            Ok(())
        } else {
            Err(Self::output_err(&output))
        }
    }
}

pub struct MockCompose;

#[async_trait::async_trait]
impl ComposeRunner for MockCompose {
    fn run(
        &self,
        target: &berth_core::ComposeTarget,
        _cmd: berth_core::ComposeCommand,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_core::ComposeEvent, EngineError>> {
        futures::stream::iter([
            Ok(berth_core::ComposeEvent::Progress {
                id: target.project_name.clone(),
                status: "Working".to_string(),
                text: None,
            }),
            Ok(berth_core::ComposeEvent::Done { code: 0 }),
        ])
        .boxed()
    }

    async fn config(
        &self,
        _target: &berth_core::ComposeTarget,
    ) -> Result<serde_json::Value, EngineError> {
        Ok(serde_json::json!({ "services": {} }))
    }

    async fn validate(
        &self,
        _folder: &std::path::Path,
        _files: &[std::path::PathBuf],
    ) -> Result<(), EngineError> {
        Ok(())
    }
}

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
        let mock = MockCompose;
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
        let mock = MockCompose;
        let config = mock.config(&target()).await.expect("mock config");
        assert!(config.get("services").is_some(), "{config}");
        mock.validate(std::path::Path::new("/work/shop"), &target().files)
            .await
            .expect("mock validate");
    }
}
