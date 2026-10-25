//! The Files tab's tree as plain data (P5-T6): what has been read, what is
//! open, and the rows that follow from them. Pure, so the order of the rows
//! is tested without a window. Rows come out in the listing's own order,
//! which is stable (`services::project_files`), so a row never moves under
//! the pointer (AD31): opening a folder or an env file adds rows under it
//! and moves none that were there.

use std::collections::{BTreeMap, BTreeSet};

use crate::services::project_files::{DirListing, EntryKind, FileRole, FsEntry, FsError, Preview};

/// One read: under way, done, or failed.
#[derive(Debug, Clone, PartialEq)]
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(FsError),
}

impl<T> Load<T> {
    pub fn from_result(result: Result<T, FsError>) -> Self {
        match result {
            Ok(value) => Self::Ready(value),
            Err(err) => Self::Failed(err),
        }
    }
}

/// Everything the tree has read so far, keyed by path from the folder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TreeData {
    pub listings: BTreeMap<String, Load<DirListing>>,
    pub keys: BTreeMap<String, Load<Vec<String>>>,
    pub previews: BTreeMap<String, Load<Preview>>,
    /// Folders and env files that are open.
    pub expanded: BTreeSet<String>,
    /// The file shown on the right.
    pub selected: Option<String>,
    /// The first listing has been used to choose the file and open the first env file.
    pub seeded: bool,
}

/// One line of the tree.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeRow {
    Entry {
        entry: FsEntry,
        depth: usize,
        expanded: bool,
        selected: bool,
        /// How many keys an open env file has, once read.
        key_count: Option<usize>,
    },
    Key {
        file: String,
        key: String,
        depth: usize,
    },
    Loading {
        depth: usize,
    },
    Failed {
        depth: usize,
        message: String,
    },
    /// A folder with more names than the tab lists.
    More {
        depth: usize,
        count: usize,
    },
    Empty {
        depth: usize,
    },
}

/// Does opening this entry show something under it?
#[must_use]
pub fn opens(entry: &FsEntry) -> bool {
    entry.kind == EntryKind::Dir || entry.role == FileRole::Env
}

/// The rows under `dir`, depth first, in listing order.
#[must_use]
pub fn tree_rows(data: &TreeData) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    push_dir(data, "", 0, &mut rows);
    rows
}

fn push_dir(data: &TreeData, dir: &str, depth: usize, rows: &mut Vec<TreeRow>) {
    match data.listings.get(dir) {
        None | Some(Load::Loading) => rows.push(TreeRow::Loading { depth }),
        Some(Load::Failed(err)) => rows.push(TreeRow::Failed {
            depth,
            message: err.describe(if dir.is_empty() { "this folder" } else { dir }),
        }),
        Some(Load::Ready(listing)) => {
            if listing.entries.is_empty() {
                rows.push(TreeRow::Empty { depth });
            }
            for entry in &listing.entries {
                let expanded = opens(entry) && data.expanded.contains(&entry.rel);
                let key_count = match (entry.role, data.keys.get(&entry.rel)) {
                    (FileRole::Env, Some(Load::Ready(keys))) => Some(keys.len()),
                    _ => None,
                };
                rows.push(TreeRow::Entry {
                    entry: entry.clone(),
                    depth,
                    expanded,
                    selected: data.selected.as_deref() == Some(entry.rel.as_str()),
                    key_count,
                });
                if !expanded {
                    continue;
                }
                if entry.kind == EntryKind::Dir {
                    push_dir(data, &entry.rel, depth + 1, rows);
                } else {
                    push_keys(data, &entry.rel, depth + 1, rows);
                }
            }
            if listing.more > 0 {
                rows.push(TreeRow::More {
                    depth,
                    count: listing.more,
                });
            }
        }
    }
}

fn push_keys(data: &TreeData, file: &str, depth: usize, rows: &mut Vec<TreeRow>) {
    match data.keys.get(file) {
        None | Some(Load::Loading) => rows.push(TreeRow::Loading { depth }),
        Some(Load::Failed(err)) => rows.push(TreeRow::Failed {
            depth,
            message: err.describe(file),
        }),
        Some(Load::Ready(keys)) if keys.is_empty() => rows.push(TreeRow::Empty { depth }),
        Some(Load::Ready(keys)) => {
            for key in keys {
                rows.push(TreeRow::Key {
                    file: file.to_string(),
                    key: key.clone(),
                    depth,
                });
            }
        }
    }
}

/// The file the tab opens on: the project's first Compose file when it lies
/// inside the folder, else the first Compose file of the listing, else the
/// first plain file.
#[must_use]
pub fn initial_selection(
    folder: Option<&std::path::Path>,
    compose_files: &[std::path::PathBuf],
    root: Option<&DirListing>,
) -> Option<String> {
    let from_project = folder.and_then(|folder| {
        compose_files.iter().find_map(|file| {
            let rel = file.strip_prefix(folder).ok()?;
            let rel = rel.to_string_lossy().into_owned();
            (!rel.is_empty()).then_some(rel)
        })
    });
    from_project.or_else(|| {
        let entries = &root?.entries;
        entries
            .iter()
            .find(|e| e.role == FileRole::Compose)
            .or_else(|| {
                entries
                    .iter()
                    .find(|e| e.kind == EntryKind::File && e.role != FileRole::Env)
            })
            .map(|e| e.rel.clone())
    })
}

/// The first env file of a listing: the one the tab opens by default, as the
/// frame shows `.env` open with its keys.
#[must_use]
pub fn first_env(listing: &DirListing) -> Option<&FsEntry> {
    listing.entries.iter().find(|e| e.role == FileRole::Env)
}
