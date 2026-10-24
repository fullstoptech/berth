//! [`MockColima`]: in-memory Colima profiles for mock mode, loaded from
//! the mock world or handed a profile list directly.

use futures::StreamExt;

use super::MockWorld;
use crate::colima::ColimaControl;
use crate::EngineError;

pub struct MockColima {
    profiles: std::sync::Mutex<Vec<berth_core::ColimaProfile>>,
}

impl MockColima {
    pub fn new(profiles: Vec<berth_core::ColimaProfile>) -> Self {
        Self {
            profiles: std::sync::Mutex::new(profiles),
        }
    }

    pub fn from_fixture(json: &str) -> Result<Self, EngineError> {
        Ok(Self::from_world(&MockWorld::from_json(json)?))
    }

    #[must_use]
    pub fn from_world(world: &MockWorld) -> Self {
        Self::new(world.colima_profiles.clone())
    }

    #[must_use]
    pub fn bundled() -> Self {
        Self::from_world(&MockWorld::bundled())
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
    async fn mock_colima_start_stop_flips_running_and_streams_exit() {
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
    async fn mock_colima_status_unknown_profile_is_not_found() {
        let mock = MockColima::new(vec![]);
        let err = mock.status("nope").await.expect_err("missing profile");
        assert!(matches!(err, EngineError::NotFound(_)), "{err}");
    }

    #[tokio::test]
    async fn mock_colima_bundled_loads_world_profiles() {
        let mock = MockColima::bundled();
        let profiles = mock.profiles().await.expect("profiles");
        assert_eq!(profiles.len(), 2, "{profiles:?}");
        let default = mock.status("default").await.expect("default profile");
        assert!(default.running);
        assert_eq!(default.cpus, 4);
        assert_eq!(berth_core::fmt::bytes(default.memory), "8 GB");
        let work = mock.status("work").await.expect("work profile");
        assert!(!work.running);
        let from_fixture =
            MockColima::from_fixture(super::super::BUNDLED_JSON).expect("from fixture");
        assert_eq!(from_fixture.profiles().await.expect("profiles").len(), 2);
    }
}
