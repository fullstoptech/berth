use futures::StreamExt;

use crate::colima_parse;
use crate::EngineError;

// All literal colima arguments live in colima_parse::start_args and this
// table (D28), so a CLI change has two places to look, not twenty.
const LIST_ARGS: [&str; 2] = ["list", "--json"];
const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[async_trait::async_trait]
pub trait ColimaControl: Send + Sync {
    fn binary(&self) -> Option<std::path::PathBuf>;
    async fn profiles(&self) -> Result<Vec<berth_core::ColimaProfile>, EngineError>;
    async fn status(&self, profile: &str) -> Result<berth_core::ColimaProfile, EngineError>;
    fn start(
        &self,
        profile: &str,
        opts: Option<berth_core::ColimaStartOptions>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>>;
    fn stop(
        &self,
        profile: &str,
        cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>>;
    fn restart(
        &self,
        profile: &str,
        cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>>;
}

fn yaml_for(profile: &str) -> Option<String> {
    std::fs::read_to_string(
        berth_sys::paths::colima_dir()
            .join(profile)
            .join("colima.yaml"),
    )
    .ok()
}

pub struct ColimaCli {
    search: berth_sys::SearchPath,
}

impl ColimaCli {
    pub fn new(search: berth_sys::SearchPath) -> Self {
        Self { search }
    }

    fn not_found() -> EngineError {
        EngineError::Sys(berth_sys::SysError::NotFound {
            program: "colima".to_string(),
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

    async fn run(&self, args: &[std::ffi::OsString]) -> Result<berth_sys::Output, EngineError> {
        let binary = self.binary().ok_or_else(Self::not_found)?;
        berth_sys::run_capture(
            &binary,
            args,
            berth_sys::SpawnOpts {
                path: Some(self.search.path_var()),
                timeout: Some(RUN_TIMEOUT),
                ..Default::default()
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .map_err(EngineError::from)
    }

    fn stream(
        &self,
        args: Vec<std::ffi::OsString>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
        let Some(binary) = self.binary() else {
            return futures::stream::once(async { Err(Self::not_found()) }).boxed();
        };
        berth_sys::spawn_lines(
            &binary,
            &args,
            berth_sys::SpawnOpts {
                path: Some(self.search.path_var()),
                ..Default::default()
            },
            cancel,
        )
        .map(|item| item.map_err(EngineError::from))
        .boxed()
    }
}

fn os_args(parts: &[&str]) -> Vec<std::ffi::OsString> {
    parts.iter().map(std::ffi::OsString::from).collect()
}

#[async_trait::async_trait]
impl ColimaControl for ColimaCli {
    fn binary(&self) -> Option<std::path::PathBuf> {
        self.search.find("colima")
    }

    async fn profiles(&self) -> Result<Vec<berth_core::ColimaProfile>, EngineError> {
        let output = self.run(&os_args(&LIST_ARGS)).await?;
        if output.code != Some(0) {
            return Err(Self::output_err(&output));
        }
        let mut profiles = colima_parse::parse_list(&output.stdout)?;
        for profile in &mut profiles {
            if let Some(yaml) = yaml_for(&profile.name) {
                colima_parse::apply_yaml(profile, &yaml);
            }
        }
        // Running profiles carry no socket in list output; status has it.
        for profile in &mut profiles {
            if profile.running && profile.docker_socket.is_none() {
                if let Ok(status) = self.status(&profile.name).await {
                    profile.docker_socket = status.docker_socket;
                    profile.mount_type = status.mount_type;
                }
            }
        }
        Ok(profiles)
    }

    async fn status(&self, profile: &str) -> Result<berth_core::ColimaProfile, EngineError> {
        let output = self
            .run(&os_args(&["status", "-p", profile, "--json"]))
            .await?;
        if output.code != Some(0) {
            return Err(Self::output_err(&output));
        }
        let mut parsed = colima_parse::parse_status(&output.stdout)?;
        parsed.name = profile.to_string();
        if let Some(yaml) = yaml_for(profile) {
            colima_parse::apply_yaml(&mut parsed, &yaml);
        }
        Ok(parsed)
    }

    fn start(
        &self,
        profile: &str,
        opts: Option<berth_core::ColimaStartOptions>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
        let args = match &opts {
            Some(opts) => colima_parse::start_args(profile, opts, cfg!(target_os = "macos")),
            None => vec!["start".to_string(), "-p".to_string(), profile.to_string()],
        };
        self.stream(args.iter().map(std::ffi::OsString::from).collect(), cancel)
    }

    fn stop(
        &self,
        profile: &str,
        cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
        self.stream(os_args(&["stop", "-p", profile]), cancel)
    }

    fn restart(
        &self,
        profile: &str,
        cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
        self.stream(os_args(&["restart", "-p", profile]), cancel)
    }
}

pub use crate::mock::colima::{MockColima, MockColimaCall};
