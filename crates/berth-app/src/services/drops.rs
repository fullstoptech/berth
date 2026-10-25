//! What a drop on the window means (P5-T5, D37): which of the dropped paths are folders Berth can
//! open, what the overlay says while they hover, and nothing else. No I/O beyond `is_dir`.

use std::path::{Path, PathBuf};

use crate::services::folders::display_path;

/// The dropped paths sorted into folders to open and files Berth has no use for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dropped {
    /// Folders to open, in drop order, each once. A Compose file or a Dockerfile stands for the
    /// folder it sits in.
    pub folders: Vec<PathBuf>,
    /// File names that are neither (`notes.txt`).
    pub rejected: Vec<String>,
}

/// Whether `name` is a Compose file: `compose.yaml`, `docker-compose.yml`, `compose.prod.yaml` …
#[must_use]
pub fn is_compose_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let stem = lower
        .strip_suffix(".yaml")
        .or_else(|| lower.strip_suffix(".yml"));
    let Some(stem) = stem else {
        return false;
    };
    ["compose", "docker-compose"]
        .iter()
        .any(|base| stem == *base || stem.strip_prefix(base).is_some_and(|r| r.starts_with('.')))
}

/// Whether `name` is a Dockerfile: `Dockerfile`, `Dockerfile.dev`, `api.Dockerfile`.
#[must_use]
pub fn is_dockerfile_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "dockerfile" || lower.starts_with("dockerfile.") || lower.ends_with(".dockerfile")
}

/// Sort what was dropped. A path that is a directory is a folder; a Compose file or a Dockerfile
/// is the folder it is in; anything else is rejected by name.
#[must_use]
pub fn classify(paths: &[PathBuf]) -> Dropped {
    let mut out = Dropped::default();
    for path in paths {
        let folder = if path.is_dir() {
            Some(path.clone())
        } else {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if is_compose_name(&name) || is_dockerfile_name(&name) {
                path.parent().map(Path::to_path_buf)
            } else {
                out.rejected.push(name);
                None
            }
        };
        if let Some(folder) = folder {
            if !out.folders.contains(&folder) {
                out.folders.push(folder);
            }
        }
    }
    out
}

/// What the overlay says while files hover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverText {
    /// "Drop to open new-app".
    pub title: String,
    /// The folder's path under the title ("~/Dev/side/new-app"), or the names of several.
    pub detail: String,
    /// The sentence under it.
    pub body: String,
    /// True when nothing hovering can be opened: the overlay warns instead of inviting.
    pub refuse: bool,
}

/// The overlay's words for `dropped`. `home` shortens paths to `~/...`.
#[must_use]
pub fn hover_text(dropped: &Dropped, home: &Path) -> HoverText {
    let skipped = match dropped.rejected.len() {
        0 => String::new(),
        1 => " One other file is skipped.".to_string(),
        n => format!(" {n} other files are skipped."),
    };
    match dropped.folders.as_slice() {
        [] => HoverText {
            title: match dropped.rejected.as_slice() {
                [one] => format!("Berth cannot open {one}"),
                _ => "Berth cannot open these files".to_string(),
            },
            detail: dropped.rejected.join(", "),
            body: "Drop a folder, a Compose file or a Dockerfile.".to_string(),
            refuse: true,
        },
        [one] => HoverText {
            title: format!(
                "Drop to open {}",
                one.file_name()
                    .map_or_else(|| one.display().to_string(), |n| n.to_string_lossy().into_owned())
            ),
            detail: display_path(one, home),
            body: format!(
                "Berth reads the folder and shows what it finds. Nothing is written yet.{skipped}"
            ),
            refuse: false,
        },
        many => HoverText {
            title: format!("Drop to open {} folders", many.len()),
            detail: many
                .iter()
                .map(|f| {
                    f.file_name()
                        .map_or_else(|| f.display().to_string(), |n| n.to_string_lossy().into_owned())
                })
                .collect::<Vec<_>>()
                .join(", "),
            body: format!(
                "Berth reads each folder in turn and shows what it finds. Nothing is written yet.{skipped}"
            ),
            refuse: false,
        },
    }
}

#[cfg(test)]
#[path = "drops_tests.rs"]
mod tests;
