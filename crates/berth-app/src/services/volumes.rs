//! Volumes (P4-T4): what the Volumes table lists, the create work the
//! actions service runs, the size fetch and the helper image check. Backup
//! and restore are in [`super::volume_backup`]. A volume knows its engine
//! (`Volume::engine`, AD27).

use std::collections::BTreeMap;
use std::sync::Arc;

use bytesize::ByteSize;
use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

use berth_core::{Container, EngineId, Project, Volume, VolumeSpec};
use berth_engine::EngineClient;

use crate::state::EngineScope;

/// The pending key and activity action of a create.
pub const CREATE: &str = "volume.create";

/// The image the browser, backup and restore read a volume through (D51).
pub const HELPER_IMAGE: &str = "alpine:3";
/// What the first-use dialog says the helper weighs: `alpine:3` is 13.6 MB
/// on disk on this Mac's engine; the size is known only once it is there.
pub const HELPER_SIZE: ByteSize = ByteSize::mb(14);

// ---------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageFilter {
    #[default]
    All,
    /// Volumes no container uses (the ones Clean up would offer).
    Unused,
}

impl UsageFilter {
    pub const ALL: [Self; 2] = [Self::All, Self::Unused];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Unused => "unused",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Unused => "Unused",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|f| f.id() == id)
            .unwrap_or(Self::All)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortColumn {
    Name,
    Project,
    Size,
    UsedBy,
    Driver,
    Created,
}

/// One table row, everything the cells and the filter read.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeRow {
    pub name: String,
    pub engine: EngineId,
    pub project: Option<String>,
    /// `None` until the size fetch has answered.
    pub size: Option<ByteSize>,
    pub used_by: Vec<String>,
    pub driver: String,
    pub created: Option<DateTime<Utc>>,
}

impl VolumeRow {
    #[must_use]
    pub fn unused(&self) -> bool {
        self.used_by.is_empty()
    }

    /// "Calculating…" until the size arrives.
    #[must_use]
    pub fn size_text(&self) -> String {
        self.size
            .map_or_else(|| "Calculating\u{2026}".to_string(), berth_core::fmt::bytes)
    }

    /// The Used by cell: the container, or the first and "+N".
    #[must_use]
    pub fn used_by_text(&self) -> String {
        match self.used_by.as_slice() {
            [] => "\u{2014}".to_string(),
            [one] => one.clone(),
            [first, rest @ ..] => format!("{first} +{}", rest.len()),
        }
    }
}

/// What [`build_list`] reads.
pub struct ListInput<'a> {
    pub volumes: &'a [Volume],
    pub containers: &'a [Container],
    pub projects: &'a [Project],
    pub scope: &'a EngineScope,
    /// Sizes the fetch has found, for volumes whose own size is empty.
    pub sizes: &'a BTreeMap<String, ByteSize>,
    pub text: &'a str,
    pub filter: UsageFilter,
    pub sort: Option<(SortColumn, bool)>,
}

/// The rows to draw and the counts the header needs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VolumeList {
    /// Volumes in the engine scope.
    pub total: usize,
    /// Volumes in scope that match the filter text, whatever the usage filter.
    pub matched: usize,
    pub unused: usize,
    pub rows: Vec<VolumeRow>,
}

fn row_of(input: &ListInput<'_>, volume: &Volume) -> VolumeRow {
    VolumeRow {
        name: volume.name.clone(),
        engine: volume.engine.clone(),
        project: volume
            .project
            .as_ref()
            .and_then(|id| input.projects.iter().find(|p| p.id == *id))
            .map(|p| p.name.clone()),
        size: volume
            .size
            .or_else(|| input.sizes.get(&volume.name).copied()),
        used_by: volume
            .used_by
            .iter()
            .map(|id| {
                input
                    .containers
                    .iter()
                    .find(|c| c.id == *id)
                    .map_or_else(|| id.0.chars().take(12).collect(), |c| c.name.clone())
            })
            .collect(),
        driver: volume.driver.clone(),
        created: volume.created,
    }
}

fn matches_text(row: &VolumeRow, terms: &[String]) -> bool {
    let haystack = format!(
        "{} {} {} {}",
        row.name,
        row.project.as_deref().unwrap_or(""),
        row.driver,
        row.used_by.join(" ")
    )
    .to_lowercase();
    terms.iter().all(|t| haystack.contains(t))
}

/// Compare two optional values, the empty one last whichever way the
/// column runs.
fn present_first<T: Ord>(a: Option<T>, b: Option<T>, ascending: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Some(a), Some(b)) if ascending => a.cmp(&b),
        (Some(a), Some(b)) => b.cmp(&a),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare(
    a: &VolumeRow,
    b: &VolumeRow,
    column: SortColumn,
    ascending: bool,
) -> std::cmp::Ordering {
    let dir = |order: std::cmp::Ordering| if ascending { order } else { order.reverse() };
    match column {
        SortColumn::Name => dir(a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        SortColumn::Project => present_first(
            a.project.as_ref().map(|p| p.to_lowercase()),
            b.project.as_ref().map(|p| p.to_lowercase()),
            ascending,
        ),
        SortColumn::Size => present_first(a.size, b.size, ascending),
        SortColumn::UsedBy => dir(a.used_by.len().cmp(&b.used_by.len())),
        SortColumn::Driver => dir(a.driver.cmp(&b.driver)),
        SortColumn::Created => present_first(a.created, b.created, ascending),
    }
}

/// The volumes of the engine scope, filtered and sorted.
#[must_use]
pub fn build_list(input: &ListInput<'_>) -> VolumeList {
    let terms: Vec<String> = input
        .text
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let in_scope: Vec<VolumeRow> = input
        .volumes
        .iter()
        .map(|v| row_of(input, v))
        .filter(|row| crate::ui::scope_includes(input.scope, &row.engine))
        .collect();
    let total = in_scope.len();
    let unused = in_scope.iter().filter(|r| r.unused()).count();
    let by_text: Vec<VolumeRow> = in_scope
        .into_iter()
        .filter(|r| matches_text(r, &terms))
        .collect();
    let matched = by_text.len();
    let mut rows: Vec<VolumeRow> = by_text
        .into_iter()
        .filter(|r| input.filter == UsageFilter::All || r.unused())
        .collect();
    let (column, ascending) = input.sort.unwrap_or((SortColumn::Name, true));
    rows.sort_by(|a, b| compare(a, b, column, ascending).then_with(|| a.name.cmp(&b.name)));
    VolumeList {
        total,
        matched,
        unused,
        rows,
    }
}

// ---------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------

/// Docker's rule for a volume name: letters and digits first, then letters,
/// digits, `_`, `.` and `-`.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.chars().count() >= 2
        && name.len() <= 255
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// The sentence under the name field, or `None` when the name will do.
#[must_use]
pub fn name_problem(name: &str, taken: bool) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if taken {
        return Some(format!("A volume named {name} already exists"));
    }
    if !valid_name(name) {
        return Some(
            "Use letters, digits, _ . - and start with a letter or digit (2 characters at least)"
                .to_string(),
        );
    }
    None
}

/// The driver options typed as `key=value` words, separated by spaces or
/// lines: `type=tmpfs device=tmpfs o=size=100m,uid=1000`.
///
/// # Errors
/// The word that is not `key=value`.
pub fn parse_options(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for word in text.split_whitespace() {
        match word.split_once('=') {
            Some((key, value)) if !key.is_empty() => {
                out.insert(key.to_string(), value.to_string());
            }
            _ => return Err(format!("\"{word}\" is not key=value")),
        }
    }
    Ok(out)
}

/// The spec the create sheet sends.
///
/// # Errors
/// The first problem with the name or the options.
pub fn create_spec(name: &str, driver: &str, options: &str) -> Result<VolumeSpec, String> {
    let name = name.trim();
    if let Some(problem) = name_problem(name, false) {
        return Err(problem);
    }
    if name.is_empty() {
        return Err("Give the volume a name".to_string());
    }
    Ok(VolumeSpec {
        name: name.to_string(),
        driver: driver.trim().to_string(),
        driver_opts: parse_options(options)?,
        labels: BTreeMap::new(),
    })
}

/// What the actions service does to one volume. Deleting is not here: it
/// is `EngineAction::Remove` with `ResourceRef::Volume` (AD25a), run by
/// `ActionsService::remove`.
#[derive(Debug, Clone, PartialEq)]
pub enum VolumeOp {
    Create(VolumeSpec),
}

impl VolumeOp {
    /// The pending key and activity action.
    #[must_use]
    pub fn action_id(&self) -> &'static str {
        match self {
            Self::Create(_) => CREATE,
        }
    }
}

/// Run one create to its outcome. A create says its own result (the sheet
/// is closing).
///
/// # Errors
/// The engine's message.
pub async fn run(
    client: &dyn EngineClient,
    op: VolumeOp,
    cancel: &CancellationToken,
) -> Result<crate::services::edit::Done, String> {
    let work = async {
        match op {
            VolumeOp::Create(spec) => {
                client
                    .create_volume(&spec)
                    .await
                    .map(|_| crate::services::edit::Done {
                        replaced_by: None,
                        message: Some(created_message()),
                    })
            }
        }
    };
    tokio::select! {
        () = cancel.cancelled() => Err("cancelled".to_string()),
        result = work => result.map_err(|e| e.to_string()),
    }
}

/// "Created 1 volume".
#[must_use]
pub fn created_message() -> String {
    "Created 1 volume".to_string()
}

// ---------------------------------------------------------------------
// Sizes
// ---------------------------------------------------------------------

/// Ask every running engine in scope for its volume sizes (`system df`,
/// slow on a big engine, so never on the UI path). A name two engines both
/// have keeps the later answer.
pub async fn fetch_sizes(clients: Vec<Arc<dyn EngineClient>>) -> BTreeMap<String, ByteSize> {
    let mut out = BTreeMap::new();
    for client in clients {
        match client.volume_sizes().await {
            Ok(sizes) => out.extend(sizes),
            Err(err) => tracing::warn!(%err, engine = %client.id().0, "volume sizes failed"),
        }
    }
    out
}

/// Whether the table has a row still showing "Calculating…" that the cache
/// cannot fill.
#[must_use]
pub fn needs_sizes(volumes: &[Volume], sizes: &BTreeMap<String, ByteSize>) -> bool {
    volumes
        .iter()
        .any(|v| v.size.is_none() && !sizes.contains_key(&v.name))
}

// ---------------------------------------------------------------------
// The helper image
// ---------------------------------------------------------------------

/// Whether the engine has the helper image.
///
/// # Errors
/// The engine's message.
pub async fn helper_present(client: &dyn EngineClient) -> Result<bool, String> {
    let images = client.images().await.map_err(|e| e.to_string())?;
    Ok(images
        .iter()
        .any(|i| i.repo_tags.iter().any(|t| t == HELPER_IMAGE)))
}

/// Pull the helper image, reporting the share done (0 to 100) as layers
/// arrive.
///
/// # Errors
/// The engine's message.
pub async fn pull_helper(
    client: &dyn EngineClient,
    cancel: CancellationToken,
    mut progress: impl FnMut(u8) + Send,
) -> Result<(), String> {
    use futures::StreamExt;
    let mut layers: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut stream = client.pull(HELPER_IMAGE, None, cancel);
    while let Some(item) = stream.next().await {
        let update = item.map_err(|e| e.to_string())?;
        if let (Some(layer), Some(total)) = (update.layer, update.total) {
            layers.insert(layer, (update.current.unwrap_or(0), total));
        }
        progress(percent(&layers));
    }
    Ok(())
}

/// [`helper_present`] on the I/O runtime, for the UI to await.
///
/// # Errors
/// The engine's message.
pub async fn helper_present_io(client: Arc<dyn EngineClient>) -> Result<bool, String> {
    crate::services::files::on_io(async move { helper_present(client.as_ref()).await })
        .await
        .unwrap_or_else(|| Err("Cancelled".to_string()))
}

/// [`pull_helper`] on the I/O runtime; `on_progress` runs here, on the
/// caller's side, so it may write signals.
///
/// # Errors
/// The engine's message.
pub async fn pull_helper_io(
    client: Arc<dyn EngineClient>,
    cancel: CancellationToken,
    mut on_progress: impl FnMut(u8),
) -> Result<(), String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u8>();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    crate::services::actions::spawn_io(async move {
        let result = pull_helper(client.as_ref(), cancel, move |p| {
            let _ = tx.send(p);
        })
        .await;
        let _ = done_tx.send(result);
    });
    while let Some(p) = rx.recv().await {
        on_progress(p);
    }
    done_rx
        .await
        .unwrap_or_else(|_| Err("Cancelled".to_string()))
}

/// The share of bytes pulled over all layers seen so far, 0 to 100.
#[must_use]
pub fn percent(layers: &BTreeMap<String, (u64, u64)>) -> u8 {
    let (done, total) = layers.values().fold((0_u64, 0_u64), |(d, t), (cur, tot)| {
        (d + cur.min(tot), t + tot)
    });
    if total == 0 {
        return 0;
    }
    u8::try_from((done * 100 / total).min(100)).unwrap_or(100)
}

#[cfg(test)]
#[path = "volumes_tests.rs"]
mod tests;
