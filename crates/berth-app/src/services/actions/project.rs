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
        job: &Job,
        cancel: CancellationToken,
    ) -> Result<edit::Done, String> {
        let ran = compose::run(
            inner.compose.as_ref(),
            &self.target,
            self.command.clone(),
            cancel,
        )
        .await;
        if let Some(project) = &job.project {
            let mut failures = match inner.compose_failures.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            match &ran {
                Ok(()) => {
                    failures.remove(project);
                }
                Err(failure) => {
                    failures.insert(project.clone(), failure.lines.clone());
                }
            }
        }
        ran.map(|()| edit::Done::default())
            .map_err(|failure| failure.message)
    }
}

impl ActionsService {
    /// What the project's last failed Compose command printed: up to `compose::FAILURE_LINES`
    /// stderr lines and error events, oldest first (AD33). Empty when the last command succeeded or
    /// none has run. The Problems detail shows these lines.
    #[must_use]
    pub fn compose_failure_lines(&self, project: &berth_core::ProjectId) -> Vec<String> {
        let failures = match self.inner.compose_failures.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        failures.get(project).cloned().unwrap_or_default()
    }

    /// Does the project's Compose file load (`docker compose config -q`)? `Err` holds Compose's
    /// own words, which `project_files::parse_compose_problem` reads a line and a message from.
    /// Nothing starts or changes; the project detail screen asks when it opens.
    pub async fn validate_compose(
        &self,
        folder: &std::path::Path,
        files: &[std::path::PathBuf],
    ) -> Result<(), String> {
        self.inner
            .compose
            .validate(folder, files)
            .await
            .map_err(|err| err.to_string())
    }

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
