//! The folder service (P5-T4, PLAN §6.18, D36): what the folder browser and
//! the Drop folder flow (P5-T5) know about folders on disk.
//!
//! Berth reads only what the user is looking at. [`read_folder`] lists one
//! level and returns folders only. [`annotate`] stats the few named files of
//! one child (`detect::markers`) and counts its entries; nothing here walks
//! a tree or runs in the background.
//!
//! The pure pieces ([`Browser`], [`Known`], [`places`], [`complete_names`],
//! [`record_open`]) carry no I/O of their own and are unit-tested; the I/O
//! functions are plain blocking functions with `async` wrappers that run
//! them on the blocking pool and stop early on a cancelled token.
//!
//! API for P5-T5: [`read_folder`], [`read_preview`], [`marker_of`],
//! [`Known::from_projects`], [`record_open`], [`pick_engine`], [`places`],
//! [`recent_places`], [`display_path`], [`resolve_input`].

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use berth_core::{
    Detection, EngineId, FoundKind, PinnedFolder, Project, ProjectId, ServiceKind, StackGuess,
};
use berth_rules::detect::{self, FolderMarkers};
use tokio_util::sync::CancellationToken;

/// The URL `open` takes to show Files and Folders in System Settings.
pub const PRIVACY_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders";

/// A folder with more entries than this counts as this many.
pub const ITEM_CAP: usize = 10_000;

/// Children annotated per blocking call, so a large folder fills in while
/// the user looks at it.
pub const ANNOTATE_CHUNK: usize = 64;

/// Recent folders the Places list shows.
pub const RECENT_SHOWN: usize = 6;

// ---------------------------------------------------------------------------------------------
// Errors

/// Why a folder could not be read, in the words the browser needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderError {
    /// macOS privacy (`EPERM`) or file permissions (`EACCES`).
    NoAccess(PathBuf),
    NotFound(PathBuf),
    NotAFolder(PathBuf),
    Failed(PathBuf, String),
}

impl FolderError {
    /// Maps the error of reading `folder`. `EPERM` (1) is how macOS answers a
    /// folder the app has no Files and Folders grant for; `EACCES` (13) is a
    /// plain mode bit. Rust reports both as `PermissionDenied`: both are
    /// "no access".
    #[must_use]
    pub fn from_io(folder: &Path, err: &io::Error) -> Self {
        let folder = folder.to_path_buf();
        match err.kind() {
            io::ErrorKind::PermissionDenied => Self::NoAccess(folder),
            io::ErrorKind::NotFound => Self::NotFound(folder),
            io::ErrorKind::NotADirectory => Self::NotAFolder(folder),
            _ => Self::Failed(folder, err.to_string()),
        }
    }

    #[must_use]
    pub fn folder(&self) -> &Path {
        match self {
            Self::NoAccess(p) | Self::NotFound(p) | Self::NotAFolder(p) | Self::Failed(p, _) => p,
        }
    }

    /// "Documents", or "/" for the root.
    #[must_use]
    pub fn folder_name(&self) -> String {
        folder_name(self.folder())
    }

    /// The state's title: "Berth can't read Documents".
    #[must_use]
    pub fn headline(&self) -> String {
        let name = self.folder_name();
        match self {
            Self::NoAccess(_) | Self::Failed(..) => format!("Berth can't read {name}"),
            Self::NotFound(_) => format!("{name} is not there"),
            Self::NotAFolder(_) => format!("{name} is not a folder"),
        }
    }

    #[must_use]
    pub fn is_no_access(&self) -> bool {
        matches!(self, Self::NoAccess(_))
    }
}

impl std::fmt::Display for FolderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed(_, message) => write!(f, "{}: {message}", self.headline()),
            _ => f.write_str(&self.headline()),
        }
    }
}

/// The last component of `path` ("/" for the root).
#[must_use]
pub fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

// ---------------------------------------------------------------------------------------------
// Entries

/// What reading one child folder found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Details {
    pub markers: FolderMarkers,
    /// `None` when the count was left out (see [`is_privacy_protected`]) or the
    /// folder could not be read.
    pub items: Option<usize>,
}

/// One folder in a column. Files are not listed: a project is a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub hidden: bool,
    /// `None` until [`annotate`] has run for this child.
    pub details: Option<Details>,
}

/// What the right-hand side of a row says about a folder, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    InBerth,
    Compose,
    Dockerfile,
}

impl Marker {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::InBerth => "In Berth",
            Self::Compose => "Compose",
            Self::Dockerfile => "Dockerfile",
        }
    }
}

/// The marker a folder shows: known project, else Compose, else Dockerfile.
#[must_use]
pub fn marker_of(markers: FolderMarkers, known: bool) -> Option<Marker> {
    if known {
        Some(Marker::InBerth)
    } else if markers.compose {
        Some(Marker::Compose)
    } else if markers.dockerfile {
        Some(Marker::Dockerfile)
    } else {
        None
    }
}

/// Sorts the way a column reads: by name, ignoring case; the exact name breaks
/// a tie, so the order never changes between reads (AD31).
pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Reads one level of `folder`: its subfolders (symlinks to folders count),
/// sorted. Nothing below them is opened.
///
/// # Errors
/// [`FolderError`] mapped from the failed `read_dir`.
pub fn read_folder(folder: &Path) -> Result<Vec<Entry>, FolderError> {
    let reader = std::fs::read_dir(folder).map_err(|e| FolderError::from_io(folder, &e))?;
    let mut out = Vec::new();
    for item in reader {
        let Ok(item) = item else { continue };
        let path = item.path();
        let is_dir = match item.file_type() {
            Ok(kind) if kind.is_dir() => true,
            Ok(kind) if kind.is_symlink() => std::fs::metadata(&path).is_ok_and(|m| m.is_dir()),
            _ => false,
        };
        if !is_dir {
            continue;
        }
        let name = item.file_name().to_string_lossy().into_owned();
        out.push(Entry {
            hidden: name.starts_with('.'),
            name,
            path,
            details: None,
        });
    }
    sort_entries(&mut out);
    Ok(out)
}

/// The macOS folders that answer a read with a privacy prompt the first time.
/// Berth does not read their children to draw a parent's column: the prompt
/// belongs to the user opening that folder, not to listing the home.
#[must_use]
pub fn is_privacy_protected(path: &Path, home: &Path) -> bool {
    path.parent() == Some(home)
        && matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some("Desktop" | "Documents" | "Downloads")
        )
}

/// Counts the entries of `folder` that are not hidden, stopping at
/// [`ITEM_CAP`]. `None` when it cannot be read.
#[must_use]
pub fn count_items(folder: &Path) -> Option<usize> {
    let reader = std::fs::read_dir(folder).ok()?;
    let mut count = 0;
    for item in reader.flatten() {
        if !item.file_name().to_string_lossy().starts_with('.') {
            count += 1;
            if count >= ITEM_CAP {
                break;
            }
        }
    }
    Some(count)
}

/// Reads the named files and the entry count of one child of a visible column.
#[must_use]
pub fn annotate(path: &Path, home: &Path) -> Details {
    if is_privacy_protected(path, home) {
        return Details::default();
    }
    Details {
        markers: detect::markers(path),
        items: count_items(path),
    }
}

/// Annotates `paths` (one chunk), checking the token between children.
#[must_use]
pub fn annotate_chunk(
    paths: &[PathBuf],
    home: &Path,
    cancel: &CancellationToken,
) -> Vec<(PathBuf, Details)> {
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        if cancel.is_cancelled() {
            break;
        }
        out.push((path.clone(), annotate(path, home)));
    }
    out
}

/// [`read_folder`] on the blocking pool.
///
/// # Errors
/// As [`read_folder`]; a task that panicked reads as `Failed`.
pub async fn list(folder: PathBuf) -> Result<Vec<Entry>, FolderError> {
    let path = folder.clone();
    tokio::task::spawn_blocking(move || read_folder(&path))
        .await
        .unwrap_or_else(|e| Err(FolderError::Failed(folder, e.to_string())))
}

/// Annotates `paths` in chunks of [`ANNOTATE_CHUNK`], handing each chunk to
/// `on_chunk` as it is ready. Stops when `cancel` fires.
pub async fn annotate_all(
    paths: Vec<PathBuf>,
    home: PathBuf,
    cancel: CancellationToken,
    mut on_chunk: impl FnMut(Vec<(PathBuf, Details)>),
) {
    for chunk in paths.chunks(ANNOTATE_CHUNK) {
        if cancel.is_cancelled() {
            return;
        }
        let chunk = chunk.to_vec();
        let (home, token) = (home.clone(), cancel.clone());
        let Ok(done) =
            tokio::task::spawn_blocking(move || annotate_chunk(&chunk, &home, &token)).await
        else {
            return;
        };
        if cancel.is_cancelled() {
            return;
        }
        on_chunk(done);
    }
}

// ---------------------------------------------------------------------------------------------
// Known folders ("In Berth")

/// A project Berth already has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownProject {
    pub id: ProjectId,
    pub name: String,
}

/// The folders of the projects Berth already knows, for the "In Berth" marker
/// and "Go to <project>".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Known(BTreeMap<PathBuf, KnownProject>);

impl Known {
    #[must_use]
    pub fn from_projects(projects: &[Project]) -> Self {
        let mut map = BTreeMap::new();
        for project in projects {
            if let Some(folder) = &project.folder {
                map.entry(folder.clone()).or_insert_with(|| KnownProject {
                    id: project.id.clone(),
                    name: project.name.clone(),
                });
            }
        }
        Self(map)
    }

    #[must_use]
    pub fn get(&self, folder: &Path) -> Option<&KnownProject> {
        self.0.get(folder)
    }

    #[must_use]
    pub fn contains(&self, folder: &Path) -> bool {
        self.0.contains_key(folder)
    }
}

// ---------------------------------------------------------------------------------------------
// Paths typed by the user

/// `~/Dev/side` for a path under `home`; the path itself elsewhere.
#[must_use]
pub fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// What the path bar's text means: `~` is the home, a relative path starts at
/// `base`, a trailing slash is ignored.
#[must_use]
pub fn resolve_input(input: &str, base: &Path, home: &Path) -> PathBuf {
    let text = input.trim();
    let joined = if text == "~" {
        home.to_path_buf()
    } else if let Some(rest) = text.strip_prefix("~/") {
        home.join(rest)
    } else if text.starts_with('/') {
        PathBuf::from(text)
    } else {
        base.join(text)
    };
    // Normalise `.`, `..` and trailing slashes without touching the disk.
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
}

/// What a typed path is on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathState {
    Folder,
    Missing,
    NotAFolder,
    Denied,
}

/// One `stat` of a typed path.
#[must_use]
pub fn classify_path(path: &Path) -> PathState {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => PathState::Folder,
        Ok(_) => PathState::NotAFolder,
        Err(e) => match FolderError::from_io(path, &e) {
            FolderError::NoAccess(_) => PathState::Denied,
            _ => PathState::Missing,
        },
    }
}

/// The text Tab completes `prefix` to among `names`: the whole name when one
/// matches, else the longest shared start, else `None` when nothing is gained.
/// Case is ignored for matching and taken from the first match.
#[must_use]
pub fn complete_names(prefix: &str, names: &[String]) -> Option<String> {
    let lower = prefix.to_lowercase();
    let mut matches: Vec<&String> = names
        .iter()
        .filter(|n| n.to_lowercase().starts_with(&lower))
        .collect();
    matches.sort();
    let first = matches.first()?;
    if matches.len() == 1 {
        return Some(format!("{first}/"));
    }
    let mut common = first.chars().count();
    for other in &matches[1..] {
        let shared = first
            .chars()
            .zip(other.chars())
            .take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
            .count();
        common = common.min(shared);
    }
    let text: String = first.chars().take(common).collect();
    (text.chars().count() > prefix.chars().count()).then_some(text)
}

/// Tab on the path bar: reads the folder being typed into (one level) and
/// completes the last part. The result keeps the text the user typed up to
/// that part (`~/Dev/si` -> `~/Dev/side/`).
#[must_use]
pub fn complete_path(input: &str, base: &Path, home: &Path, show_hidden: bool) -> Option<String> {
    let (typed_parent, prefix) = match input.rfind('/') {
        Some(at) => (&input[..=at], &input[at + 1..]),
        None => ("", input),
    };
    let parent = if typed_parent.is_empty() {
        base.to_path_buf()
    } else {
        resolve_input(typed_parent, base, home)
    };
    let names: Vec<String> = read_folder(&parent)
        .ok()?
        .into_iter()
        .filter(|e| show_hidden || !e.hidden || prefix.starts_with('.'))
        .map(|e| e.name)
        .collect();
    complete_names(prefix, &names).map(|done| format!("{typed_parent}{done}"))
}

// ---------------------------------------------------------------------------------------------
// Places and recents

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceKind {
    Home,
    Favourite,
    Standard,
    Volume,
}

/// One row of the Places and Recent lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
    pub kind: PlaceKind,
}

impl Place {
    #[must_use]
    pub fn icon(&self) -> &'static str {
        match self.kind {
            PlaceKind::Home => "home",
            PlaceKind::Volume => "hard-drive",
            PlaceKind::Favourite | PlaceKind::Standard => "folder",
        }
    }
}

/// The volumes under `volumes_dir` (`/Volumes`): one place per entry.
#[must_use]
pub fn mounted_volumes(volumes_dir: &Path) -> Vec<Place> {
    let Ok(reader) = std::fs::read_dir(volumes_dir) else {
        return Vec::new();
    };
    let mut out: Vec<Place> = reader
        .flatten()
        .filter(|item| std::fs::metadata(item.path()).is_ok_and(|m| m.is_dir()))
        .map(|item| Place {
            name: item.file_name().to_string_lossy().into_owned(),
            path: item.path(),
            kind: PlaceKind::Volume,
        })
        .filter(|place| !place.name.starts_with('.'))
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Places in the order the frame shows them: Home, the favourites (Dev), then
/// Desktop and Documents, then the mounted volumes. A folder is listed once.
#[must_use]
pub fn places(home: &Path, favourites: &[PathBuf], volumes: Vec<Place>) -> Vec<Place> {
    let mut out = vec![Place {
        name: "Home".to_string(),
        path: home.to_path_buf(),
        kind: PlaceKind::Home,
    }];
    let push = |place: Place, out: &mut Vec<Place>| {
        if !out.iter().any(|p| p.path == place.path) {
            out.push(place);
        }
    };
    for path in favourites {
        push(
            Place {
                name: folder_name(path),
                path: path.clone(),
                kind: PlaceKind::Favourite,
            },
            &mut out,
        );
    }
    for name in ["Desktop", "Documents"] {
        push(
            Place {
                name: name.to_string(),
                path: home.join(name),
                kind: PlaceKind::Standard,
            },
            &mut out,
        );
    }
    for volume in volumes {
        push(volume, &mut out);
    }
    out
}

/// The folders opened most recently, newest first.
#[must_use]
pub fn recent_places(store: &berth_store::Store, limit: usize) -> Vec<Place> {
    match store.recent_folders(limit) {
        Ok(folders) => folders
            .into_iter()
            .map(|path| Place {
                name: folder_name(&path),
                path,
                kind: PlaceKind::Standard,
            })
            .collect(),
        Err(err) => {
            tracing::warn!(%err, "recent folders load failed");
            Vec::new()
        }
    }
}

/// The folder the browser opens at: the parent of the last folder opened,
/// else the first favourite, else the home.
#[must_use]
pub fn start_folder(recent: &[Place], favourites: &[PathBuf], home: &Path) -> PathBuf {
    recent
        .first()
        .and_then(|p| p.path.parent())
        .map(Path::to_path_buf)
        .or_else(|| favourites.first().cloned())
        .unwrap_or_else(|| home.to_path_buf())
}

// ---------------------------------------------------------------------------------------------
// Opening a folder: pins and recents

/// What opening a folder did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    /// Berth already has a project there: go to it.
    GoTo(ProjectId),
    /// The folder is pinned now and listed under Recent.
    Pinned,
}

/// The engine a new pin belongs to: the engine in scope, else the default,
/// else the first running one, else `colima:default`.
#[must_use]
pub fn pick_engine(
    scope: Option<&EngineId>,
    default: Option<&EngineId>,
    running: &[EngineId],
) -> EngineId {
    scope
        .or(default)
        .or_else(|| running.first())
        .cloned()
        .unwrap_or_else(|| EngineId("colima:default".to_string()))
}

/// Records that the user opened `folder`: it goes to the top of Recent, and
/// unless Berth already has a project there it is pinned (a pin already
/// stored keeps its date).
///
/// # Errors
/// The store's error when the pin or the recent entry cannot be written.
pub fn record_open(
    store: &berth_store::Store,
    folder: &Path,
    engine: &EngineId,
    known: &Known,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<OpenOutcome, berth_store::StoreError> {
    store.push_recent(folder)?;
    if let Some(project) = known.get(folder) {
        return Ok(OpenOutcome::GoTo(project.id.clone()));
    }
    if !store.pins()?.iter().any(|p| p.folder == folder) {
        store.pin(&PinnedFolder {
            folder: folder.to_path_buf(),
            engine: engine.clone(),
            added: now,
        })?;
    }
    Ok(OpenOutcome::Pinned)
}

/// Remove the pin of `folder` (Remove from Berth, P10-T3). Only the store's row goes: the folder,
/// its files and every Docker resource are left alone. Says whether there was a pin to remove.
///
/// # Errors
/// The store's error when the pin cannot be read or deleted.
pub fn remove_pin(
    store: &berth_store::Store,
    folder: &Path,
) -> Result<bool, berth_store::StoreError> {
    let pinned = store.pins()?.iter().any(|p| p.folder == folder);
    store.unpin(folder)?;
    Ok(pinned)
}

// ---------------------------------------------------------------------------------------------
// Preview of the highlighted folder

/// What the right-hand pane shows about one folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub folder: PathBuf,
    pub items: usize,
    pub detection: Detection,
}

/// Reads `folder` for the preview: the entry count and `detect`, which opens
/// only its named manifest files.
///
/// # Errors
/// [`FolderError`] when the folder cannot be read.
pub fn read_preview(folder: &Path) -> Result<Preview, FolderError> {
    let reader = std::fs::read_dir(folder).map_err(|e| FolderError::from_io(folder, &e))?;
    let items = reader
        .flatten()
        .filter(|i| !i.file_name().to_string_lossy().starts_with('.'))
        .count();
    let detection = detect::detect(folder)
        .map_err(|e| FolderError::Failed(folder.to_path_buf(), e.to_string()))?;
    Ok(Preview {
        folder: folder.to_path_buf(),
        items,
        detection,
    })
}

/// [`read_preview`] on the blocking pool.
///
/// # Errors
/// As [`read_preview`].
pub async fn preview(folder: PathBuf) -> Result<Preview, FolderError> {
    let path = folder.clone();
    tokio::task::spawn_blocking(move || read_preview(&path))
        .await
        .unwrap_or_else(|e| Err(FolderError::Failed(folder, e.to_string())))
}

/// "Node 22 · pnpm · Next.js 15" for a detected stack.
#[must_use]
pub fn stack_label(stack: &StackGuess) -> String {
    fn join(parts: Vec<String>) -> String {
        parts.join(" \u{b7} ")
    }
    match stack {
        StackGuess::Node {
            pm,
            framework,
            framework_version,
            version,
        } => {
            let mut parts = vec![version
                .as_ref()
                .map_or_else(|| "Node".to_string(), |v| format!("Node {v}"))];
            parts.push(pm.clone());
            if let Some(framework) = framework {
                parts.push(match framework_version {
                    Some(v) => format!("{framework} {v}"),
                    None => framework.clone(),
                });
            }
            join(parts)
        }
        StackGuess::Python { tool, framework } => {
            let mut parts = vec!["Python".to_string(), tool.clone()];
            parts.extend(framework.clone());
            join(parts)
        }
        StackGuess::Go => "Go".to_string(),
        StackGuess::Rust => "Rust".to_string(),
        StackGuess::Php { framework } => join(
            std::iter::once("PHP".to_string())
                .chain(framework.clone())
                .collect(),
        ),
        StackGuess::Ruby { framework } => join(
            std::iter::once("Ruby".to_string())
                .chain(framework.clone())
                .collect(),
        ),
        StackGuess::Java { build, framework } => {
            let mut parts = vec!["Java".to_string(), build.clone()];
            parts.extend(framework.clone());
            join(parts)
        }
        StackGuess::Static => "Static site".to_string(),
    }
}

/// The name a service kind goes by in the preview.
#[must_use]
pub fn service_label(kind: ServiceKind) -> &'static str {
    match kind {
        ServiceKind::Postgres => "Postgres",
        ServiceKind::Mysql => "MySQL",
        ServiceKind::Mongo => "MongoDB",
        ServiceKind::Redis => "Redis",
        ServiceKind::Rabbitmq => "RabbitMQ",
        ServiceKind::Meilisearch => "Meilisearch",
        ServiceKind::Minio => "S3 storage",
        ServiceKind::Mailpit => "Mailpit",
    }
}

/// Which of the three files the preview always lists was found, and its detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundRow {
    pub file: &'static str,
    /// `Some("FROM node:22-alpine")` when found; `None` for "not found".
    pub detail: Option<String>,
}

/// The Dockerfile, `.env` and Compose rows of the preview, in that order.
#[must_use]
pub fn found_rows(detection: &Detection) -> [FoundRow; 3] {
    let mut dockerfile = None;
    let mut env = None;
    let mut compose = None;
    for found in &detection.found {
        match &found.kind {
            FoundKind::Dockerfile { from } => {
                dockerfile = Some(
                    from.as_ref()
                        .map_or_else(String::new, |f| format!("FROM {f}")),
                );
            }
            FoundKind::Env { keys } => {
                env = Some(format!(
                    "{keys} key{}, values stay hidden",
                    if *keys == 1 { "" } else { "s" }
                ));
            }
            FoundKind::Compose if compose.is_none() => {
                compose = Some(
                    found
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
            }
            _ => {}
        }
    }
    [
        FoundRow {
            file: "Dockerfile",
            detail: dockerfile,
        },
        FoundRow {
            file: ".env",
            detail: env,
        },
        FoundRow {
            file: "compose.yaml",
            detail: compose,
        },
    ]
}

// ---------------------------------------------------------------------------------------------
// The browser model

/// What a column holds.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnState {
    /// Being read; `slow` once it has taken longer than a local folder should.
    Loading {
        slow: bool,
    },
    Ready(Vec<Entry>),
    Failed(FolderError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub folder: PathBuf,
    pub state: ColumnState,
    pub selected: Option<PathBuf>,
}

impl Column {
    fn loading(folder: PathBuf) -> Self {
        Self {
            folder,
            state: ColumnState::Loading { slow: false },
            selected: None,
        }
    }
}

/// The column view: one column per folder from the root down to the folder
/// being looked at, the highlighted child of each, the filter and the hidden
/// files switch. Pure state; the sheet feeds it listings.
#[derive(Debug, Clone, PartialEq)]
pub struct Browser {
    pub columns: Vec<Column>,
    pub filter: String,
    pub show_hidden: bool,
}

/// The folders from `root` down to `target`, both included. A target outside
/// the root is its own chain.
#[must_use]
pub fn chain(root: &Path, target: &Path) -> Vec<PathBuf> {
    let Ok(rest) = target.strip_prefix(root) else {
        return vec![target.to_path_buf()];
    };
    let mut out = vec![root.to_path_buf()];
    let mut at = root.to_path_buf();
    for part in rest.components() {
        at.push(part.as_os_str());
        out.push(at.clone());
    }
    out
}

/// The first column's folder: the home for anything under it, else `/`.
#[must_use]
pub fn root_for(target: &Path, home: &Path) -> PathBuf {
    if target.starts_with(home) {
        home.to_path_buf()
    } else {
        PathBuf::from("/")
    }
}

impl Browser {
    /// A browser whose chain ends at `target`, every column still loading.
    #[must_use]
    pub fn at(target: &Path, home: &Path) -> Self {
        let mut browser = Self {
            columns: Vec::new(),
            filter: String::new(),
            show_hidden: false,
        };
        browser.open_path(target, home);
        browser
    }

    /// Points the columns at `target`. Columns that already show the same
    /// folders stay as they are; the folders that need reading are returned.
    pub fn open_path(&mut self, target: &Path, home: &Path) -> Vec<PathBuf> {
        let wanted = chain(&root_for(target, home), target);
        let keep = wanted
            .iter()
            .zip(&self.columns)
            .take_while(|(w, c)| **w == c.folder)
            .count();
        self.columns.truncate(keep);
        let mut to_read = Vec::new();
        for folder in &wanted[keep..] {
            self.columns.push(Column::loading(folder.clone()));
            to_read.push(folder.clone());
        }
        // Each column but the last highlights the next folder of the chain.
        for (i, column) in self.columns.iter_mut().enumerate() {
            column.selected = wanted.get(i + 1).cloned();
        }
        self.filter.clear();
        to_read
    }

    /// The listing of `folder` arrived (or failed). A folder that is no longer
    /// a column is ignored.
    pub fn set_listing(&mut self, folder: &Path, result: Result<Vec<Entry>, FolderError>) {
        if let Some(column) = self.columns.iter_mut().find(|c| c.folder == folder) {
            column.state = match result {
                Ok(entries) => ColumnState::Ready(entries),
                Err(err) => ColumnState::Failed(err),
            };
        }
    }

    /// The first chunk of annotations arrived for `folder`'s children.
    pub fn set_details(&mut self, folder: &Path, chunk: Vec<(PathBuf, Details)>) {
        let Some(column) = self.columns.iter_mut().find(|c| c.folder == folder) else {
            return;
        };
        let ColumnState::Ready(entries) = &mut column.state else {
            return;
        };
        let by_path: BTreeMap<PathBuf, Details> = chunk.into_iter().collect();
        for entry in entries {
            if let Some(details) = by_path.get(&entry.path) {
                entry.details = Some(*details);
            }
        }
    }

    /// `folder` is still loading after the slow threshold.
    pub fn mark_slow(&mut self, folder: &Path) {
        if let Some(column) = self.columns.iter_mut().find(|c| c.folder == folder) {
            if matches!(column.state, ColumnState::Loading { .. }) {
                column.state = ColumnState::Loading { slow: true };
            }
        }
    }

    /// Highlights `path` in `column`. The columns to its right stay only when
    /// the next one shows that very folder.
    pub fn select(&mut self, column: usize, path: &Path) {
        let Some(col) = self.columns.get_mut(column) else {
            return;
        };
        col.selected = Some(path.to_path_buf());
        if self.columns.get(column + 1).map(|c| c.folder.as_path()) != Some(path) {
            self.columns.truncate(column + 1);
        }
    }

    /// Highlights `path` in `column` and shows its contents in the next one.
    /// Returns the folder to read when the column is new.
    pub fn descend(&mut self, column: usize, path: &Path) -> Option<PathBuf> {
        self.select(column, path);
        if self.columns.len() == column + 1 {
            self.columns.push(Column::loading(path.to_path_buf()));
            self.filter.clear();
            return Some(path.to_path_buf());
        }
        self.filter.clear();
        None
    }

    /// One column to the left; at the first column, the root's parent.
    /// Returns the folders to read.
    pub fn up(&mut self, home: &Path) -> Vec<PathBuf> {
        if self.columns.len() > 1 {
            self.columns.pop();
            self.filter.clear();
            return Vec::new();
        }
        let parent = self
            .columns
            .first()
            .and_then(|c| c.folder.parent())
            .map(Path::to_path_buf);
        match parent {
            Some(parent) => {
                let selected = self.columns.first().map(|c| c.folder.clone());
                self.columns.clear();
                let read = self.open_path(&parent, home);
                if let Some(last) = self.columns.last_mut() {
                    last.selected = selected;
                }
                read
            }
            None => Vec::new(),
        }
    }

    /// The column the user is in: the last one.
    #[must_use]
    pub fn deepest(&self) -> Option<&Column> {
        self.columns.last()
    }

    /// The folder in the path bar.
    #[must_use]
    pub fn current_folder(&self) -> Option<&Path> {
        self.deepest().map(|c| c.folder.as_path())
    }

    /// What "Open" acts on: the highlighted folder of the last column, else the
    /// last column's folder. `None` while that column cannot be read.
    #[must_use]
    pub fn target(&self) -> Option<PathBuf> {
        let column = self.deepest()?;
        if matches!(column.state, ColumnState::Failed(_)) {
            return None;
        }
        Some(
            column
                .selected
                .clone()
                .unwrap_or_else(|| column.folder.clone()),
        )
    }

    /// The folder the right-hand pane describes: a highlighted folder only.
    #[must_use]
    pub fn preview_target(&self) -> Option<PathBuf> {
        let column = self.deepest()?;
        if matches!(column.state, ColumnState::Failed(_)) {
            return None;
        }
        column.selected.clone()
    }

    /// The failure of the last column, when it has one.
    #[must_use]
    pub fn failure(&self) -> Option<&FolderError> {
        match &self.deepest()?.state {
            ColumnState::Failed(err) => Some(err),
            _ => None,
        }
    }

    /// Indexes into `entries` of the rows of `column` that pass the hidden
    /// switch and (for the last column only) the filter.
    #[must_use]
    pub fn visible(&self, column: usize) -> Vec<usize> {
        let Some(col) = self.columns.get(column) else {
            return Vec::new();
        };
        let ColumnState::Ready(entries) = &col.state else {
            return Vec::new();
        };
        let filter = if column + 1 == self.columns.len() {
            self.filter.trim().to_lowercase()
        } else {
            String::new()
        };
        entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.show_hidden || !e.hidden)
            .filter(|(_, e)| filter.is_empty() || e.name.to_lowercase().contains(&filter))
            .map(|(i, _)| i)
            .collect()
    }

    /// Moves the highlight of the last column by `delta` rows (clamped) and
    /// returns the folder now highlighted.
    pub fn step(&mut self, delta: i32) -> Option<PathBuf> {
        let last = self.columns.len().checked_sub(1)?;
        let rows = self.visible(last);
        if rows.is_empty() {
            return None;
        }
        let ColumnState::Ready(entries) = &self.columns[last].state else {
            return None;
        };
        let current = self.columns[last]
            .selected
            .as_ref()
            .and_then(|sel| rows.iter().position(|&i| entries[i].path == *sel));
        let next = match current {
            None if delta >= 0 => 0,
            None => rows.len() - 1,
            Some(at) => {
                let moved = i64::try_from(at).unwrap_or(0) + i64::from(delta);
                usize::try_from(moved.max(0))
                    .unwrap_or(0)
                    .min(rows.len() - 1)
            }
        };
        let path = entries[rows[next]].path.clone();
        self.columns[last].selected = Some(path.clone());
        Some(path)
    }
}

// ---------------------------------------------------------------------------------------------
// Opening the privacy settings

/// Shows Files and Folders in System Settings with the system `open` (rule:
/// binaries come from `SearchPath`). Fire and forget; a failure is logged.
pub fn open_privacy_settings() {
    let Some(open) = berth_sys::SearchPath::from_process().find("open") else {
        tracing::warn!("no open binary on the search path");
        return;
    };
    let work = async move {
        let result = berth_sys::run_capture(
            &open,
            &[std::ffi::OsString::from(PRIVACY_SETTINGS_URL)],
            berth_sys::SpawnOpts {
                timeout: Some(std::time::Duration::from_secs(10)),
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await;
        if let Err(err) = result {
            tracing::warn!(%err, "open System Settings failed");
        }
    };
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(work);
    } else {
        std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                rt.block_on(work);
            }
        });
    }
}

#[cfg(test)]
#[path = "folders_tests.rs"]
mod tests;
