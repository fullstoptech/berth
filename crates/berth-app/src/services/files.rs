//! Container files (P3-T8, PLAN D51, D23): list a folder, read one file,
//! put one file. Everything goes through the archive API, which needs no
//! shell and works on stopped containers (P1-T6 accepted it in place of
//! D51's `stat` helper).
//!
//! The pure half is tar in, tar out and path arithmetic: [`extract_file`]
//! (one file from a download), [`build_upload_tar`] (one file for an
//! upload), and the path helpers the breadcrumbs use. The I/O half is
//! [`FileSource`], one container's listing, archive stream and upload, with
//! [`EngineFiles`] over an `EngineClient` (`list_files` streams tar headers
//! only); the UI never calls an engine, it calls [`list_dir`],
//! [`read_file`] and [`write_file`], which run on the I/O runtime.

use std::io::Read;
use std::sync::Arc;

use berth_core::{ContainerId, FileEntry, FileKind, FileTarget};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use berth_engine::{Bs, EngineClient, EngineError};

use crate::services::actions::spawn_io;

/// The most a download or an upload holds in memory.
pub const MAX_TRANSFER_BYTES: u64 = 256 * 1024 * 1024;

/// Why a files operation did not go through. `Display` is the sentence the
/// panel shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesError {
    NotFound(String),
    NotADirectory(String),
    NotAFile { name: String, kind: &'static str },
    TooLarge { name: String, limit: u64 },
    InvalidName(String),
    Engine(String),
    Local(String),
    Cancelled,
}

impl std::fmt::Display for FilesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotADirectory(path) => write!(f, "{path} is not a folder"),
            Self::NotAFile { name, kind } => write!(f, "{name} is a {kind}, not a file"),
            Self::TooLarge { name, limit } => write!(
                f,
                "{name} is larger than {}",
                berth_core::fmt::bytes(bytesize::ByteSize::b(*limit))
            ),
            Self::InvalidName(name) => write!(f, "\"{name}\" is not a valid file name"),
            Self::NotFound(message) | Self::Engine(message) | Self::Local(message) => {
                f.write_str(message)
            }
            Self::Cancelled => f.write_str("Cancelled"),
        }
    }
}

impl From<EngineError> for FilesError {
    fn from(err: EngineError) -> Self {
        match err {
            EngineError::NotFound(message) => Self::NotFound(message),
            EngineError::Cancelled => Self::Cancelled,
            other => Self::Engine(other.to_string()),
        }
    }
}

// ---------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------

/// A typed or computed folder path made absolute and tidy: a leading `/`,
/// no empty, `.` or `..` parts, no trailing `/` (the root stays `/`).
#[must_use]
pub fn normalize_dir(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

/// `dir` and a name inside it.
#[must_use]
pub fn join_path(dir: &str, name: &str) -> String {
    let dir = normalize_dir(dir);
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The folder above `dir`; the root's parent is the root.
#[must_use]
pub fn parent_dir(dir: &str) -> String {
    normalize_dir(&format!("{}/..", normalize_dir(dir)))
}

/// One step of the breadcrumb trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub path: String,
}

/// The trail for `dir`: the root as "/" first, then one crumb per folder.
#[must_use]
pub fn breadcrumbs(dir: &str) -> Vec<Crumb> {
    let dir = normalize_dir(dir);
    let mut out = vec![Crumb {
        label: "/".to_string(),
        path: "/".to_string(),
    }];
    let mut path = String::new();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        path.push('/');
        path.push_str(part);
        out.push(Crumb {
            label: part.to_string(),
            path: path.clone(),
        });
    }
    out
}

/// Folders first, then names, each group in case-insensitive order.
pub fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        let rank = |e: &FileEntry| u8::from(e.kind != FileKind::Dir);
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

// ---------------------------------------------------------------------
// Tar, pure
// ---------------------------------------------------------------------

fn kind_of(entry_type: tar::EntryType) -> FileKind {
    match entry_type {
        tar::EntryType::Directory => FileKind::Dir,
        tar::EntryType::Symlink | tar::EntryType::Link => FileKind::Link,
        tar::EntryType::Regular | tar::EntryType::Continuous | tar::EntryType::GNUSparse => {
            FileKind::File
        }
        _ => FileKind::Other,
    }
}

fn kind_word(kind: FileKind) -> &'static str {
    match kind {
        FileKind::File => "file",
        FileKind::Dir => "folder",
        FileKind::Link => "link",
        FileKind::Other => "special file",
    }
}

fn parse_error(err: &std::io::Error) -> FilesError {
    FilesError::Engine(format!("Could not read the archive: {err}"))
}

/// The content of the file called `name` in a download tar (the archive of
/// a file holds one entry named by its basename).
///
/// # Errors
/// There is no such entry, or it is a folder, link or special file.
pub fn extract_file(tar_bytes: &[u8], name: &str) -> Result<Vec<u8>, FilesError> {
    let mut archive = tar::Archive::new(std::io::Cursor::new(tar_bytes));
    let entries = archive.entries().map_err(|e| parse_error(&e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| parse_error(&e))?;
        let path = entry.path().map_err(|e| parse_error(&e))?;
        let raw = path.to_string_lossy().into_owned();
        if raw.trim_matches('/') != name {
            continue;
        }
        let kind = kind_of(entry.header().entry_type());
        if kind != FileKind::File {
            return Err(FilesError::NotAFile {
                name: name.to_string(),
                kind: kind_word(kind),
            });
        }
        let mut out = Vec::new();
        entry
            .read_to_end(&mut out)
            .map_err(|e| FilesError::Engine(format!("Could not read {name}: {e}")))?;
        return Ok(out);
    }
    Err(FilesError::NotFound(format!(
        "{name} is not in the archive"
    )))
}

/// Is `name` one plain file name (no folders, no dot names)?
#[must_use]
pub fn valid_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\0')
}

/// A tar holding one file, for `EngineClient::upload` into a folder. The
/// file is owned by root with mode 0644.
///
/// # Errors
/// The name is not a plain file name.
pub fn build_upload_tar(
    name: &str,
    contents: &[u8],
    modified: DateTime<Utc>,
) -> Result<Vec<u8>, FilesError> {
    if !valid_file_name(name) {
        return Err(FilesError::InvalidName(name.to_string()));
    }
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(contents.len() as u64);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(u64::try_from(modified.timestamp()).unwrap_or(0));
    let mut builder = tar::Builder::new(Vec::new());
    builder
        .append_data(&mut header, name, contents)
        .map_err(|e| FilesError::Local(format!("Could not pack {name}: {e}")))?;
    builder
        .into_inner()
        .map_err(|e| FilesError::Local(format!("Could not pack {name}: {e}")))
}

// ---------------------------------------------------------------------
// I/O
// ---------------------------------------------------------------------

/// One container's files as the archive API offers them.
#[async_trait::async_trait]
pub trait FileSource: Send + Sync {
    /// The entries directly inside `dir`.
    async fn list(&self, dir: &str) -> Result<Vec<FileEntry>, FilesError>;
    /// The tar stream for `path`.
    fn download(&self, path: &str, cancel: CancellationToken) -> Bs<Bytes>;
    /// Unpack `tar` into the folder `dir`, replacing files of the same name.
    async fn upload(&self, dir: &str, tar: Bytes) -> Result<(), FilesError>;
}

/// A container's files through an engine client.
pub struct EngineFiles {
    client: Arc<dyn EngineClient>,
    target: FileTarget,
}

impl EngineFiles {
    #[must_use]
    pub fn new(client: Arc<dyn EngineClient>, container: ContainerId) -> Self {
        Self {
            client,
            target: FileTarget::Container(container),
        }
    }
}

#[async_trait::async_trait]
impl FileSource for EngineFiles {
    async fn list(&self, dir: &str) -> Result<Vec<FileEntry>, FilesError> {
        Ok(self.client.list_files(&self.target, dir).await?)
    }

    fn download(&self, path: &str, cancel: CancellationToken) -> Bs<Bytes> {
        self.client.download(&self.target, path, cancel)
    }

    async fn upload(&self, dir: &str, tar: Bytes) -> Result<(), FilesError> {
        Ok(self.client.upload(&self.target, dir, tar).await?)
    }
}

/// Context that stands in for the engine: scenes and tests provide one
/// (`provide_context(FilesBackend(..))`), and the Files panel uses it
/// before it looks for an engine client.
#[derive(Clone)]
pub struct FilesBackend(pub Arc<dyn FileSource>);

/// Run `future` on the I/O runtime and wait for it here.
async fn on_io<T: Send + 'static>(
    future: impl std::future::Future<Output = T> + Send + 'static,
) -> Option<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    spawn_io(async move {
        let _ = tx.send(future.await);
    });
    rx.await.ok()
}

/// The entries inside `dir`, sorted folders first. Cancel stops the read.
///
/// # Errors
/// The engine's message, a missing folder, or a path that is not a folder.
pub async fn list_dir(
    source: Arc<dyn FileSource>,
    dir: String,
    cancel: CancellationToken,
) -> Result<Vec<FileEntry>, FilesError> {
    let dir = normalize_dir(&dir);
    on_io(async move {
        let listed = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(FilesError::Cancelled),
            listed = source.list(&dir) => listed,
        };
        let mut entries = listed?;
        sort_entries(&mut entries);
        Ok(entries)
    })
    .await
    .unwrap_or(Err(FilesError::Cancelled))
}

/// The whole tar of `path`, refusing past [`MAX_TRANSFER_BYTES`].
async fn collect_tar(
    source: &dyn FileSource,
    path: &str,
    name: &str,
    cancel: CancellationToken,
) -> Result<Vec<u8>, FilesError> {
    let mut stream = source.download(path, cancel.clone());
    let mut out: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.map_err(FilesError::from)?);
        if out.len() as u64 > MAX_TRANSFER_BYTES {
            return Err(FilesError::TooLarge {
                name: name.to_string(),
                limit: MAX_TRANSFER_BYTES,
            });
        }
    }
    if cancel.is_cancelled() {
        return Err(FilesError::Cancelled);
    }
    Ok(out)
}

/// The bytes of the file at `path`.
///
/// # Errors
/// The engine's message, a missing file, a folder or link, or a file over
/// [`MAX_TRANSFER_BYTES`].
pub async fn read_file(
    source: Arc<dyn FileSource>,
    path: String,
    cancel: CancellationToken,
) -> Result<Vec<u8>, FilesError> {
    let name = path.rsplit('/').next().unwrap_or("").to_string();
    on_io(async move {
        let tar = collect_tar(source.as_ref(), &path, &name, cancel).await?;
        extract_file(&tar, &name)
    })
    .await
    .unwrap_or(Err(FilesError::Cancelled))
}

/// Put one file in `dir`, replacing a file of the same name.
///
/// # Errors
/// The name is not a plain file name, the file is over
/// [`MAX_TRANSFER_BYTES`], or the engine refused.
pub async fn write_file(
    source: Arc<dyn FileSource>,
    dir: String,
    name: String,
    contents: Vec<u8>,
    modified: DateTime<Utc>,
) -> Result<(), FilesError> {
    if contents.len() as u64 > MAX_TRANSFER_BYTES {
        return Err(FilesError::TooLarge {
            name,
            limit: MAX_TRANSFER_BYTES,
        });
    }
    on_io(async move {
        let tar = build_upload_tar(&name, &contents, modified)?;
        source.upload(&normalize_dir(&dir), Bytes::from(tar)).await
    })
    .await
    .unwrap_or(Err(FilesError::Cancelled))
}

/// Write downloaded bytes where the save panel pointed.
///
/// # Errors
/// The disk's message.
pub async fn save_local(path: std::path::PathBuf, contents: Vec<u8>) -> Result<(), FilesError> {
    on_io(async move {
        tokio::fs::write(&path, contents)
            .await
            .map_err(|e| FilesError::Local(format!("Could not save {}: {e}", path.display())))
    })
    .await
    .unwrap_or(Err(FilesError::Cancelled))
}

/// Read a local file for upload, refusing past [`MAX_TRANSFER_BYTES`].
///
/// # Errors
/// The disk's message, or a file over the limit.
pub async fn load_local(path: std::path::PathBuf) -> Result<Vec<u8>, FilesError> {
    on_io(async move {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let size = tokio::fs::metadata(&path)
            .await
            .map_err(|e| FilesError::Local(format!("Could not read {name}: {e}")))?
            .len();
        if size > MAX_TRANSFER_BYTES {
            return Err(FilesError::TooLarge {
                name,
                limit: MAX_TRANSFER_BYTES,
            });
        }
        tokio::fs::read(&path)
            .await
            .map_err(|e| FilesError::Local(format!("Could not read {name}: {e}")))
    })
    .await
    .unwrap_or(Err(FilesError::Cancelled))
}

// ---------------------------------------------------------------------
// An in-memory container, for scenes and tests
// ---------------------------------------------------------------------

#[derive(Clone)]
struct MemoryNode {
    kind: FileKind,
    contents: Vec<u8>,
    modified: i64,
}

/// A container's files held in memory, speaking the archive API the way
/// Docker does (root named by its basename, `/` listed from depth one).
/// Scenes mount a panel on it; tests drive the service with it.
pub struct MemoryFiles {
    nodes: std::sync::Mutex<std::collections::BTreeMap<String, MemoryNode>>,
    failure: Option<String>,
}

impl MemoryFiles {
    /// An empty container (only `/`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            failure: None,
        }
    }

    /// A container whose every read fails with `message`.
    #[must_use]
    pub fn failing(message: &str) -> Self {
        Self {
            failure: Some(message.to_string()),
            ..Self::new()
        }
    }

    /// Add a file; its folders appear with it.
    #[must_use]
    pub fn with_file(self, path: &str, contents: &[u8], modified: i64) -> Self {
        self.put(path, FileKind::File, contents.to_vec(), modified);
        self
    }

    /// Add an empty folder.
    #[must_use]
    pub fn with_dir(self, path: &str, modified: i64) -> Self {
        self.put(path, FileKind::Dir, Vec::new(), modified);
        self
    }

    fn put(&self, path: &str, kind: FileKind, contents: Vec<u8>, modified: i64) {
        let path = normalize_dir(path);
        let Ok(mut nodes) = self.nodes.lock() else {
            return;
        };
        let mut parent = parent_dir(&path);
        while parent != "/" {
            nodes.entry(parent.clone()).or_insert(MemoryNode {
                kind: FileKind::Dir,
                contents: Vec::new(),
                modified,
            });
            parent = parent_dir(&parent);
        }
        nodes.insert(
            path,
            MemoryNode {
                kind,
                contents,
                modified,
            },
        );
    }

    /// The content of the file at `path`, as an upload left it.
    #[must_use]
    pub fn contents(&self, path: &str) -> Option<Vec<u8>> {
        let nodes = self.nodes.lock().ok()?;
        nodes
            .get(&normalize_dir(path))
            .filter(|n| n.kind == FileKind::File)
            .map(|n| n.contents.clone())
    }

    fn archive(&self, path: &str) -> Result<Vec<u8>, EngineError> {
        if let Some(message) = &self.failure {
            return Err(EngineError::Docker(message.clone()));
        }
        let path = normalize_dir(path);
        let nodes = self
            .nodes
            .lock()
            .map_err(|_| EngineError::Docker("files are locked".to_string()))?;
        let root = if path == "/" {
            None
        } else {
            Some(nodes.get(&path).ok_or_else(|| {
                EngineError::NotFound(format!("Could not find the file {path} in container"))
            })?)
        };
        let base = path.rsplit('/').next().unwrap_or("").to_string();
        let mut builder = tar::Builder::new(Vec::new());
        let mut add = |name: String, node: &MemoryNode| -> Result<(), EngineError> {
            let mut header = tar::Header::new_gnu();
            header.set_mtime(u64::try_from(node.modified).unwrap_or(0));
            header.set_mode(0o644);
            match node.kind {
                FileKind::Dir => {
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_size(0);
                    builder.append_data(&mut header, format!("{name}/"), std::io::empty())
                }
                _ => {
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_size(node.contents.len() as u64);
                    builder.append_data(&mut header, name, node.contents.as_slice())
                }
            }
            .map_err(|e| EngineError::Docker(e.to_string()))
        };
        match root {
            None => {
                for (p, node) in nodes.iter() {
                    add(p.trim_start_matches('/').to_string(), node)?;
                }
            }
            Some(node) => {
                add(base.clone(), node)?;
                if node.kind == FileKind::Dir {
                    let prefix = format!("{path}/");
                    for (p, child) in nodes.iter().filter(|(p, _)| p.starts_with(&prefix)) {
                        add(format!("{base}/{}", &p[prefix.len()..]), child)?;
                    }
                }
            }
        }
        builder
            .into_inner()
            .map_err(|e| EngineError::Docker(e.to_string()))
    }
}

impl Default for MemoryFiles {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl FileSource for MemoryFiles {
    async fn list(&self, dir: &str) -> Result<Vec<FileEntry>, FilesError> {
        if let Some(message) = &self.failure {
            return Err(FilesError::Engine(message.clone()));
        }
        let dir = normalize_dir(dir);
        let nodes = self
            .nodes
            .lock()
            .map_err(|_| FilesError::Engine("files are locked".to_string()))?;
        if dir != "/" {
            match nodes.get(&dir) {
                None => {
                    return Err(FilesError::NotFound(format!(
                        "Could not find the file {dir} in container"
                    )))
                }
                Some(node) if node.kind != FileKind::Dir => {
                    return Err(FilesError::NotADirectory(dir));
                }
                Some(_) => {}
            }
        }
        Ok(nodes
            .iter()
            .filter(|(path, _)| parent_dir(path) == dir && path.as_str() != "/")
            .map(|(path, node)| FileEntry {
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                kind: node.kind,
                size: (node.kind == FileKind::File)
                    .then(|| bytesize::ByteSize::b(node.contents.len() as u64)),
                modified: DateTime::from_timestamp(node.modified, 0),
            })
            .collect())
    }

    fn download(&self, path: &str, _cancel: CancellationToken) -> Bs<Bytes> {
        // Chunks of seven bytes, so a header is always split across reads.
        let items: Vec<Result<Bytes, EngineError>> = match self.archive(path) {
            Ok(tar) => tar
                .chunks(7)
                .map(|c| Ok(Bytes::copy_from_slice(c)))
                .collect(),
            Err(err) => vec![Err(err)],
        };
        futures::stream::iter(items).boxed()
    }

    async fn upload(&self, dir: &str, tar: Bytes) -> Result<(), FilesError> {
        let mut archive = tar::Archive::new(std::io::Cursor::new(tar));
        let entries = archive.entries().map_err(|e| parse_error(&e))?;
        for entry in entries {
            let mut entry = entry.map_err(|e| parse_error(&e))?;
            let name = entry
                .path()
                .map_err(|e| parse_error(&e))?
                .to_string_lossy()
                .into_owned();
            let mut contents = Vec::new();
            entry
                .read_to_end(&mut contents)
                .map_err(|e| parse_error(&e))?;
            let modified = i64::try_from(entry.header().mtime().unwrap_or(0)).unwrap_or(0);
            self.put(
                &join_path(dir, name.trim_matches('/')),
                FileKind::File,
                contents,
                modified,
            );
        }
        Ok(())
    }
}
