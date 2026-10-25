//! Services own every I/O task and write signals; the UI never calls an
//! engine (D23). One file per service (PLAN §6.11).

pub mod actions;
pub mod activity;
pub mod colima;
pub mod compose;
pub mod confirm;
pub mod engines;
pub mod search;

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
