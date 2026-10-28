//! Build, push, export and import (P8-T3, AD43): the I/O behind the Images
//! screen's build sheet and the image inspector's Push and Export buttons.
//!
//! A build or a push streams events into a state the sheet reads
//! ([`BuildState`], [`PushState`]; pure, folded one event at a time like
//! [`super::PullState`]). Every operation takes a child of the services'
//! `CancellationToken`, shows as a pending entry on `Subject::Image` while
//! it runs, writes one Activity entry when it ends and re-lists the engines'
//! images. A [`berth_core::BuildSpec`] is never logged or kept in state:
//! build arguments can be secrets.

use std::path::{Path, PathBuf};

use dioxus::prelude::*;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use berth_core::{BuildContext, BuildEvent, BuildSpec, EngineId, Outcome, PullProgress, Subject};
use berth_engine::client::Bs;

use super::{check_reference, credentials_for_reference, io, with_default_tag, ImageStore};
use crate::services::actions::spawn_io;
use crate::services::{activity, Services};
use crate::state::{AppState, PendingKey, PendingState};

/// Action ids: the Activity entry's and the pending key's.
pub const BUILD_ACTION: &str = "image.build";
pub const PUSH_ACTION: &str = "image.push";
pub const EXPORT_ACTION: &str = "image.export";
pub const IMPORT_ACTION: &str = "image.import";

/// A build is addressed by its engine and first tag.
pub type BuildKey = (EngineId, String);
/// A push is addressed by its engine and full reference (`:latest` added).
pub type PushKey = (EngineId, String);
/// What a running operation's token is filed under: engine, action, name.
pub(super) type OpKey = (EngineId, &'static str, String);

/// How many output lines a build keeps; the oldest go first.
const LOG_CAP: usize = 5000;

// ---------------------------------------------------------------------
// Build state (pure)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildPhase {
    Running,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildLineKind {
    /// `Step 3/7 : RUN make`
    Step,
    Log,
    /// The failing step's error line.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildLine {
    pub kind: BuildLineKind,
    pub text: String,
}

/// One build as the output said so far. Holds no build arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildState {
    pub engine: EngineId,
    pub tags: Vec<String>,
    /// Where the context came from: the folder, or "Dockerfile" for one
    /// written in the editor.
    pub context: String,
    pub lines: Vec<BuildLine>,
    /// The latest `Step n/m` seen.
    pub step: Option<(u32, u32)>,
    /// The step number the failure line followed.
    pub failed_step: Option<u32>,
    /// The failing step's error line.
    pub failure: Option<String>,
    pub phase: BuildPhase,
    pub started: std::time::Instant,
}

impl BuildState {
    #[must_use]
    pub fn new(engine: EngineId, spec: &BuildSpec) -> Self {
        Self {
            engine,
            tags: spec.tags.clone(),
            context: match &spec.context {
                BuildContext::Folder(folder) => folder.display().to_string(),
                BuildContext::Inline(_) => "Dockerfile".to_string(),
            },
            lines: Vec::new(),
            step: None,
            failed_step: None,
            failure: None,
            phase: BuildPhase::Running,
            started: std::time::Instant::now(),
        }
    }

    #[must_use]
    pub fn key(&self) -> BuildKey {
        (
            self.engine.clone(),
            self.tags.first().cloned().unwrap_or_default(),
        )
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.phase == BuildPhase::Running
    }

    /// Fold one event of the build in.
    pub fn apply(&mut self, event: &BuildEvent) {
        let (kind, text) = match event {
            BuildEvent::Step { index, total, text } => {
                self.step = Some((*index, *total));
                (BuildLineKind::Step, text)
            }
            BuildEvent::Failed { text } => {
                self.failed_step = self.step.map(|(index, _)| index);
                self.failure = Some(text.clone());
                (BuildLineKind::Failed, text)
            }
            BuildEvent::Log(text) => (BuildLineKind::Log, text),
        };
        self.lines.push(BuildLine {
            kind,
            text: text.clone(),
        });
        if self.lines.len() > LOG_CAP {
            let excess = self.lines.len() - LOG_CAP;
            self.lines.drain(..excess);
        }
    }

    /// How far the build is, 0 to 1, by steps; `None` before the first.
    #[must_use]
    pub fn fraction(&self) -> Option<f32> {
        let (index, total) = self.step?;
        if total == 0 {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        let done = if self.phase == BuildPhase::Done {
            1.0
        } else {
            // Step `index` is under way, not done.
            index.saturating_sub(1) as f32 / total as f32
        };
        Some(done.clamp(0.0, 1.0))
    }
}

// ---------------------------------------------------------------------
// Push state (pure)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushPhase {
    Running,
    Done,
    Failed(String),
    Cancelled,
}

/// One push as the registry's lines said so far. Bollard's push stream
/// names no layers, so this keeps the latest line, not a per-layer table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushState {
    pub engine: EngineId,
    /// The reference pushed, tag included.
    pub reference: String,
    /// The latest status line ("Pushing", "Pushed", "The push refers to …").
    pub note: String,
    /// The digest of the pushed manifest, once the registry reported it.
    pub digest: Option<String>,
    pub phase: PushPhase,
    pub started: std::time::Instant,
}

impl PushState {
    #[must_use]
    pub fn new(engine: EngineId, reference: &str) -> Self {
        Self {
            engine,
            reference: reference.to_string(),
            note: String::new(),
            digest: None,
            phase: PushPhase::Running,
            started: std::time::Instant::now(),
        }
    }

    #[must_use]
    pub fn key(&self) -> PushKey {
        (self.engine.clone(), self.reference.clone())
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.phase == PushPhase::Running
    }

    /// Fold one progress line in.
    pub fn apply(&mut self, event: &PullProgress) {
        if event.status.is_empty() {
            return;
        }
        if let Some(digest) = digest_in(&event.status) {
            self.digest = Some(digest);
        }
        self.note.clone_from(&event.status);
    }
}

/// The `sha256:…` in a closing line such as
/// `1: digest: sha256:abc size: 527`.
fn digest_in(status: &str) -> Option<String> {
    let after = status.split_once("digest: ")?.1;
    let digest = after.split_whitespace().next()?;
    digest.starts_with("sha256:").then(|| digest.to_string())
}

// ---------------------------------------------------------------------
// Checking a build (pure apart from two file-system lookups)
// ---------------------------------------------------------------------

/// The spec with its tags trimmed and de-duplicated, or the sentence that
/// says what is wrong. Both the sheet and the assistant's `images.build`
/// come through here.
///
/// # Errors
/// A sentence for the user.
pub fn check_build_spec(mut spec: BuildSpec) -> Result<BuildSpec, String> {
    if spec.tags.is_empty() {
        return Err("Give the image a name such as myapp:dev".to_string());
    }
    let mut tags: Vec<String> = Vec::new();
    for tag in &spec.tags {
        let checked = check_reference(tag)?;
        if checked.contains('@') {
            return Err("A new image is named with a tag, not a digest".to_string());
        }
        if !tags.contains(&checked) {
            tags.push(checked);
        }
    }
    spec.tags = tags;
    if spec
        .build_args
        .iter()
        .any(|(name, _)| name.trim().is_empty())
    {
        return Err("A build argument needs a name".to_string());
    }
    match &spec.context {
        BuildContext::Inline(text) if text.trim().is_empty() => {
            return Err("The Dockerfile is empty".to_string());
        }
        BuildContext::Inline(_) => {}
        BuildContext::Folder(folder) => {
            if !folder.is_dir() {
                return Err(format!("{} is not a folder", folder.display()));
            }
            let name = spec.dockerfile.as_deref().unwrap_or("Dockerfile");
            if !folder.join(name).is_file() {
                return Err(format!("There is no {name} in {}", folder.display()));
            }
        }
    }
    Ok(spec)
}

// ---------------------------------------------------------------------
// The streams
// ---------------------------------------------------------------------

enum Msg<T> {
    Event(T),
    Done,
    Failed(String),
    Cancelled,
}

/// Run a stream on the I/O runtime and forward it, then one closing
/// message. The stream is created there too: the engine may spawn tasks
/// when asked for it.
fn pump<T: Send + 'static>(
    make: impl FnOnce() -> Bs<T> + Send + 'static,
    cancel: CancellationToken,
) -> tokio::sync::mpsc::UnboundedReceiver<Msg<T>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    spawn_io(async move {
        let mut stream = make();
        let mut failure = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(event) => {
                    let _ = tx.send(Msg::Event(event));
                }
                Err(err) => {
                    failure = Some(err.to_string());
                    break;
                }
            }
        }
        let end = match failure {
            Some(message) => Msg::Failed(message),
            None if cancel.is_cancelled() => Msg::Cancelled,
            None => Msg::Done,
        };
        let _ = tx.send(end);
    });
    rx
}

pub(super) fn outcome_of(result: &Result<(), String>) -> Outcome {
    match result {
        Ok(()) => Outcome::Ok,
        Err(reason) => Outcome::Failed {
            reason: reason.clone(),
        },
    }
}

impl ImageStore {
    /// The token of a new operation, filed under `(engine, action, name)`,
    /// and its pending entry on the image (D23).
    pub(super) fn begin_op(
        &self,
        services: &Services,
        engine: &EngineId,
        action: &'static str,
        name: &str,
        label: String,
    ) -> CancellationToken {
        let cancel = services.cancel.child_token();
        let mut cancels = self.op_cancels;
        cancels
            .write()
            .insert((engine.clone(), action, name.to_string()), cancel.clone());
        if let Some(mut app) = try_consume_context::<AppState>() {
            app.pending.write().insert(
                PendingKey {
                    subject: Subject::Image(name.to_string()),
                    action,
                },
                PendingState::Running { label },
            );
        }
        cancel
    }

    /// Forget the token and the pending entry of a finished operation.
    /// `app` is the `AppState` captured when it began (the spawned task has
    /// no context of its own).
    pub(super) fn end_op(
        &self,
        app: Option<AppState>,
        engine: &EngineId,
        action: &'static str,
        name: &str,
    ) {
        let mut cancels = self.op_cancels;
        cancels
            .write()
            .remove(&(engine.clone(), action, name.to_string()));
        if let Some(mut app) = app {
            app.pending.write().remove(&PendingKey {
                subject: Subject::Image(name.to_string()),
                action,
            });
        }
    }

    pub(super) fn record(
        services: &Services,
        engine: &EngineId,
        action: &str,
        name: &str,
        outcome: Outcome,
    ) {
        activity::record(
            &services.store,
            &activity::entry_for_action(
                engine.clone(),
                action,
                Subject::Image(name.to_string()),
                name.to_string(),
                None,
                outcome,
            ),
        );
    }

    // -- build ------------------------------------------------------------

    /// Start building `spec` on `engine`. The first tag names the build;
    /// one that is running under the same name is left alone. Registry
    /// credentials are the first tag's registry's (the one the image is
    /// likely pushed to).
    ///
    /// # Errors
    /// The sentence [`check_build_spec`] gives, or that the engine is not
    /// connected.
    pub fn build(
        &self,
        services: &Services,
        engine: &EngineId,
        spec: BuildSpec,
    ) -> Result<(), String> {
        let spec = check_build_spec(spec)?;
        let name = spec.tags[0].clone();
        let key = (engine.clone(), name.clone());
        if self
            .builds
            .peek()
            .iter()
            .any(|b| b.key() == key && b.is_running())
        {
            return Ok(());
        }
        let client = services
            .client(engine)
            .ok_or_else(|| format!("Engine {} is not connected", engine.0))?;
        let mut builds = self.builds;
        {
            let mut list = builds.write();
            list.retain(|b| b.key() != key);
            list.push(BuildState::new(engine.clone(), &spec));
        }
        let cancel = self.begin_op(
            services,
            engine,
            BUILD_ACTION,
            &name,
            format!("Building {name}"),
        );
        let credentials = credentials_for_reference(&name);
        let stream_cancel = cancel.clone();
        let mut rx = pump(
            move || client.build_image(&spec, credentials.as_ref(), stream_cancel),
            cancel,
        );
        let app = try_consume_context::<AppState>();
        let store = *self;
        let services = services.clone();
        let engine = engine.clone();
        crate::services::actions::spawn_job(async move {
            while let Some(message) = rx.recv().await {
                let (phase, outcome) = match message {
                    Msg::Event(event) => {
                        if let Some(state) = builds.write().iter_mut().find(|b| b.key() == key) {
                            state.apply(&event);
                        }
                        continue;
                    }
                    Msg::Done => (BuildPhase::Done, Some(Outcome::Ok)),
                    Msg::Failed(reason) => (
                        BuildPhase::Failed(reason.clone()),
                        Some(Outcome::Failed { reason }),
                    ),
                    Msg::Cancelled => (BuildPhase::Cancelled, None),
                };
                if let Some(state) = builds.write().iter_mut().find(|b| b.key() == key) {
                    state.phase = phase;
                }
                store.end_op(app, &engine, BUILD_ACTION, &name);
                if let Some(outcome) = outcome {
                    Self::record(&services, &engine, BUILD_ACTION, &name, outcome);
                }
                // The new image is listed once the build has tagged it.
                store.refresh(&services);
                break;
            }
        });
        Ok(())
    }

    /// Stop a running build.
    pub fn cancel_build(&self, key: &BuildKey) {
        self.cancel_op(&key.0, BUILD_ACTION, &key.1);
    }

    /// Drop a finished build from the list.
    pub fn dismiss_build(&self, key: &BuildKey) {
        let mut builds = self.builds;
        builds.write().retain(|b| b.key() != *key || b.is_running());
    }

    // -- push -------------------------------------------------------------

    /// Start pushing `reference` (`:latest` when it has no tag) from
    /// `engine`, with the credentials of the logged-in registry that serves
    /// it. A push of the same reference that is running is left alone.
    ///
    /// # Errors
    /// The sentence [`check_reference`] gives, or that the engine is not
    /// connected.
    pub fn push(
        &self,
        services: &Services,
        engine: &EngineId,
        reference: &str,
    ) -> Result<(), String> {
        let reference = with_default_tag(&check_reference(reference)?);
        let key = (engine.clone(), reference.clone());
        if self
            .pushes
            .peek()
            .iter()
            .any(|p| p.key() == key && p.is_running())
        {
            return Ok(());
        }
        let client = services
            .client(engine)
            .ok_or_else(|| format!("Engine {} is not connected", engine.0))?;
        let mut pushes = self.pushes;
        {
            let mut list = pushes.write();
            list.retain(|p| p.key() != key);
            list.push(PushState::new(engine.clone(), &reference));
        }
        let cancel = self.begin_op(
            services,
            engine,
            PUSH_ACTION,
            &reference,
            format!("Pushing {reference}"),
        );
        let credentials = credentials_for_reference(&reference);
        let stream_cancel = cancel.clone();
        let wanted = reference.clone();
        let mut rx = pump(
            move || client.push(&wanted, credentials.as_ref(), stream_cancel),
            cancel,
        );
        let app = try_consume_context::<AppState>();
        let store = *self;
        let services = services.clone();
        let engine = engine.clone();
        crate::services::actions::spawn_job(async move {
            while let Some(message) = rx.recv().await {
                let (phase, outcome) = match message {
                    Msg::Event(event) => {
                        if let Some(state) = pushes.write().iter_mut().find(|p| p.key() == key) {
                            state.apply(&event);
                        }
                        continue;
                    }
                    Msg::Done => (PushPhase::Done, Some(Outcome::Ok)),
                    Msg::Failed(reason) => (
                        PushPhase::Failed(reason.clone()),
                        Some(Outcome::Failed { reason }),
                    ),
                    Msg::Cancelled => (PushPhase::Cancelled, None),
                };
                if let Some(state) = pushes.write().iter_mut().find(|p| p.key() == key) {
                    state.phase = phase;
                }
                store.end_op(app, &engine, PUSH_ACTION, &reference);
                if let Some(outcome) = outcome {
                    Self::record(&services, &engine, PUSH_ACTION, &reference, outcome);
                }
                break;
            }
        });
        Ok(())
    }

    /// Stop a running push.
    pub fn cancel_push(&self, key: &PushKey) {
        self.cancel_op(&key.0, PUSH_ACTION, &key.1);
    }

    /// Drop a finished push from the list.
    pub fn dismiss_push(&self, key: &PushKey) {
        let mut pushes = self.pushes;
        pushes.write().retain(|p| p.key() != *key || p.is_running());
    }

    // -- export and import -------------------------------------------------

    /// Save `image` (a reference or an id) as a tar at `path`, the file a
    /// user chose in a save panel (so not a Review changes write, AD15). The
    /// tar is written beside the target as `<name>.part` and renamed when
    /// complete; a failed or cancelled export leaves nothing. `done` hears
    /// the size in bytes, or the reason (`"Cancelled"` after
    /// [`Self::cancel_transfer`]).
    pub fn export(
        &self,
        services: &Services,
        engine: &EngineId,
        image: &str,
        path: PathBuf,
        done: impl FnOnce(Result<u64, String>) + 'static,
    ) {
        let Some(client) = services.client(engine) else {
            done(Err(format!("Engine {} is not connected", engine.0)));
            return;
        };
        let cancel = self.begin_op(
            services,
            engine,
            EXPORT_ACTION,
            image,
            format!("Exporting {image}"),
        );
        let app = try_consume_context::<AppState>();
        let store = *self;
        let services = services.clone();
        let engine = engine.clone();
        let name = image.to_string();
        let wanted = image.to_string();
        let work_cancel = cancel.clone();
        io(
            async move { write_export(client.as_ref(), &wanted, &path, &work_cancel).await },
            move |result| {
                store.end_op(app, &engine, EXPORT_ACTION, &name);
                if !cancel.is_cancelled() {
                    Self::record(
                        &services,
                        &engine,
                        EXPORT_ACTION,
                        &name,
                        outcome_of(&result.clone().map(|_| ())),
                    );
                }
                done(result);
            },
        );
    }

    /// Load the tar at `path` (an open panel's choice) into `engine`. `done`
    /// hears the names the engine reports ("Loaded image: …"), or the
    /// reason. The image lists are read again afterwards.
    pub fn import(
        &self,
        services: &Services,
        engine: &EngineId,
        path: PathBuf,
        done: impl FnOnce(Result<Vec<String>, String>) + 'static,
    ) {
        let Some(client) = services.client(engine) else {
            done(Err(format!("Engine {} is not connected", engine.0)));
            return;
        };
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().to_string(),
        );
        let cancel = self.begin_op(
            services,
            engine,
            IMPORT_ACTION,
            &name,
            format!("Importing {name}"),
        );
        let app = try_consume_context::<AppState>();
        let store = *self;
        let services = services.clone();
        let engine = engine.clone();
        let work_cancel = cancel.clone();
        io(
            async move {
                let tar = tokio::fs::read(&path)
                    .await
                    .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
                tokio::select! {
                    () = work_cancel.cancelled() => Err("Cancelled".to_string()),
                    loaded = client.import_image(bytes::Bytes::from(tar)) => {
                        loaded.map_err(|e| e.to_string())
                    }
                }
            },
            move |result| {
                store.end_op(app, &engine, IMPORT_ACTION, &name);
                if !cancel.is_cancelled() {
                    let subject = result
                        .as_ref()
                        .ok()
                        .and_then(|names| names.first().cloned())
                        .unwrap_or_else(|| name.clone());
                    Self::record(
                        &services,
                        &engine,
                        IMPORT_ACTION,
                        &subject,
                        outcome_of(&result.clone().map(|_| ())),
                    );
                }
                if result.is_ok() {
                    store.refresh(&services);
                }
                done(result);
            },
        );
    }

    /// Stop a running export (`name` is the image as given to
    /// [`Self::export`]) or import (the file's name).
    pub fn cancel_transfer(&self, engine: &EngineId, action: &'static str, name: &str) {
        self.cancel_op(engine, action, name);
    }

    fn cancel_op(&self, engine: &EngineId, action: &'static str, name: &str) {
        if let Some(token) = self
            .op_cancels
            .peek()
            .get(&(engine.clone(), action, name.to_string()))
        {
            token.cancel();
        }
    }
}

/// Stream an image's tar into `<path>.part`, then rename it to `path`.
async fn write_export(
    client: &dyn berth_engine::EngineClient,
    image: &str,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<u64, String> {
    use tokio::io::AsyncWriteExt as _;
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    let mut stream = client.export_image(image, cancel.clone());
    let mut written = 0_u64;
    let result: Result<(), String> = loop {
        match stream.next().await {
            Some(Ok(chunk)) => {
                if let Err(e) = file.write_all(&chunk).await {
                    break Err(format!("Cannot write {}: {e}", path.display()));
                }
                written += chunk.len() as u64;
            }
            Some(Err(e)) => break Err(e.to_string()),
            None if cancel.is_cancelled() => break Err("Cancelled".to_string()),
            None => break file.flush().await.map_err(|e| e.to_string()),
        }
    };
    drop(file);
    match result {
        Ok(()) => tokio::fs::rename(&part, path)
            .await
            .map(|()| written)
            .map_err(|e| format!("Cannot write {}: {e}", path.display())),
        Err(reason) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(reason)
        }
    }
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;
