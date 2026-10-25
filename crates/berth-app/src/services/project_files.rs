//! What the Project detail screen reads from a project's folder and how it
//! hands the folder to other apps (P5-T6).
//!
//! The Files tab lists the folder through [`ProjectFs`]: [`DiskFs`] for a
//! real folder, [`MemoryFs`] for a scene or a test (mock projects have no
//! folder on this Mac). Listings keep one stable order, so a row never moves
//! under the pointer (AD31): compose files, `.env` files, other env files,
//! Dockerfiles, folders, then the rest, each group by name.
//!
//! `.env` values are secrets. [`ProjectFs::env_keys`] returns names only; a
//! value is read by [`ProjectFs::env_value`] when the user clicks to reveal
//! it, and the preview of an env file is refused. Nothing here logs a value.
//!
//! "Open folder", "Open in editor", "Reveal in Finder" and "Open terminal
//! here" are built as [`OpenCommand`]s from `berth_sys::SearchPath`
//! (binaries come only from there) and run by [`launch`].

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use berth_core::{Project, ProjectId};

/// Entries one folder listing keeps; the rest are counted in [`DirListing::more`].
pub const LIST_CAP: usize = 500;
/// How much of a file the preview reads.
pub const PREVIEW_MAX_BYTES: u64 = 256 * 1024;
/// How many lines the preview keeps.
pub const PREVIEW_MAX_LINES: usize = 2000;
/// A file with a NUL byte in its first bytes is shown as binary.
const BINARY_PROBE: usize = 8 * 1024;

/// What a name in the folder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    /// A symbolic link: listed, never followed.
    Link,
}

/// Why the Files tab treats a file specially.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRole {
    /// `compose.yaml`, `docker-compose.yml`, `compose.override.yaml`…
    Compose,
    /// `.env`, `.env.local`: values are secrets, shown masked.
    Env,
    /// `.env.example` and its kin: no secrets by convention, shown as text.
    EnvTemplate,
    Dockerfile,
    Other,
}

impl FileRole {
    /// The group the file sorts in: lower first.
    fn rank(self) -> u8 {
        match self {
            Self::Compose => 0,
            Self::Env => 1,
            Self::EnvTemplate => 2,
            Self::Dockerfile => 3,
            Self::Other => 5,
        }
    }
}

/// The role a file name has.
#[must_use]
pub fn classify(name: &str) -> FileRole {
    let lower = name.to_ascii_lowercase();
    let is_yaml = |s: &str| s.ends_with(".yaml") || s.ends_with(".yml");
    if matches!(
        lower.as_str(),
        "compose.yaml" | "compose.yml" | "docker-compose.yaml" | "docker-compose.yml"
    ) || ((lower.starts_with("compose.") || lower.starts_with("docker-compose."))
        && is_yaml(&lower))
    {
        return FileRole::Compose;
    }
    if lower == ".env" {
        return FileRole::Env;
    }
    if let Some(suffix) = lower.strip_prefix(".env.") {
        return if matches!(
            suffix,
            "example" | "sample" | "template" | "dist" | "defaults"
        ) {
            FileRole::EnvTemplate
        } else {
            FileRole::Env
        };
    }
    if lower == "dockerfile" || lower.starts_with("dockerfile.") || lower.ends_with(".dockerfile") {
        return FileRole::Dockerfile;
    }
    FileRole::Other
}

/// One name in a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsEntry {
    pub name: String,
    /// The path from the project folder, `/`-separated, no leading slash.
    pub rel: String,
    pub kind: EntryKind,
    pub role: FileRole,
    pub size: Option<u64>,
}

impl FsEntry {
    /// The sort key of a listing: group, then name without regard to case.
    fn order(&self) -> (u8, String, String) {
        let rank = if self.kind == EntryKind::Dir {
            4
        } else {
            self.role.rank()
        };
        (rank, self.name.to_lowercase(), self.name.clone())
    }
}

/// One folder, in the order the tab shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirListing {
    pub entries: Vec<FsEntry>,
    /// How many more names the folder holds past [`LIST_CAP`].
    pub more: usize,
}

impl DirListing {
    /// Sort `entries` into the tab's order and apply [`LIST_CAP`].
    #[must_use]
    pub fn from_entries(mut entries: Vec<FsEntry>) -> Self {
        entries.sort_by_cached_key(FsEntry::order);
        let more = entries.len().saturating_sub(LIST_CAP);
        entries.truncate(LIST_CAP);
        Self { entries, more }
    }
}

/// What a text preview holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    Text {
        lines: Vec<String>,
        /// Lines in the file, or in what was read when `truncated`.
        total_lines: usize,
        bytes: u64,
        truncated: bool,
    },
    Binary {
        bytes: u64,
    },
}

/// Why a read failed, in words for the tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    Denied,
    /// An env file's text is never previewed; its keys are listed instead.
    Masked,
    /// A path that leaves the project folder, or is not a file or folder.
    Refused,
    Other(String),
}

impl FsError {
    fn from_io(err: &std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound,
            std::io::ErrorKind::PermissionDenied => Self::Denied,
            _ => Self::Other(err.to_string()),
        }
    }

    /// A sentence for `what` ("this folder", "compose.yaml").
    #[must_use]
    pub fn describe(&self, what: &str) -> String {
        match self {
            Self::NotFound => format!("{what} is not on this Mac"),
            Self::Denied => format!("Berth can't read {what}. Allow it in System Settings, Privacy & Security, Files and Folders."),
            Self::Masked => format!("{what} holds secrets, so its text is not shown"),
            Self::Refused => format!("{what} is outside the project folder"),
            Self::Other(message) => format!("Could not read {what}: {message}"),
        }
    }
}

/// Reads a project folder. Paths are relative to the folder, `/`-separated;
/// the empty string is the folder itself.
pub trait ProjectFs: Send + Sync {
    /// The names in one folder, in tab order.
    fn list(&self, rel: &str) -> Result<DirListing, FsError>;
    /// The variable names of an env file in file order, each once. Never values.
    fn env_keys(&self, rel: &str) -> Result<Vec<String>, FsError>;
    /// The value of one variable of an env file, read when the user reveals it.
    fn env_value(&self, rel: &str, key: &str) -> Result<Option<String>, FsError>;
    /// The start of a text file. Refused for an env file.
    fn preview(&self, rel: &str) -> Result<Preview, FsError>;
    /// True when every read is already in memory, so a caller may read on
    /// the spot instead of on a blocking thread.
    fn instant(&self) -> bool {
        false
    }
}

/// A scene or test hands this to the Files tab in place of the folder on disk.
#[derive(Clone)]
pub struct ProjectFsOverride(pub Arc<dyn ProjectFs>);

/// The `rel` path as a list of names, or `None` when it leaves the folder.
fn parts(rel: &str) -> Option<Vec<&str>> {
    let mut out = Vec::new();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            other if other.contains('\\') || other.contains('\0') => return None,
            other => out.push(other),
        }
    }
    Some(out)
}

/// `a/b` + `c` as `a/b/c`.
#[must_use]
pub fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// The file name at the end of `rel`.
#[must_use]
pub fn file_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

fn is_hidden(name: &str) -> bool {
    matches!(name, ".git" | ".DS_Store")
}

/// The variable on one line of an env file: its name and raw value text.
/// Comments, blank lines and lines without `=` give `None`.
#[must_use]
pub fn parse_env_line(line: &str) -> Option<(&str, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").map_or(line, str::trim_start);
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let mut chars = key.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return None;
    }
    let value = value.trim();
    let value = match value.chars().next() {
        Some(quote @ ('"' | '\'')) => value[1..]
            .find(quote)
            .map_or(&value[1..], |end| &value[1..=end])
            .to_string(),
        _ => value
            .split_once(" #")
            .map_or(value, |(before, _)| before)
            .trim_end()
            .to_string(),
    };
    Some((key, value))
}

/// The names an env file defines, in file order, each once.
#[must_use]
pub fn env_keys_of(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    text.lines()
        .filter_map(parse_env_line)
        .filter(|(key, _)| seen.insert((*key).to_string()))
        .map(|(key, _)| key.to_string())
        .collect()
}

/// The value the last assignment of `key` gives, as Compose reads an env file.
#[must_use]
pub fn env_value_of(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(parse_env_line)
        .filter(|(k, _)| *k == key)
        .map(|(_, value)| value)
        .next_back()
}

fn preview_of(bytes: &[u8], size: u64) -> Preview {
    let probe = &bytes[..bytes.len().min(BINARY_PROBE)];
    if probe.contains(&0) {
        return Preview::Binary { bytes: size };
    }
    let text = String::from_utf8_lossy(bytes);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let read_all = size <= bytes.len() as u64;
    let total_lines = lines.len();
    let truncated = !read_all || total_lines > PREVIEW_MAX_LINES;
    lines.truncate(PREVIEW_MAX_LINES);
    Preview::Text {
        lines,
        total_lines,
        bytes: size,
        truncated,
    }
}

/// A real folder on this Mac.
#[derive(Debug, Clone)]
pub struct DiskFs {
    root: PathBuf,
}

impl DiskFs {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The path of `rel` under the folder, refusing `..`, absolute paths and
    /// anything that is not a plain name chain.
    fn resolve(&self, rel: &str) -> Result<PathBuf, FsError> {
        let names = parts(rel).ok_or(FsError::Refused)?;
        let mut path = self.root.clone();
        for name in names {
            let candidate = Path::new(name);
            if candidate.components().count() != 1
                || !matches!(candidate.components().next(), Some(Component::Normal(_)))
            {
                return Err(FsError::Refused);
            }
            path.push(name);
        }
        Ok(path)
    }

    /// The text of a file, at most [`PREVIEW_MAX_BYTES`] of it, and its size.
    fn read_head(&self, rel: &str) -> Result<(Vec<u8>, u64), FsError> {
        let path = self.resolve(rel)?;
        let meta = std::fs::symlink_metadata(&path).map_err(|e| FsError::from_io(&e))?;
        if !meta.is_file() {
            return Err(FsError::Refused);
        }
        let mut head = Vec::new();
        std::fs::File::open(&path)
            .and_then(|file| file.take(PREVIEW_MAX_BYTES).read_to_end(&mut head))
            .map_err(|e| FsError::from_io(&e))?;
        Ok((head, meta.len()))
    }
}

impl ProjectFs for DiskFs {
    fn list(&self, rel: &str) -> Result<DirListing, FsError> {
        let dir = self.resolve(rel)?;
        let mut entries = Vec::new();
        for item in std::fs::read_dir(&dir).map_err(|e| FsError::from_io(&e))? {
            let Ok(item) = item else { continue };
            let name = item.file_name().to_string_lossy().into_owned();
            if is_hidden(&name) {
                continue;
            }
            let Ok(file_type) = item.file_type() else {
                continue;
            };
            let kind = if file_type.is_symlink() {
                EntryKind::Link
            } else if file_type.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            };
            let size = (kind == EntryKind::File)
                .then(|| item.metadata().ok().map(|m| m.len()))
                .flatten();
            entries.push(FsEntry {
                rel: join_rel(rel.trim_matches('/'), &name),
                role: if kind == EntryKind::Dir {
                    FileRole::Other
                } else {
                    classify(&name)
                },
                name,
                kind,
                size,
            });
        }
        Ok(DirListing::from_entries(entries))
    }

    fn env_keys(&self, rel: &str) -> Result<Vec<String>, FsError> {
        let (head, _) = self.read_head(rel)?;
        Ok(env_keys_of(&String::from_utf8_lossy(&head)))
    }

    fn env_value(&self, rel: &str, key: &str) -> Result<Option<String>, FsError> {
        let (head, _) = self.read_head(rel)?;
        Ok(env_value_of(&String::from_utf8_lossy(&head), key))
    }

    fn preview(&self, rel: &str) -> Result<Preview, FsError> {
        if classify(file_name(rel)) == FileRole::Env {
            return Err(FsError::Masked);
        }
        let (head, size) = self.read_head(rel)?;
        Ok(preview_of(&head, size))
    }
}

/// A folder held in memory: the Files tab of a scene or a test.
#[derive(Debug, Clone, Default)]
pub struct MemoryFs {
    files: BTreeMap<String, String>,
    dirs: BTreeSet<String>,
}

impl MemoryFs {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A file at `rel` with `text`; its folders exist too.
    #[must_use]
    pub fn file(mut self, rel: &str, text: &str) -> Self {
        let rel = rel.trim_matches('/').to_string();
        let mut dir = String::new();
        let names: Vec<&str> = rel.split('/').collect();
        for name in &names[..names.len().saturating_sub(1)] {
            dir = join_rel(&dir, name);
            self.dirs.insert(dir.clone());
        }
        self.files.insert(rel, text.to_string());
        self
    }

    /// An empty folder at `rel`.
    #[must_use]
    pub fn dir(mut self, rel: &str) -> Self {
        self.dirs.insert(rel.trim_matches('/').to_string());
        self
    }

    fn text(&self, rel: &str) -> Result<&String, FsError> {
        let names = parts(rel).ok_or(FsError::Refused)?;
        self.files.get(&names.join("/")).ok_or(FsError::NotFound)
    }
}

impl ProjectFs for MemoryFs {
    fn list(&self, rel: &str) -> Result<DirListing, FsError> {
        let dir = parts(rel).ok_or(FsError::Refused)?.join("/");
        if !dir.is_empty() && !self.dirs.contains(&dir) {
            return Err(FsError::NotFound);
        }
        let child = |path: &str| {
            let name = if dir.is_empty() {
                path
            } else {
                path.strip_prefix(&format!("{dir}/"))?
            };
            (!name.is_empty() && !name.contains('/')).then(|| name.to_string())
        };
        let mut entries: Vec<FsEntry> = self
            .dirs
            .iter()
            .filter_map(|path| {
                child(path).map(|name| FsEntry {
                    rel: path.clone(),
                    name,
                    kind: EntryKind::Dir,
                    role: FileRole::Other,
                    size: None,
                })
            })
            .collect();
        entries.extend(self.files.iter().filter_map(|(path, text)| {
            child(path).map(|name| FsEntry {
                rel: path.clone(),
                role: classify(&name),
                name,
                kind: EntryKind::File,
                size: Some(text.len() as u64),
            })
        }));
        Ok(DirListing::from_entries(entries))
    }

    fn env_keys(&self, rel: &str) -> Result<Vec<String>, FsError> {
        Ok(env_keys_of(self.text(rel)?))
    }

    fn env_value(&self, rel: &str, key: &str) -> Result<Option<String>, FsError> {
        Ok(env_value_of(self.text(rel)?, key))
    }

    fn preview(&self, rel: &str) -> Result<Preview, FsError> {
        if classify(file_name(rel)) == FileRole::Env {
            return Err(FsError::Masked);
        }
        let text = self.text(rel)?;
        Ok(preview_of(text.as_bytes(), text.len() as u64))
    }

    fn instant(&self) -> bool {
        true
    }
}

/// The folder a project's Files tab reads: its folder on disk.
#[must_use]
pub fn fs_for(project: &Project) -> Option<Arc<dyn ProjectFs>> {
    let folder = project.folder.clone()?;
    Some(Arc::new(DiskFs::new(folder)))
}

/// The selected project, from the state's selection.
#[must_use]
pub fn selected_project(state: &crate::state::AppState) -> Option<Project> {
    use dioxus::prelude::ReadableExt;
    let crate::state::Selection::Project(id) = &*state.selection.peek() else {
        return None;
    };
    project_by_id(&state.projects.peek(), id).cloned()
}

fn project_by_id<'a>(projects: &'a [Project], id: &ProjectId) -> Option<&'a Project> {
    projects.iter().find(|p| p.id == *id)
}

// ---------------------------------------------------------------------
// Compose validation messages
// ---------------------------------------------------------------------

/// What `docker compose config` said about a Compose file that does not load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeProblem {
    /// The line in the file, when Compose named one.
    pub line: Option<usize>,
    /// Compose's own words without the path and the `yaml:` prefix.
    pub message: String,
    /// The file Compose named, as a name inside the folder.
    pub file: Option<String>,
}

impl ComposeProblem {
    /// "line 4: mapping values are not allowed in this context".
    #[must_use]
    pub fn text(&self) -> String {
        match self.line {
            Some(line) => format!("line {line}: {}", self.message),
            None => self.message.clone(),
        }
    }
}

/// Read the line and message out of Compose's error text. `folder` is cut
/// from any path the text names.
#[must_use]
pub fn parse_compose_problem(raw: &str, folder: &Path) -> ComposeProblem {
    let folder_text = format!("{}/", folder.to_string_lossy());
    let cleaned = raw.replace(&folder_text, "");
    let lines: Vec<&str> = cleaned
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut file = None;
    let mut line_no = None;
    let mut message = None;
    for line in &lines {
        let mut rest = *line;
        if let Some(after) = rest.strip_prefix("validating ") {
            if let Some((name, tail)) = after.split_once(": ") {
                file = Some(name.trim().to_string());
                rest = tail;
            }
        }
        let rest = rest.strip_prefix("yaml: ").unwrap_or(rest);
        if line_no.is_none() {
            if let Some(after) = rest.strip_prefix("line ") {
                let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
                if let Ok(n) = digits.parse::<usize>() {
                    line_no = Some(n);
                    let tail = after[digits.len()..].trim_start_matches(':').trim();
                    message = Some(if tail.is_empty() { rest } else { tail }.to_string());
                    continue;
                }
            }
        }
        if message.is_none() && !rest.starts_with("unmarshal errors") {
            message = Some(rest.to_string());
        }
    }
    let mut message = message.unwrap_or_else(|| "The Compose file does not load".to_string());
    if message.chars().count() > 300 {
        message = message.chars().take(300).collect::<String>() + "\u{2026}";
    }
    ComposeProblem {
        line: line_no,
        message,
        file,
    }
}

// ---------------------------------------------------------------------
// Handing the folder to other apps
// ---------------------------------------------------------------------

/// A program and its arguments, built from `SearchPath` and run by [`launch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCommand {
    pub program: PathBuf,
    pub args: Vec<std::ffi::OsString>,
}

/// Looks a program up by name: `|name| search_path.find(name)` in the app,
/// a fixed answer in a test.
pub type Finder<'a> = &'a dyn Fn(&str) -> Option<PathBuf>;

fn open_with(find: Finder<'_>, args: Vec<std::ffi::OsString>) -> Option<OpenCommand> {
    Some(OpenCommand {
        program: find("open")?,
        args,
    })
}

/// `open <folder>`: the folder in Finder.
#[must_use]
pub fn open_folder_command(find: Finder<'_>, folder: &Path) -> Option<OpenCommand> {
    open_with(find, vec![folder.as_os_str().to_owned()])
}

/// `open -R <path>`: Finder with the item selected.
#[must_use]
pub fn reveal_command(find: Finder<'_>, path: &Path) -> Option<OpenCommand> {
    open_with(find, vec!["-R".into(), path.as_os_str().to_owned()])
}

/// `open -a <terminal> <folder>`: a terminal in the folder. Terminal.app
/// unless Settings names another app.
#[must_use]
pub fn terminal_command(
    find: Finder<'_>,
    terminal_app: Option<&str>,
    folder: &Path,
) -> Option<OpenCommand> {
    let app = terminal_app
        .filter(|a| !a.trim().is_empty())
        .unwrap_or("Terminal");
    open_with(
        find,
        vec!["-a".into(), app.into(), folder.as_os_str().to_owned()],
    )
}

/// The editor Open in editor uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    /// The binary, or `None` for the default app (`open <file>`).
    pub program: Option<PathBuf>,
    /// "Visual Studio Code (detected)", "the default app"…
    pub label: String,
}

/// The editor: the one Settings names, else VS Code (`code`) when it is on
/// the search path, else the default app.
#[must_use]
pub fn editor_for(find: Finder<'_>, setting: Option<&str>) -> Editor {
    if let Some(name) = setting.map(str::trim).filter(|n| !n.is_empty()) {
        if let Some(program) = find(name) {
            return Editor {
                program: Some(program),
                label: name.to_string(),
            };
        }
    }
    match find("code") {
        Some(program) => Editor {
            program: Some(program),
            label: "Visual Studio Code (detected)".to_string(),
        },
        None => Editor {
            program: None,
            label: "the default app".to_string(),
        },
    }
}

/// Open `file` in the editor, with `folder` as its workspace when the editor
/// is a command-line one. The default app gets `open <file>`.
#[must_use]
pub fn open_in_editor_command(
    find: Finder<'_>,
    setting: Option<&str>,
    folder: &Path,
    file: &Path,
) -> Option<OpenCommand> {
    match editor_for(find, setting).program {
        Some(program) => Some(OpenCommand {
            program,
            args: vec![folder.as_os_str().to_owned(), file.as_os_str().to_owned()],
        }),
        None => open_with(find, vec![file.as_os_str().to_owned()]),
    }
}

/// The file Open in editor opens when none is chosen: the project's first
/// Compose file, else the folder itself.
#[must_use]
pub fn default_editor_target(compose_files: &[PathBuf], folder: &Path) -> Option<PathBuf> {
    Some(
        compose_files
            .first()
            .cloned()
            .unwrap_or_else(|| folder.to_path_buf()),
    )
}

/// The finder the app uses: `berth_sys::SearchPath` (D30).
pub fn search_path_finder() -> impl Fn(&str) -> Option<PathBuf> {
    let search = berth_sys::SearchPath::from_process();
    move |name| search.find(name)
}

/// Run `command` without waiting for the app it opens. A failure is logged,
/// never raised: the click had no result to show.
pub fn launch(command: OpenCommand) {
    let work = async move {
        let result = berth_sys::run_capture(
            &command.program,
            &command.args,
            berth_sys::SpawnOpts {
                timeout: Some(std::time::Duration::from_secs(10)),
                ..Default::default()
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
        if let Err(err) = result {
            tracing::warn!(%err, program = %command.program.display(), "open failed");
        }
    };
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(work);
    } else {
        dioxus::prelude::spawn(work);
    }
}

#[cfg(test)]
#[path = "project_files_tests.rs"]
mod tests;
