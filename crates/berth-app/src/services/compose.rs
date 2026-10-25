//! Project actions through `ComposeRunner` (D27, D34). Pure mappings from
//! a project action to a `ComposeCommand` and a `ComposeTarget`, plus the
//! one place a Compose event stream becomes `Ok` or an error message.

use std::sync::Arc;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

/// A project-level action (brief §7.2). `Delete` carries the options of the
/// delete sheet: named volumes and images are kept unless the user ticked
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectOp {
    Start,
    Stop,
    Restart,
    Rebuild,
    PullImages,
    Delete { volumes: bool, remove_images: bool },
}

impl ProjectOp {
    /// The `ActionDef` id; also the activity action and the pending key.
    #[must_use]
    pub fn action_id(self) -> &'static str {
        match self {
            Self::Start => "project.start",
            Self::Stop => "project.stop",
            Self::Restart => "project.restart",
            Self::Rebuild => "project.rebuild",
            Self::PullImages => "project.pull_images",
            Self::Delete { .. } => "project.delete",
        }
    }

    /// Start project = `Up`, Stop project = `Stop`, Rebuild = `Up{build}`,
    /// Delete project = `Down` with the sheet's options. No service list:
    /// a project action acts on the whole project.
    #[must_use]
    pub fn command(self) -> berth_core::ComposeCommand {
        use berth_core::ComposeCommand as C;
        match self {
            Self::Start => C::Up {
                services: Vec::new(),
                build: false,
            },
            Self::Stop => C::Stop {
                services: Vec::new(),
            },
            Self::Restart => C::Restart {
                services: Vec::new(),
            },
            Self::Rebuild => C::Up {
                services: Vec::new(),
                build: true,
            },
            Self::PullImages => C::Pull {
                services: Vec::new(),
            },
            Self::Delete {
                volumes,
                remove_images,
            } => C::Down {
                volumes,
                remove_images,
            },
        }
    }
}

/// Why a project cannot be driven through Compose, or its target. A
/// project needs a folder on this Mac and at least one known Compose file.
pub fn target_for(
    project: &berth_core::Project,
    docker_host: &str,
) -> Result<berth_core::ComposeTarget, String> {
    let Some(folder) = project.folder.clone() else {
        return Err("This project has no folder on this Mac".to_string());
    };
    if project.compose_files.is_empty() {
        return Err("No Compose file is known for this project".to_string());
    }
    Ok(berth_core::ComposeTarget {
        engine: project.engine.clone(),
        docker_host: docker_host.to_string(),
        project_name: project.name.clone(),
        folder,
        files: project.compose_files.clone(),
    })
}

/// How many trailing stderr lines make the failure message.
const TAIL_LINES: usize = 4;

/// Run one Compose command to its end. `Ok` only when the command reports
/// `Done { code: 0 }`; otherwise the message is the tail of stderr, else a
/// plain sentence naming the exit code.
pub async fn run(
    runner: &dyn berth_engine::ComposeRunner,
    target: &berth_core::ComposeTarget,
    cmd: berth_core::ComposeCommand,
    cancel: CancellationToken,
) -> Result<(), String> {
    let mut stream = runner.run(target, cmd, cancel);
    let mut tail: Vec<String> = Vec::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(berth_core::ComposeEvent::Done { code: 0 }) => return Ok(()),
            Ok(berth_core::ComposeEvent::Done { code }) => {
                return Err(if tail.is_empty() {
                    format!("docker compose exited with code {code}")
                } else {
                    tail.join("\n")
                });
            }
            Ok(berth_core::ComposeEvent::Line {
                stream: berth_core::Stream::Stderr,
                text,
            }) => {
                if !text.trim().is_empty() {
                    tail.push(text);
                    if tail.len() > TAIL_LINES {
                        tail.remove(0);
                    }
                }
            }
            Ok(_) => {}
            Err(err) => return Err(err.to_string()),
        }
    }
    Err("docker compose ended without a result".to_string())
}

/// The real runner (docker from `SearchPath`, D27) or, under `BERTH_MOCK=1`,
/// the recording mock.
pub async fn runner(mock: bool) -> Arc<dyn berth_engine::ComposeRunner> {
    if mock {
        return Arc::new(berth_engine::MockCompose::bundled());
    }
    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    Arc::new(berth_engine::CliCompose::new(search))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(folder: Option<&str>, files: &[&str]) -> berth_core::Project {
        berth_core::Project {
            id: berth_core::ProjectId("p1".to_string()),
            name: "shop".to_string(),
            folder: folder.map(std::path::PathBuf::from),
            compose_files: files.iter().map(std::path::PathBuf::from).collect(),
            engine: berth_core::EngineId("colima:default".to_string()),
            source: berth_core::ProjectSource::Compose,
            services: Vec::new(),
            git: None,
            status: berth_core::ProjectStatus::Stopped,
            pinned: false,
        }
    }

    #[test]
    fn actions_project_ops_map_to_the_compose_commands_of_the_task() {
        use berth_core::ComposeCommand as C;
        assert_eq!(
            ProjectOp::Start.command(),
            C::Up {
                services: vec![],
                build: false
            }
        );
        assert_eq!(ProjectOp::Stop.command(), C::Stop { services: vec![] });
        assert_eq!(
            ProjectOp::Restart.command(),
            C::Restart { services: vec![] }
        );
        assert_eq!(
            ProjectOp::Rebuild.command(),
            C::Up {
                services: vec![],
                build: true
            }
        );
        assert_eq!(
            ProjectOp::PullImages.command(),
            C::Pull { services: vec![] }
        );
        assert_eq!(
            ProjectOp::Delete {
                volumes: true,
                remove_images: false
            }
            .command(),
            C::Down {
                volumes: true,
                remove_images: false
            }
        );
    }

    #[test]
    fn actions_project_target_needs_a_folder_and_a_compose_file() {
        let ok = target_for(
            &project(Some("/w/shop"), &["/w/shop/compose.yaml"]),
            "unix:///sock",
        )
        .expect("target");
        assert_eq!(ok.project_name, "shop");
        assert_eq!(ok.docker_host, "unix:///sock");
        assert_eq!(ok.folder, std::path::PathBuf::from("/w/shop"));
        assert_eq!(ok.files.len(), 1);

        let no_folder = target_for(&project(None, &["/w/shop/compose.yaml"]), "h");
        assert_eq!(
            no_folder.expect_err("no folder"),
            "This project has no folder on this Mac"
        );
        let no_files = target_for(&project(Some("/w/shop"), &[]), "h");
        assert_eq!(
            no_files.expect_err("no files"),
            "No Compose file is known for this project"
        );
    }

    struct Scripted(Vec<Result<berth_core::ComposeEvent, berth_engine::EngineError>>);

    #[async_trait::async_trait]
    impl berth_engine::ComposeRunner for Scripted {
        fn run(
            &self,
            _target: &berth_core::ComposeTarget,
            _cmd: berth_core::ComposeCommand,
            _cancel: CancellationToken,
        ) -> futures::stream::BoxStream<
            'static,
            Result<berth_core::ComposeEvent, berth_engine::EngineError>,
        > {
            let items: Vec<_> = self
                .0
                .iter()
                .map(|r| match r {
                    Ok(e) => Ok(e.clone()),
                    Err(e) => Err(berth_engine::EngineError::Docker(e.to_string())),
                })
                .collect();
            futures::stream::iter(items).boxed()
        }
        async fn config(
            &self,
            _target: &berth_core::ComposeTarget,
        ) -> Result<serde_json::Value, berth_engine::EngineError> {
            Ok(serde_json::json!({}))
        }
        async fn validate(
            &self,
            _folder: &std::path::Path,
            _files: &[std::path::PathBuf],
        ) -> Result<(), berth_engine::EngineError> {
            Ok(())
        }
        async fn validate_text(
            &self,
            _folder: &std::path::Path,
            _compose_text: &str,
            _env: &[(String, String)],
        ) -> Result<(), berth_engine::EngineError> {
            Ok(())
        }
    }

    fn target() -> berth_core::ComposeTarget {
        target_for(
            &project(Some("/w/shop"), &["/w/shop/compose.yaml"]),
            "unix:///sock",
        )
        .expect("target")
    }

    #[tokio::test]
    async fn actions_compose_run_fails_with_the_stderr_tail_on_nonzero_exit() {
        let runner = Scripted(vec![
            Ok(berth_core::ComposeEvent::Line {
                stream: berth_core::Stream::Stderr,
                text: "network shop_default not found".to_string(),
            }),
            Ok(berth_core::ComposeEvent::Done { code: 1 }),
        ]);
        let result = run(
            &runner,
            &target(),
            ProjectOp::Start.command(),
            CancellationToken::new(),
        )
        .await;
        assert_eq!(result, Err("network shop_default not found".to_string()));
    }

    #[tokio::test]
    async fn actions_compose_run_succeeds_only_on_done_zero() {
        let ok = Scripted(vec![
            Ok(berth_core::ComposeEvent::Progress {
                id: "Container shop-web-1".to_string(),
                status: "Started".to_string(),
                text: None,
            }),
            Ok(berth_core::ComposeEvent::Done { code: 0 }),
        ]);
        assert_eq!(
            run(
                &ok,
                &target(),
                ProjectOp::Stop.command(),
                CancellationToken::new()
            )
            .await,
            Ok(())
        );
        let truncated = Scripted(vec![Ok(berth_core::ComposeEvent::Progress {
            id: "x".to_string(),
            status: "Working".to_string(),
            text: None,
        })]);
        assert!(run(
            &truncated,
            &target(),
            ProjectOp::Stop.command(),
            CancellationToken::new()
        )
        .await
        .is_err());
        let no_tail = Scripted(vec![Ok(berth_core::ComposeEvent::Done { code: 125 })]);
        assert_eq!(
            run(
                &no_tail,
                &target(),
                ProjectOp::Stop.command(),
                CancellationToken::new()
            )
            .await,
            Err("docker compose exited with code 125".to_string())
        );
    }
}
