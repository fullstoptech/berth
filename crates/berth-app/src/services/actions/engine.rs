//! Engine work: one Colima command (start, stop, restart, apply) and the
//! bookkeeping around it: the transient Starting/Stopping row, the output
//! of the run and the reconnect once Colima has started the engine again.

use dioxus::prelude::WritableExt;
use tokio_util::sync::CancellationToken;

use berth_core::Subject;

use super::{edit, ActionsService, Inner, Job, Work};
use crate::services::colima;
use crate::state::{AppState, PendingKey};

struct ColimaWork {
    engine: Box<berth_core::Engine>,
    profile: String,
    op: berth_core::ColimaOp,
}

#[async_trait::async_trait]
impl Work for ColimaWork {
    fn begin(&self, inner: &Inner, state: &mut AppState, job: &Job) {
        // The row says what is happening; a failure puts back what was.
        let transient = if matches!(self.op, berth_core::ColimaOp::Stop) {
            berth_core::EngineState::Stopping
        } else {
            berth_core::EngineState::Starting
        };
        if let Some(found) = state
            .engines
            .write()
            .iter_mut()
            .find(|e| e.id == job.engine)
        {
            let before = std::mem::replace(&mut found.state, transient);
            match inner.previous.lock() {
                Ok(mut map) => map.insert(job.engine.clone(), before),
                Err(poisoned) => poisoned.into_inner().insert(job.engine.clone(), before),
            };
        }
    }

    async fn execute(
        &self,
        inner: &Inner,
        job: &Job,
        cancel: CancellationToken,
    ) -> Result<edit::Done, String> {
        inner.runs.begin(&job.engine, job.key.action);
        let runs = inner.runs.clone();
        let engine = job.engine.clone();
        let sink = move |line: &berth_sys::ProcessLine| runs.push(&engine, line);
        colima::run_with(
            inner.colima.as_ref(),
            &self.profile,
            self.op.clone(),
            cancel,
            &sink,
        )
        .await
        .map(|()| edit::Done::default())
    }

    async fn finished(&self, inner: &Inner, job: &Job, executed: &Result<edit::Done, String>) {
        inner.runs.finish(
            &job.engine,
            &executed.as_ref().map(|_| ()).map_err(Clone::clone),
        );
        // Colima started the engine again: make sure Berth has a
        // client and an events loop for it.
        if executed.is_ok() && !matches!(self.op, berth_core::ColimaOp::Stop) {
            if let Some(link) = inner.link.get() {
                let status = inner.colima.status(&self.profile).await.ok();
                link.connect((*self.engine).clone(), status);
            }
        }
    }
}

impl ActionsService {
    /// After a Colima command: a success has set the final state already
    /// ([`apply_finished`]); a failure puts the engine back as it was.
    pub fn settle_engine(&self, state: &mut AppState, key: &PendingKey, ok: bool) {
        let Subject::Engine(id) = &key.subject else {
            return;
        };
        let before = match self.inner.previous.lock() {
            Ok(mut map) => map.remove(id),
            Err(poisoned) => poisoned.into_inner().remove(id),
        };
        if let (false, Some(before)) = (ok, before) {
            if let Some(found) = state.engines.write().iter_mut().find(|e| e.id == *id) {
                found.state = before;
            }
        }
    }

    /// Let this service reconnect engines Colima has started (once).
    pub fn set_link(&self, link: crate::services::engines::EngineLink) {
        let _ = self.inner.link.set(link);
    }

    /// The Colima run of an engine as the dashboard shows it: the output
    /// so far while it runs, the output and the reason after it failed.
    #[must_use]
    pub fn engine_run(&self, engine: &berth_core::EngineId) -> Option<colima::RunView> {
        self.inner.runs.view(engine)
    }

    /// Forget a failed run (the user dismissed it).
    pub fn clear_engine_run(&self, engine: &berth_core::EngineId) {
        self.inner.runs.clear(engine);
    }

    /// One Colima operation on an engine (start, stop, restart, apply).
    pub fn engine(
        &self,
        state: &mut AppState,
        engine: &berth_core::Engine,
        op: berth_core::ColimaOp,
    ) -> Result<(), String> {
        let key = PendingKey {
            subject: Subject::Engine(engine.id.clone()),
            action: colima::action_id(&op),
        };
        Self::check(state, &key.subject, key.action)?;
        let Some(profile) = colima::profile_of(engine) else {
            return Err("Only Colima engines can be started or stopped here".to_string());
        };
        self.launch(
            state,
            Job {
                key,
                engine: engine.id.clone(),
                name: engine.name.clone(),
                project: None,
                work: Box::new(ColimaWork {
                    engine: Box::new(engine.clone()),
                    profile: profile.to_string(),
                    op,
                }),
            },
        );
        Ok(())
    }
}
