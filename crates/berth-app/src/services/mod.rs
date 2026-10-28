//! Services own every I/O task and write signals; the UI never calls an
//! engine (D23). One file per service (PLAN §6.11).

pub mod actions;
pub mod activity;
pub mod assistant;
pub mod assistant_entry;
pub mod backing;
pub mod colima;
pub mod compose;
pub mod confirm;
pub mod detail;
pub mod drops;
pub mod edit;
pub mod engine_dashboard;
pub mod engines;
pub mod exec;
pub mod files;
pub mod find_images;
pub mod first_run;
pub mod folders;
pub mod git_deploy;
pub mod group_ops;
pub mod images;
pub mod login_path;
pub mod logs;
pub mod menubar;
pub mod networks;
pub mod plans;
pub mod ports;
pub mod problems;
pub mod project_files;
pub mod prune;
pub mod registries;
pub mod search;
pub mod settings_save;
pub mod stats;
pub mod templates;
pub mod timing;
pub mod tools;
pub mod updates;
pub mod visibility;
pub mod volume_backup;
pub mod volumes;

/// The moment the bundled mock world is written for: its container start
/// times give the uptimes the frames show (2 h, 6 h, 35 min, 1 d, 3 d …)
/// when read at this instant.
pub fn mock_reference_now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_792_850_550, 0).unwrap_or_default()
}

/// The home the bundled fixture is written for.
const MOCK_HOME: &str = "/Users/berth";

/// The bundled mock world with every start time moved by the distance from
/// [`mock_reference_now`] to `now`, so uptimes read as the frames show them
/// whenever the mock runs (`BERTH_MOCK=1`, scenes). Its folders sit under
/// this Mac's home, so `~/...` paths read the same on any machine.
pub fn mock_world_at(now: chrono::DateTime<chrono::Utc>) -> berth_engine::MockWorld {
    mock_world_in(now, &berth_sys::paths::home())
}

/// [`mock_world_at`] with the home given, so a test can inject one.
pub fn mock_world_in(
    now: chrono::DateTime<chrono::Utc>,
    home: &std::path::Path,
) -> berth_engine::MockWorld {
    // JSON-escape the home, then drop the quotes `to_string` adds.
    let home_json = serde_json::to_string(&home.display().to_string()).unwrap_or_default();
    let home_json = home_json.trim_matches('"');
    let json = berth_engine::mock::BUNDLED_JSON.replace(MOCK_HOME, home_json);
    let mut world = berth_engine::MockWorld::from_json(&json).unwrap_or_default();
    let shift = now - mock_reference_now();
    for container in &mut world.containers {
        container.started = container.started.map(|started| started + shift);
    }
    world
}

#[cfg(test)]
mod mock_home_tests {
    use super::*;

    #[test]
    fn mock_world_folders_move_under_the_given_home() {
        let world = mock_world_in(mock_reference_now(), std::path::Path::new("/Users/someone"));
        let folders: Vec<_> = world
            .projects
            .iter()
            .filter_map(|p| p.folder.as_deref())
            .collect();
        // One folder (/srv/landing) is outside any home on purpose.
        assert!(folders
            .iter()
            .any(|f| f.starts_with("/Users/someone/Dev/clients/acme/shop-api")));
        assert!(
            folders.iter().all(|f| !f.starts_with("/Users/berth")),
            "{folders:?}"
        );
        assert!(world
            .pins
            .iter()
            .all(|p| p.folder.starts_with("/Users/someone")));
    }
}

/// Everything a later service or screen needs from the substrate P2-T1
/// wires up: the store, the live client map and the root cancellation
/// token. Provided as Dioxus context by the App root.
#[derive(Clone)]
pub struct Services {
    pub store: std::sync::Arc<berth_store::Store>,
    pub clients: engines::ClientMap,
    pub cancel: tokio_util::sync::CancellationToken,
}

impl Services {
    /// The client for one engine, once discovery has connected it.
    pub fn client(
        &self,
        engine: &berth_core::EngineId,
    ) -> Option<std::sync::Arc<dyn berth_engine::EngineClient>> {
        self.clients
            .lock()
            .ok()
            .and_then(|map| map.get(engine).cloned())
    }
}

/// Runners for [`actions::ActionsService`]. `start` needs them
/// synchronously inside `use_hook`, but the real ones wait on the login
/// shell's PATH (up to 3 s), which must not stall the UI thread. The real
/// path therefore resolves on first use; `mock` (`BERTH_MOCK=1`) is
/// immediate.
pub fn action_runners(
    mock: bool,
) -> (
    std::sync::Arc<dyn berth_engine::ComposeRunner>,
    std::sync::Arc<dyn berth_engine::ColimaControl>,
) {
    if mock {
        (
            std::sync::Arc::new(berth_engine::MockCompose::bundled()),
            std::sync::Arc::new(berth_engine::MockColima::bundled()),
        )
    } else {
        (
            std::sync::Arc::new(lazy::LazyCompose::default()),
            std::sync::Arc::new(lazy::LazyColima::default()),
        )
    }
}

mod lazy {
    use std::sync::Arc;

    use futures::stream::{BoxStream, StreamExt};
    use tokio::sync::OnceCell;

    use berth_engine::{ColimaControl, ComposeRunner, EngineError};

    #[derive(Default)]
    pub struct LazyCompose {
        inner: Arc<OnceCell<Arc<dyn ComposeRunner>>>,
    }

    impl LazyCompose {
        async fn resolve(cell: &OnceCell<Arc<dyn ComposeRunner>>) -> &Arc<dyn ComposeRunner> {
            cell.get_or_init(|| super::compose::runner(false)).await
        }
    }

    #[async_trait::async_trait]
    impl ComposeRunner for LazyCompose {
        fn run(
            &self,
            target: &berth_core::ComposeTarget,
            cmd: berth_core::ComposeCommand,
            cancel: tokio_util::sync::CancellationToken,
        ) -> BoxStream<'static, Result<berth_core::ComposeEvent, EngineError>> {
            let cell = Arc::clone(&self.inner);
            let target = target.clone();
            futures::stream::once(
                async move { Self::resolve(&cell).await.run(&target, cmd, cancel) },
            )
            .flatten()
            .boxed()
        }

        async fn config(
            &self,
            target: &berth_core::ComposeTarget,
        ) -> Result<serde_json::Value, EngineError> {
            Self::resolve(&self.inner).await.config(target).await
        }

        async fn validate(
            &self,
            folder: &std::path::Path,
            files: &[std::path::PathBuf],
        ) -> Result<(), EngineError> {
            Self::resolve(&self.inner)
                .await
                .validate(folder, files)
                .await
        }

        async fn validate_text(
            &self,
            folder: &std::path::Path,
            compose_text: &str,
            env: &[(String, String)],
        ) -> Result<(), EngineError> {
            Self::resolve(&self.inner)
                .await
                .validate_text(folder, compose_text, env)
                .await
        }
    }

    #[derive(Default)]
    pub struct LazyColima {
        inner: Arc<OnceCell<Arc<dyn ColimaControl>>>,
    }

    impl LazyColima {
        async fn resolve(cell: &OnceCell<Arc<dyn ColimaControl>>) -> &Arc<dyn ColimaControl> {
            cell.get_or_init(|| super::colima::control(false)).await
        }
    }

    #[async_trait::async_trait]
    impl ColimaControl for LazyColima {
        /// Unknown until the first call resolves the real control.
        fn binary(&self) -> Option<std::path::PathBuf> {
            self.inner.get().and_then(|control| control.binary())
        }

        async fn profiles(&self) -> Result<Vec<berth_core::ColimaProfile>, EngineError> {
            Self::resolve(&self.inner).await.profiles().await
        }

        async fn status(&self, profile: &str) -> Result<berth_core::ColimaProfile, EngineError> {
            Self::resolve(&self.inner).await.status(profile).await
        }

        fn start(
            &self,
            profile: &str,
            opts: Option<berth_core::ColimaStartOptions>,
            cancel: tokio_util::sync::CancellationToken,
        ) -> BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
            let cell = Arc::clone(&self.inner);
            let profile = profile.to_string();
            futures::stream::once(async move {
                Self::resolve(&cell).await.start(&profile, opts, cancel)
            })
            .flatten()
            .boxed()
        }

        fn stop(
            &self,
            profile: &str,
            cancel: tokio_util::sync::CancellationToken,
        ) -> BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
            let cell = Arc::clone(&self.inner);
            let profile = profile.to_string();
            futures::stream::once(async move { Self::resolve(&cell).await.stop(&profile, cancel) })
                .flatten()
                .boxed()
        }

        fn restart(
            &self,
            profile: &str,
            cancel: tokio_util::sync::CancellationToken,
        ) -> BoxStream<'static, Result<berth_sys::ProcessLine, EngineError>> {
            let cell = Arc::clone(&self.inner);
            let profile = profile.to_string();
            futures::stream::once(
                async move { Self::resolve(&cell).await.restart(&profile, cancel) },
            )
            .flatten()
            .boxed()
        }
    }
}
