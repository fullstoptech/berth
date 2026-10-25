//! Volume work (P4-T4): create one volume. Deleting goes through
//! `remove.rs`.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use berth_core::Subject;

use super::{edit, ActionsService, Inner, Job, Work};
use crate::state::{AppState, PendingKey};

struct VolumeWork {
    client: Arc<dyn berth_engine::EngineClient>,
    op: crate::services::volumes::VolumeOp,
}

#[async_trait::async_trait]
impl Work for VolumeWork {
    async fn execute(
        &self,
        _inner: &Inner,
        _job: &Job,
        cancel: CancellationToken,
    ) -> Result<edit::Done, String> {
        crate::services::volumes::run(self.client.as_ref(), self.op.clone(), &cancel).await
    }
}

impl ActionsService {
    /// Create one volume (P4-T4). Deleting goes through [`remove`]. `Err`
    /// carries the disabled reason; nothing is set pending in that case.
    ///
    /// [`remove`]: ActionsService::remove
    pub fn volume(
        &self,
        state: &mut AppState,
        engine: &berth_core::EngineId,
        name: &str,
        op: crate::services::volumes::VolumeOp,
    ) -> Result<(), String> {
        let key = PendingKey {
            subject: Subject::Volume(name.to_string()),
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
                work: Box::new(VolumeWork { client, op }),
            },
        );
        Ok(())
    }
}
