//! Remove work (`EngineAction::Remove`, D25a): delete one image, volume or
//! network.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::{edit, ActionsService, Inner, Job, Work};
use crate::state::{AppState, PendingKey};

struct RemoveWork {
    client: Arc<dyn berth_engine::EngineClient>,
    target: berth_core::ResourceRef,
    force: bool,
}

#[async_trait::async_trait]
impl Work for RemoveWork {
    async fn execute(
        &self,
        _inner: &Inner,
        _job: &Job,
        cancel: CancellationToken,
    ) -> Result<edit::Done, String> {
        // The confirm dialog says this result itself, from its summary.
        edit::guard(
            &cancel,
            berth_engine::remove_resource(self.client.as_ref(), &self.target, self.force),
        )
        .await
        .map(|()| edit::Done::default())
    }
}

impl ActionsService {
    /// Delete one image, volume or network (`EngineAction::Remove`, D25a).
    /// The pending key's subject and action come from `target`; `name` is
    /// what the activity entry calls it. Same contract as [`container`]:
    /// `Err` carries the reason (an image in use, a network with containers
    /// on it, a built-in network) and nothing is set pending.
    ///
    /// [`container`]: ActionsService::container
    pub fn remove(
        &self,
        state: &mut AppState,
        engine: &berth_core::EngineId,
        target: &berth_core::ResourceRef,
        name: &str,
        force: bool,
    ) -> Result<(), String> {
        let key = PendingKey {
            subject: target.subject(),
            action: target.action_id(),
        };
        Self::check(state, &key.subject, key.action)?;
        let client = self.client_for(engine)?;
        self.launch(
            state,
            Job {
                key,
                engine: engine.clone(),
                name: name.to_string(),
                project: None,
                work: Box::new(RemoveWork {
                    client,
                    target: target.clone(),
                    force,
                }),
            },
        );
        Ok(())
    }
}
