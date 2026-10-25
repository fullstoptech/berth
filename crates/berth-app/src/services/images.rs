//! The images service (P4-T1, PLAN §6.11, D23): what the Images screen,
//! its inspector panel and the Run sheet read, and the I/O behind their
//! buttons. The UI reads the store; this file owns every engine call.
//!
//! `AppState::images` is one flat list over all engines and an `Image`
//! carries no engine, so the store keeps its own per-engine lists (one
//! `client.images()` per engine, refreshed whenever the flat list changes)
//! and takes scope and owner from those. Pulls are folded into per-layer
//! progress ([`PullState`], pure) as Docker streams them.

use std::collections::{BTreeMap, HashMap, HashSet};

use dioxus::prelude::*;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use berth_core::{
    ContainerId, ContainerSpec, EngineId, Image, ImageDetail, ImageRef, Outcome, PullProgress,
    Subject,
};

use crate::services::actions::spawn_io;
use crate::services::{activity, Services};

// ---------------------------------------------------------------------
// Pull progress (pure)
// ---------------------------------------------------------------------

/// Where one layer of a pull is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerPhase {
    Waiting,
    Downloading,
    Downloaded,
    Extracting,
    Complete,
    /// The engine had it already.
    Existing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerState {
    pub id: String,
    pub phase: LayerPhase,
    pub current: u64,
    pub total: u64,
}

impl LayerState {
    /// How much of the layer's download is done, 0 to 1. A layer with no
    /// size yet counts as not started.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn fraction(&self) -> f32 {
        match self.phase {
            LayerPhase::Waiting => 0.0,
            LayerPhase::Downloading if self.total > 0 => {
                (self.current as f32 / self.total as f32).clamp(0.0, 1.0)
            }
            LayerPhase::Downloading => 0.0,
            LayerPhase::Downloaded
            | LayerPhase::Extracting
            | LayerPhase::Complete
            | LayerPhase::Existing => 1.0,
        }
    }

    /// The right-hand text of the layer's row ("Pull complete",
    /// "31.1 MB of 41.5 MB", "Waiting").
    #[must_use]
    pub fn label(&self) -> String {
        match self.phase {
            LayerPhase::Waiting => "Waiting".to_string(),
            LayerPhase::Downloading if self.total > 0 => format!(
                "{} of {}",
                berth_core::fmt::bytes(bytesize::ByteSize(self.current)),
                berth_core::fmt::bytes(bytesize::ByteSize(self.total))
            ),
            LayerPhase::Downloading => "Downloading".to_string(),
            LayerPhase::Downloaded => "Downloaded".to_string(),
            LayerPhase::Extracting => "Extracting".to_string(),
            LayerPhase::Complete => "Pull complete".to_string(),
            LayerPhase::Existing => "Already exists".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullPhase {
    Running,
    Done,
    Failed(String),
    Cancelled,
}

/// One pull: engine, reference and the layers as far as the stream said.
#[derive(Debug, Clone, PartialEq)]
pub struct PullState {
    pub engine: EngineId,
    pub reference: String,
    pub platform: Option<String>,
    pub layers: Vec<LayerState>,
    /// The last line that belongs to no layer ("Pulling from library/alpine").
    pub note: String,
    pub phase: PullPhase,
}

/// What a pull is addressed by.
pub type PullKey = (EngineId, String);

impl PullState {
    #[must_use]
    pub fn new(engine: EngineId, reference: &str, platform: Option<&str>) -> Self {
        Self {
            engine,
            reference: reference.to_string(),
            platform: platform.map(str::to_string),
            layers: Vec::new(),
            note: String::new(),
            phase: PullPhase::Running,
        }
    }

    #[must_use]
    pub fn key(&self) -> PullKey {
        (self.engine.clone(), self.reference.clone())
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.phase == PullPhase::Running
    }

    /// Fold one event of Docker's pull stream in. Events with a layer id
    /// and a layer status move that layer; the rest are the pull's note.
    pub fn apply(&mut self, event: &PullProgress) {
        let status = event.status.as_str();
        let phase = if status.starts_with("Pulling fs layer") || status.starts_with("Waiting") {
            Some(LayerPhase::Waiting)
        } else if status.starts_with("Downloading") {
            Some(LayerPhase::Downloading)
        } else if status.starts_with("Verifying Checksum")
            || status.starts_with("Download complete")
        {
            Some(LayerPhase::Downloaded)
        } else if status.starts_with("Extracting") {
            Some(LayerPhase::Extracting)
        } else if status.starts_with("Pull complete") {
            Some(LayerPhase::Complete)
        } else if status.starts_with("Already exists") {
            Some(LayerPhase::Existing)
        } else {
            None
        };
        let (Some(id), Some(phase)) = (event.layer.as_deref(), phase) else {
            if !status.is_empty() {
                self.note = status.to_string();
            }
            return;
        };
        let index = if let Some(i) = self.layers.iter().position(|l| l.id == id) {
            i
        } else {
            self.layers.push(LayerState {
                id: id.to_string(),
                phase: LayerPhase::Waiting,
                current: 0,
                total: 0,
            });
            self.layers.len() - 1
        };
        let layer = &mut self.layers[index];
        layer.phase = phase;
        if phase == LayerPhase::Downloading {
            if let Some(total) = event.total.filter(|t| *t > 0) {
                layer.total = total;
            }
            layer.current = event
                .current
                .unwrap_or(layer.current)
                .min(if layer.total > 0 {
                    layer.total
                } else {
                    u64::MAX
                });
        } else if matches!(
            phase,
            LayerPhase::Downloaded | LayerPhase::Extracting | LayerPhase::Complete
        ) {
            // A finished download is the whole layer, whatever the last
            // progress line said.
            layer.current = layer.total;
        }
    }

    /// Bytes downloaded and bytes to download, over layers whose size is
    /// known.
    #[must_use]
    pub fn bytes(&self) -> (u64, u64) {
        self.layers
            .iter()
            .filter(|l| l.total > 0)
            .fold((0, 0), |(done, total), l| {
                (done + l.current.min(l.total), total + l.total)
            })
    }

    /// Whole percent of the download; `None` until a size is known.
    #[must_use]
    pub fn percent(&self) -> Option<u8> {
        let (done, total) = self.bytes();
        if total == 0 {
            return None;
        }
        let pct = done.saturating_mul(100) / total;
        Some(u8::try_from(pct.min(100)).unwrap_or(100))
    }

    /// Layers finished (pulled, or there already) out of all seen.
    #[must_use]
    pub fn layers_done(&self) -> (usize, usize) {
        let done = self
            .layers
            .iter()
            .filter(|l| matches!(l.phase, LayerPhase::Complete | LayerPhase::Existing))
            .count();
        (done, self.layers.len())
    }

    /// "68 MB of 105 MB", or the note while no size is known.
    #[must_use]
    pub fn bytes_label(&self) -> String {
        let (done, total) = self.bytes();
        if total == 0 {
            return self.note.clone();
        }
        format!(
            "{} of {}",
            berth_core::fmt::bytes(bytesize::ByteSize(done)),
            berth_core::fmt::bytes(bytesize::ByteSize(total))
        )
    }
}

// ---------------------------------------------------------------------
// References and the Run sheet's starting point (pure)
// ---------------------------------------------------------------------

/// Whether `text` is something `docker pull` accepts: no spaces, an
/// optional registry and path of lowercase name parts, an optional `:tag`
/// or `@sha256:digest`.
///
/// # Errors
/// A sentence saying what is wrong.
pub fn check_reference(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Type an image such as alpine:3".to_string());
    }
    if text.chars().any(char::is_whitespace) {
        return Err("An image name has no spaces".to_string());
    }
    let (name, digest) = match text.split_once('@') {
        Some((name, digest)) => (name, Some(digest)),
        None => (text, None),
    };
    if let Some(digest) = digest {
        if !digest.starts_with("sha256:") || digest.len() < 15 {
            return Err("A digest looks like sha256:… followed by 64 hex digits".to_string());
        }
    }
    // A tag follows the last `/`'s name part.
    let last = name.rsplit('/').next().unwrap_or(name);
    let (path, tag) = match last.split_once(':') {
        Some((_, tag)) => (&name[..name.len() - tag.len() - 1], Some(tag)),
        None => (name, None),
    };
    if let Some(tag) = tag {
        check_tag(tag)?;
    }
    let parts: Vec<&str> = path.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        // The first part may be a registry host with a port.
        let host = index == 0 && parts.len() > 1 && (part.contains('.') || part.contains(':'));
        let ok = !part.is_empty()
            && part.chars().all(|c| {
                c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || matches!(c, '_' | '.' | '-')
                    || (host && (c == ':' || c.is_ascii_uppercase()))
            })
            && part
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        if !ok {
            return Err(format!("{part} is not a valid part of an image name"));
        }
    }
    Ok(text.to_string())
}

/// Whether `tag` is one Docker takes: up to 128 letters, digits, `_`, `.`
/// and `-`.
///
/// # Errors
/// A sentence saying what is wrong.
pub fn check_tag(tag: &str) -> Result<(), String> {
    let ok = !tag.is_empty()
        && tag.len() <= 128
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if ok {
        Ok(())
    } else {
        Err(format!("{tag} is not a valid tag"))
    }
}

/// `alpine` becomes `alpine:latest`, as `docker pull` reads it; a
/// reference with a tag or digest stays.
#[must_use]
pub fn with_default_tag(reference: &str) -> String {
    let last = reference.rsplit('/').next().unwrap_or(reference);
    if reference.contains('@') || last.contains(':') {
        reference.to_string()
    } else {
        format!("{reference}:latest")
    }
}

/// A container name for a fresh run of `repo`: its last path part, then
/// `-2`, `-3` … until nothing on the engine has it.
#[must_use]
pub fn suggest_name(repo: &str, taken: &[&str]) -> String {
    let base: String = repo
        .rsplit('/')
        .next()
        .unwrap_or(repo)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let base = if base.is_empty()
        || !base
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        "container".to_string()
    } else {
        base
    };
    if !taken.contains(&base.as_str()) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|name| !taken.contains(&name.as_str()))
        .unwrap_or(base)
}

/// What to run an image as: its first tag (or its id when it has none).
#[must_use]
pub fn run_reference(image: &Image) -> String {
    image
        .repo_tags
        .first()
        .cloned()
        .unwrap_or_else(|| image.id.clone())
}

/// A bare spec that starts `image` as it is: the sheet fills in the rest.
#[must_use]
pub fn base_spec(image: &Image, name: &str) -> ContainerSpec {
    ContainerSpec {
        name: name.to_string(),
        image: ImageRef {
            reference: run_reference(image),
            id: Some(image.id.clone()),
        },
        command: None,
        entrypoint: None,
        env: Vec::new(),
        ports: Vec::new(),
        mounts: Vec::new(),
        networks: Vec::new(),
        labels: std::collections::BTreeMap::new(),
        restart: berth_core::RestartPolicy::No,
        limits: berth_core::Limits::default(),
        platform: None,
        working_dir: None,
        user: None,
        healthcheck: None,
        cap_add: Vec::new(),
        cap_drop: Vec::new(),
        network_aliases: std::collections::BTreeMap::new(),
    }
}

// ---------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------

/// What the inspector knows about one image.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageEntry {
    Ready(Box<ImageDetail>),
    Failed { message: String, gone: bool },
}

/// The two tabs of the Images section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImagesTab {
    #[default]
    OnThisEngine,
    Find,
}

/// The "dangling" filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageFilter {
    #[default]
    All,
    Dangling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Size,
    Created,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

pub type Sort = (SortColumn, SortDirection);

/// The Images section's state and I/O. Provided as context from the App
/// root (and by a scene root); every field is a signal, so the store is
/// `Copy`.
#[derive(Clone, Copy)]
pub struct ImageStore {
    /// Every engine's images, as that engine last listed them.
    pub by_engine: Signal<BTreeMap<EngineId, Vec<Image>>>,
    /// Inspector details by (engine, image id).
    pub details: Signal<HashMap<(EngineId, String), ImageEntry>>,
    /// Pulls, newest last; finished ones stay until dismissed.
    pub pulls: Signal<Vec<PullState>>,
    pub tab: Signal<ImagesTab>,
    pub query: Signal<String>,
    pub query_rev: Signal<u32>,
    pub filter: Signal<ImageFilter>,
    pub sort: Signal<Sort>,
    /// The Pull image panel under the tabs.
    pub pull_open: Signal<bool>,
    requested: Signal<HashSet<(EngineId, String)>>,
    cancels: Signal<HashMap<PullKey, CancellationToken>>,
}

impl ImageStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_engine: Signal::new(BTreeMap::new()),
            details: Signal::new(HashMap::new()),
            pulls: Signal::new(Vec::new()),
            tab: Signal::new(ImagesTab::default()),
            query: Signal::new(String::new()),
            query_rev: Signal::new(0),
            filter: Signal::new(ImageFilter::default()),
            sort: Signal::new((SortColumn::Created, SortDirection::Descending)),
            pull_open: Signal::new(false),
            requested: Signal::new(HashSet::new()),
            cancels: Signal::new(HashMap::new()),
        }
    }

    /// Put an engine's list in as if it had been fetched (scenes, tests).
    pub fn seed(&self, engine: &EngineId, images: Vec<Image>) {
        let mut by_engine = self.by_engine;
        by_engine.write().insert(engine.clone(), images);
    }

    /// Put a detail in as if it had been fetched (scenes, tests).
    pub fn seed_detail(&self, engine: &EngineId, detail: ImageDetail) {
        let key = (engine.clone(), detail.image.id.clone());
        let mut requested = self.requested;
        requested.write().insert(key.clone());
        let mut details = self.details;
        details
            .write()
            .insert(key, ImageEntry::Ready(Box::new(detail)));
    }

    /// How many pulls are running (the "1 pulling" of the tab).
    #[must_use]
    pub fn pulling(&self) -> usize {
        self.pulls.read().iter().filter(|p| p.is_running()).count()
    }

    /// The engine that has `image_id`, preferring `prefer` (the engine the
    /// scope names), else the first that lists it.
    #[must_use]
    pub fn owner_of(&self, image_id: &str, prefer: Option<&EngineId>) -> Option<EngineId> {
        let by_engine = self.by_engine.peek();
        let has = |engine: &EngineId| {
            by_engine
                .get(engine)
                .is_some_and(|images| images.iter().any(|i| i.id == image_id))
        };
        if let Some(engine) = prefer.filter(|e| has(e)) {
            return Some(engine.clone());
        }
        by_engine.keys().find(|e| has(e)).cloned()
    }

    /// List every connected engine's images again.
    pub fn refresh(&self, services: &Services) {
        let clients: Vec<(EngineId, std::sync::Arc<dyn berth_engine::EngineClient>)> =
            match services.clients.lock() {
                Ok(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                Err(poisoned) => poisoned
                    .into_inner()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            };
        for (engine, client) in clients {
            let mut by_engine = self.by_engine;
            io(
                async move { client.images().await },
                move |result| match result {
                    Ok(images) => {
                        by_engine.write().insert(engine, images);
                    }
                    Err(err) => {
                        tracing::warn!(%err, engine = %engine.0, "images list failed");
                    }
                },
            );
        }
    }

    /// Ask for one image's detail (layers, ports, env keys) unless it was
    /// asked for already. Safe to call on every render.
    pub fn ensure_detail(&self, services: &Services, engine: &EngineId, id: &str) {
        let key = (engine.clone(), id.to_string());
        let mut requested = self.requested;
        if requested.peek().contains(&key) {
            return;
        }
        requested.write().insert(key.clone());
        let mut details = self.details;
        let Some(client) = services.client(engine) else {
            details.write().insert(
                key,
                ImageEntry::Failed {
                    message: format!("Engine {} is not connected", engine.0),
                    gone: false,
                },
            );
            return;
        };
        let wanted = id.to_string();
        io(async move { client.image(&wanted).await }, move |result| {
            let entry = match result {
                Ok(detail) => ImageEntry::Ready(Box::new(detail)),
                Err(err) => ImageEntry::Failed {
                    gone: matches!(err, berth_engine::EngineError::NotFound(_)),
                    message: err.to_string(),
                },
            };
            details.write().insert(key, entry);
        });
    }

    /// Forget a detail so the next [`ensure_detail`] reads it again.
    ///
    /// [`ensure_detail`]: ImageStore::ensure_detail
    pub fn invalidate(&self, engine: &EngineId, id: &str) {
        let mut requested = self.requested;
        requested.write().remove(&(engine.clone(), id.to_string()));
    }

    /// Start pulling `reference` on `engine`. A pull of the same reference
    /// that is running is left alone.
    pub fn pull(
        &self,
        services: &Services,
        engine: &EngineId,
        reference: &str,
        platform: Option<&str>,
    ) -> Result<(), String> {
        let reference = check_reference(reference)?;
        let key = (engine.clone(), reference.clone());
        if self
            .pulls
            .peek()
            .iter()
            .any(|p| p.key() == key && p.is_running())
        {
            return Ok(());
        }
        let client = services
            .client(engine)
            .ok_or_else(|| format!("Engine {} is not connected", engine.0))?;
        let mut pulls = self.pulls;
        {
            let mut list = pulls.write();
            list.retain(|p| p.key() != key);
            list.push(PullState::new(engine.clone(), &reference, platform));
        }
        let cancel = services.cancel.child_token();
        let mut cancels = self.cancels;
        cancels.write().insert(key.clone(), cancel.clone());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PullMessage>();
        let wanted = reference.clone();
        let wanted_platform = platform.map(str::to_string);
        let stream_cancel = cancel.clone();
        spawn_io(async move {
            let mut stream =
                client.pull(&wanted, wanted_platform.as_deref(), stream_cancel.clone());
            let mut failure = None;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(event) => {
                        let _ = tx.send(PullMessage::Event(event));
                    }
                    Err(err) => {
                        failure = Some(err.to_string());
                        break;
                    }
                }
            }
            let end = match failure {
                Some(message) => PullMessage::Failed(message),
                None if stream_cancel.is_cancelled() => PullMessage::Cancelled,
                None => PullMessage::Done,
            };
            let _ = tx.send(end);
        });
        let store = *self;
        let store_services = services.clone();
        let engine = engine.clone();
        spawn(async move {
            while let Some(message) = rx.recv().await {
                let finished = !matches!(message, PullMessage::Event(_));
                let outcome = match &message {
                    PullMessage::Done => Some(Outcome::Ok),
                    PullMessage::Failed(reason) => Some(Outcome::Failed {
                        reason: reason.clone(),
                    }),
                    _ => None,
                };
                let mut pulls = store.pulls;
                if let Some(state) = pulls.write().iter_mut().find(|p| p.key() == key) {
                    match message {
                        PullMessage::Event(event) => state.apply(&event),
                        PullMessage::Done => state.phase = PullPhase::Done,
                        PullMessage::Failed(reason) => state.phase = PullPhase::Failed(reason),
                        PullMessage::Cancelled => state.phase = PullPhase::Cancelled,
                    }
                }
                if finished {
                    let mut cancels = store.cancels;
                    cancels.write().remove(&key);
                    if let Some(outcome) = outcome {
                        activity::record(
                            &store_services.store,
                            &activity::entry_for_action(
                                engine.clone(),
                                "image.pull",
                                Subject::Image(reference.clone()),
                                reference.clone(),
                                None,
                                outcome,
                            ),
                        );
                    }
                    // The engine's own image event refreshes the lists;
                    // asking again covers an engine that sends none.
                    store.refresh(&store_services);
                    break;
                }
            }
        });
        Ok(())
    }

    /// Stop a running pull.
    pub fn cancel_pull(&self, key: &PullKey) {
        if let Some(token) = self.cancels.peek().get(key) {
            token.cancel();
        }
    }

    /// Drop a finished pull from the list.
    pub fn dismiss_pull(&self, key: &PullKey) {
        let mut pulls = self.pulls;
        pulls.write().retain(|p| p.key() != *key || p.is_running());
    }

    /// Give `image` another name (`docker tag`). `done` hears the result.
    pub fn tag(
        &self,
        services: &Services,
        engine: &EngineId,
        image: &Image,
        repo: &str,
        tag: &str,
        done: impl FnOnce(Result<(), String>) + 'static,
    ) {
        let Some(client) = services.client(engine) else {
            done(Err(format!("Engine {} is not connected", engine.0)));
            return;
        };
        let store = services.store.clone();
        let engine = engine.clone();
        let id = image.id.clone();
        let target = format!("{repo}:{tag}");
        let (repo, tag) = (repo.to_string(), tag.to_string());
        let this = *self;
        let services = services.clone();
        io(
            async move {
                client
                    .tag_image(&id, &repo, &tag)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |result| {
                activity::record(
                    &store,
                    &activity::entry_for_action(
                        engine.clone(),
                        "image.tag",
                        Subject::Image(target.clone()),
                        target,
                        None,
                        match &result {
                            Ok(()) => Outcome::Ok,
                            Err(reason) => Outcome::Failed {
                                reason: reason.clone(),
                            },
                        },
                    ),
                );
                if result.is_ok() {
                    this.refresh(&services);
                }
                done(result);
            },
        );
    }

    /// Create and start a container from `spec` (the Run sheet). `done`
    /// hears the new container's id.
    pub fn run(
        &self,
        services: &Services,
        engine: &EngineId,
        spec: ContainerSpec,
        done: impl FnOnce(Result<ContainerId, String>) + 'static,
    ) {
        let Some(client) = services.client(engine) else {
            done(Err(format!("Engine {} is not connected", engine.0)));
            return;
        };
        let store = services.store.clone();
        let engine = engine.clone();
        let name = spec.name.clone();
        let image = spec.image.reference.clone();
        io(
            async move { client.create(&spec, true).await.map_err(|e| e.to_string()) },
            move |result| {
                activity::record(
                    &store,
                    &activity::entry_for_action(
                        engine,
                        "image.run",
                        Subject::Image(image),
                        name,
                        None,
                        match &result {
                            Ok(_) => Outcome::Ok,
                            Err(reason) => Outcome::Failed {
                                reason: reason.clone(),
                            },
                        },
                    ),
                );
                done(result);
            },
        );
    }
}

impl Default for ImageStore {
    fn default() -> Self {
        Self::new()
    }
}

enum PullMessage {
    Event(PullProgress),
    Done,
    Failed(String),
    Cancelled,
}

/// Run `work` on the I/O runtime and `then` with its result on the UI side.
fn io<T: Send + 'static>(
    work: impl std::future::Future<Output = T> + Send + 'static,
    then: impl FnOnce(T) + 'static,
) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    spawn_io(async move {
        let _ = tx.send(work.await);
    });
    spawn(async move {
        if let Ok(value) = rx.await {
            then(value);
        }
    });
}

/// Provide one [`ImageStore`] from the App root (or a scene root).
pub fn provide_image_store() -> ImageStore {
    use_hook(|| provide_context(ImageStore::new()))
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
