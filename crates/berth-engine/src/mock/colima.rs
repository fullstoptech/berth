//! [`MockColima`]: in-memory Colima profiles for mock mode, loaded from
//! the mock world or handed a profile list directly.

use futures::StreamExt;

use super::MockWorld;
use crate::colima::ColimaControl;
use crate::EngineError;

/// What the mock was asked to do, in order (tests read it back).
#[derive(Debug, Clone, PartialEq)]
pub enum MockColimaCall {
    Start {
        profile: String,
        opts: Option<berth_core::ColimaStartOptions>,
    },
    Stop {
        profile: String,
    },
    Restart {
        profile: String,
    },
}

pub struct MockColima {
    profiles: std::sync::Mutex<Vec<berth_core::ColimaProfile>>,
    calls: std::sync::Mutex<Vec<MockColimaCall>>,
    /// Failures queued for the next commands, oldest first: the command
    /// prints the text on stderr and exits with code 1, changing nothing.
    failures: std::sync::Mutex<std::collections::VecDeque<String>>,
    /// Pause between output lines, so a screen can show progress.
    line_delay: std::sync::Mutex<std::time::Duration>,
}

impl MockColima {
    pub fn new(profiles: Vec<berth_core::ColimaProfile>) -> Self {
        Self {
            profiles: std::sync::Mutex::new(profiles),
            calls: std::sync::Mutex::new(Vec::new()),
            failures: std::sync::Mutex::new(std::collections::VecDeque::new()),
            line_delay: std::sync::Mutex::new(std::time::Duration::ZERO),
        }
    }

    /// The next command fails with `message` (stderr, exit code 1) and
    /// leaves the profiles as they were.
    pub fn fail_next(&self, message: &str) {
        if let Ok(mut failures) = self.failures.lock() {
            failures.push_back(message.to_string());
        }
    }

    /// Wait this long between output lines.
    pub fn set_line_delay(&self, delay: std::time::Duration) {
        if let Ok(mut slot) = self.line_delay.lock() {
            *slot = delay;
        }
    }

    /// Every start, stop and restart asked for so far.
    #[must_use]
    pub fn calls(&self) -> Vec<MockColimaCall> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }

    fn record(&self, call: MockColimaCall) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    fn next_failure(&self) -> Option<String> {
        self.failures.lock().ok().and_then(|mut f| f.pop_front())
    }

    fn delay(&self) -> std::time::Duration {
        self.line_delay.lock().map(|d| *d).unwrap_or_default()
    }

    /// A started profile takes the resources it was started with, as Colima
    /// writes them into the profile's config.
    fn apply_options(&self, profile: &str, opts: &berth_core::ColimaStartOptions) {
        const GIB: u64 = 1 << 30;
        if let Ok(mut profiles) = self.profiles.lock() {
            if let Some(found) = profiles.iter_mut().find(|p| p.name == profile) {
                if let Some(cpus) = opts.cpus {
                    found.cpus = cpus;
                }
                if let Some(gib) = opts.memory_gib {
                    found.memory = bytesize::ByteSize(u64::from(gib) * GIB);
                }
                if let Some(gib) = opts.disk_gib {
                    found.disk = bytesize::ByteSize(u64::from(gib) * GIB);
                }
                if let Some(vm_type) = &opts.vm_type {
                    found.vm_type = Some(vm_type.clone());
                }
                if let Some(rosetta) = opts.rosetta {
                    found.rosetta = Some(rosetta);
                }
            }
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
        opts: Option<berth_core::ColimaStartOptions>,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
        self.record(MockColimaCall::Start {
            profile: profile.to_string(),
            opts: opts.clone(),
        });
        if let Some(message) = self.next_failure() {
            return failed_stream(message, self.delay());
        }
        if let Some(opts) = &opts {
            self.apply_options(profile, opts);
        }
        self.set_running(profile, true);
        script_stream(start_script(profile), self.delay())
    }

    fn stop(
        &self,
        profile: &str,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
        self.record(MockColimaCall::Stop {
            profile: profile.to_string(),
        });
        if let Some(message) = self.next_failure() {
            return failed_stream(message, self.delay());
        }
        self.set_running(profile, false);
        script_stream(
            vec![format!("INFO[0000] stopping colima [profile={profile}]")],
            self.delay(),
        )
    }

    fn restart(
        &self,
        profile: &str,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
        self.record(MockColimaCall::Restart {
            profile: profile.to_string(),
        });
        if let Some(message) = self.next_failure() {
            return failed_stream(message, self.delay());
        }
        self.set_running(profile, true);
        let mut lines = vec![format!("INFO[0000] stopping colima [profile={profile}]")];
        lines.extend(start_script(profile));
        script_stream(lines, self.delay())
    }
}

/// The lines a real `colima start` prints on stderr, in order.
fn start_script(profile: &str) -> Vec<String> {
    vec![
        format!("INFO[0000] starting colima [profile={profile}]"),
        "INFO[0000] runtime: docker".to_string(),
        "INFO[0001] starting ...  context=vm".to_string(),
        "INFO[0005] provisioning ...  context=docker".to_string(),
        "INFO[0007] waiting for docker socket ...  context=docker".to_string(),
    ]
}

/// Stderr lines (Colima logs there), a pause after each, then exit 0.
fn script_stream(
    lines: Vec<String>,
    delay: std::time::Duration,
) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
    let mut items: Vec<berth_sys::ProcessLine> = lines
        .into_iter()
        .map(berth_sys::ProcessLine::Stderr)
        .collect();
    items.push(berth_sys::ProcessLine::Exit(Some(0)));
    paced(items, delay)
}

/// The failure text on stderr, then exit code 1.
fn failed_stream(
    message: String,
    delay: std::time::Duration,
) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
    paced(
        vec![
            berth_sys::ProcessLine::Stderr(message),
            berth_sys::ProcessLine::Exit(Some(1)),
        ],
        delay,
    )
}

fn paced(
    items: Vec<berth_sys::ProcessLine>,
    delay: std::time::Duration,
) -> futures::stream::BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
    futures::stream::iter(items)
        .then(move |item| async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            Ok(item)
        })
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
    async fn mock_colima_start_with_options_changes_the_profile_and_is_recorded() {
        let mock = MockColima::new(vec![profile("default", false)]);
        let opts = berth_core::ColimaStartOptions {
            cpus: Some(6),
            memory_gib: Some(12),
            disk_gib: Some(80),
            vm_type: None,
            rosetta: Some(true),
            arch: None,
        };
        let _: Vec<_> = mock
            .start(
                "default",
                Some(opts.clone()),
                tokio_util::sync::CancellationToken::new(),
            )
            .collect()
            .await;
        let found = mock.status("default").await.expect("status");
        assert!(found.running);
        assert_eq!(found.cpus, 6);
        assert_eq!(found.memory, bytesize::ByteSize(12 * (1 << 30)));
        assert_eq!(found.disk, bytesize::ByteSize(80 * (1 << 30)));
        assert_eq!(found.rosetta, Some(true));
        assert_eq!(
            mock.calls(),
            vec![MockColimaCall::Start {
                profile: "default".to_string(),
                opts: Some(opts)
            }]
        );
    }

    #[tokio::test]
    async fn mock_colima_a_queued_failure_prints_stderr_exits_one_and_changes_nothing() {
        let mock = MockColima::new(vec![profile("default", false)]);
        mock.fail_next("disk image is locked");
        let items: Vec<_> = mock
            .start("default", None, tokio_util::sync::CancellationToken::new())
            .collect()
            .await;
        let lines: Vec<_> = items
            .into_iter()
            .map(|i| i.expect("no stream error"))
            .collect();
        assert_eq!(
            lines,
            vec![
                berth_sys::ProcessLine::Stderr("disk image is locked".to_string()),
                berth_sys::ProcessLine::Exit(Some(1)),
            ]
        );
        assert!(!mock.status("default").await.expect("status").running);
        // Only the next command fails.
        let items: Vec<_> = mock
            .start("default", None, tokio_util::sync::CancellationToken::new())
            .collect()
            .await;
        assert_eq!(
            items.last().and_then(|i| i.as_ref().ok()),
            Some(&berth_sys::ProcessLine::Exit(Some(0)))
        );
        assert!(mock.status("default").await.expect("status").running);
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
