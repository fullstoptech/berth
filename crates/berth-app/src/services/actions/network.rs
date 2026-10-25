//! Network work (P4-T5): create, delete, connect, disconnect.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use berth_core::Subject;

use super::{edit, ActionsService, Inner, Job, Work};
use crate::services::networks;
use crate::state::{AppState, PendingKey};

struct NetworkWork {
    client: Arc<dyn berth_engine::EngineClient>,
    op: networks::NetOp,
}

#[async_trait::async_trait]
impl Work for NetworkWork {
    async fn execute(
        &self,
        _inner: &Inner,
        job: &Job,
        cancel: CancellationToken,
    ) -> Result<edit::Done, String> {
        networks::run_net(self.client.as_ref(), &job.name, self.op.clone(), &cancel).await
    }
}

impl ActionsService {
    /// One network action (P4-T5): create, delete, connect, disconnect.
    /// `id` is the network's id (its name for a create, which has none yet)
    /// and keys the pending state; `name` is what toasts and the activity
    /// entry call it. Same contract as [`container`]: `Err` carries the
    /// reason and nothing is set pending.
    ///
    /// [`container`]: ActionsService::container
    pub fn network(
        &self,
        state: &mut AppState,
        engine: &berth_core::EngineId,
        id: &str,
        name: &str,
        op: networks::NetOp,
    ) -> Result<(), String> {
        let key = PendingKey {
            subject: Subject::Network(id.to_string()),
            action: op.action_id(),
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
                work: Box::new(NetworkWork { client, op }),
            },
        );
        Ok(())
    }
}
