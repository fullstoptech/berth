//! Work on a folder that may have no `Project` yet (P5-T3): checking the compose text a plan
//! would write, and running the command a plan asks for once its files are written.

use dioxus::prelude::{ReadableExt, WritableExt};

use berth_core::Subject;

use super::{ActionsService, Job};
use crate::services::compose;
use crate::services::plans::{action_id_for, PlanRun};
use crate::state::{AppState, PendingKey, PendingState};

impl ActionsService {
    /// Check a compose text with `docker compose config` before Write is offered
    /// (`ComposeRunner::validate_text`, AD33). `env` are the values the plan's new `.env` gives.
    ///
    /// # Errors
    /// What Compose said, as a sentence.
    pub async fn validate_compose_text(
        &self,
        folder: &std::path::Path,
        text: &str,
        env: &[(String, String)],
    ) -> Result<(), String> {
        self.inner
            .compose
            .validate_text(folder, text, env)
            .await
            .map_err(|e| e.to_string())
    }

    /// Run `command` for the folder of a written plan as a project action: the pending state is
    /// set at once (the project row shows "Starting…"), the outcome lands in the activity log and,
    /// on failure, the Compose output is kept for the Problems detail.
    ///
    /// # Errors
    /// A sentence when the engine is not connected or the same action is already running.
    pub fn run_for_folder(
        &self,
        state: &mut AppState,
        run: &PlanRun,
        command: berth_core::ComposeCommand,
    ) -> Result<PendingKey, String> {
        let key = PendingKey {
            subject: Subject::Project(run.project.clone()),
            action: action_id_for(&command),
        };
        if matches!(
            state.pending.peek().get(&key),
            Some(PendingState::Running { .. })
        ) {
            return Err(format!("{} is already running", run.name));
        }
        // A failure from before is replaced by this run.
        state.pending.write().remove(&key);
        let client = self.client_for(&run.engine)?;
        let target = compose::target_for_folder(
            &run.folder,
            &run.engine,
            client.docker_host(),
            &run.files,
            Some(&run.compose_name),
        );
        self.launch(
            state,
            Job {
                key: key.clone(),
                engine: run.engine.clone(),
                name: run.name.clone(),
                project: Some(run.project.clone()),
                work: Box::new(super::project::ComposeWork { target, command }),
            },
        );
        Ok(key)
    }
}
