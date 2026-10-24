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

/// In-memory Colima for mock mode and tests. Takes the profile list directly
/// until P1-T8 loads fixtures/mock/berth.json.
pub struct MockColima {
    profiles: std::sync::Mutex<Vec<berth_core::ColimaProfile>>,
}

impl MockColima {
    pub fn new(profiles: Vec<berth_core::ColimaProfile>) -> Self {
        Self {
            profiles: std::sync::Mutex::new(profiles),
        }
    }

    fn set_running(&self, profile: &str, running: bool) {
        if let Ok(mut profiles) = self.profiles.lock() {
            if let Some(found) = profiles.iter_mut().find(|p| p.name == profile) {
                found.running = running;
            }
        }
    }
}

#[async_trait::async_trait]
impl ColimaControl for MockColima {
    fn binary(&self) -> Option<std::path::PathBuf> {
        // In the mock world colima exists; discovery keys off this.
        Some(std::path::PathBuf::from("colima"))
    }

    async fn profiles(&self) -> Result<Vec<berth_core::ColimaProfile>, EngineError> {
        self.profiles
            .lock()
            .map(|p| p.clone())
            .map_err(|_| EngineError::Docker("mock colima lock poisoned".to_string()))
    }

    async fn status(&self, profile: &str) -> Result<berth_core::ColimaProfile, EngineError> {
        self.profiles()
            .await?
            .into_iter()
            .find(|p| p.name == profile)
            .ok_or_else(|| EngineError::NotFound(format!("colima profile {profile}")))
    }

    fn start(
        &self,
        profile: &str,
        _opts: Option<berth_core::ColimaStartOptions>,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
        self.set_running(profile, true);
        mock_stream(format!("starting {profile}"))
    }

    fn stop(
        &self,
        profile: &str,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
        self.set_running(profile, false);
        mock_stream(format!("stopping {profile}"))
    }

    fn restart(
        &self,
        profile: &str,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
        self.set_running(profile, true);
        mock_stream(format!("restarting {profile}"))
    }
}

fn mock_stream(
    line: String,
) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
    futures::stream::iter([
        Ok(berth_sys::ProcessLine::Stdout(line)),
        Ok(berth_sys::ProcessLine::Exit(Some(0))),
    ])
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, running: bool) -> berth_core::ColimaProfile {
        berth_core::ColimaProfile {
            name: name.to_string(),
            running,
            arch: berth_core::Arch::Aarch64,
            cpus: 2,
            memory: bytesize::ByteSize(1024),
            disk: bytesize::ByteSize(2048),
            runtime: berth_core::Runtime::Docker,
            vm_type: Some(berth_core::VmType::Vz),
            rosetta: Some(false),
            mount_type: None,
            docker_socket: None,
        }
    }

    #[tokio::test]
    async fn colima_mock_start_stop_flips_running_and_streams_exit() {
        let mock = MockColima::new(vec![profile("default", false)]);
        assert_eq!(mock.binary(), Some(std::path::PathBuf::from("colima")));
        let items: Vec<_> = mock
            .start("default", None, tokio_util::sync::CancellationToken::new())
            .collect()
            .await;
        assert_eq!(
            items.last().and_then(|i| i.as_ref().ok()),
            Some(&berth_sys::ProcessLine::Exit(Some(0)))
        );
        assert!(mock.status("default").await.expect("status").running);
        let _: Vec<_> = mock
            .stop("default", tokio_util::sync::CancellationToken::new())
            .collect()
            .await;
        assert!(!mock.status("default").await.expect("status").running);
    }

    #[tokio::test]
    async fn colima_mock_status_unknown_profile_is_not_found() {
        let mock = MockColima::new(vec![]);
        let err = mock.status("nope").await.expect_err("missing profile");
        assert!(matches!(err, EngineError::NotFound(_)), "{err}");
    }
}
