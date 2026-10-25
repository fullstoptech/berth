//! Project work: one Compose command for a project.

use tokio_util::sync::CancellationToken;

use berth_core::Subject;

use super::{edit, ActionsService, Inner, Job, ProjectOp, Work};
use crate::services::compose;
use crate::state::{AppState, PendingKey};

/// A Compose command run through the `ComposeRunner`.
struct ComposeWork {
    target: berth_core::ComposeTarget,
    command: berth_core::ComposeCommand,
}

#[async_trait::async_trait]
impl Work for ComposeWork {
    async fn execute(
        &self,
        inner: &Inner,
        _job: &Job,
        cancel: CancellationToken,
    ) -> Result<edit::Done, String> {
        compose::run(
            inner.compose.as_ref(),
            &self.target,
            self.command.clone(),
            cancel,
        )
        .await
        .map(|()| edit::Done::default())
    }
}

impl ActionsService {
    /// One project action through Compose.
    pub fn project(
        &self,
        state: &mut AppState,
        project: &berth_core::Project,
        op: ProjectOp,
    ) -> Result<(), String> {
        let key = PendingKey {
            subject: Subject::Project(project.id.clone()),
            action: op.action_id(),
        };
        Self::check(state, &key.subject, key.action)?;
        let client = self.client_for(&project.engine)?;
        let target = compose::target_for(project, client.docker_host())?;
        self.launch(
            state,
            Job {
                key,
                engine: project.engine.clone(),
                name: project.name.clone(),
                project: Some(project.id.clone()),
                work: Box::new(ComposeWork {
                    target,
                    command: op.command(),
                }),
            },
        );
        Ok(())
    }
}
