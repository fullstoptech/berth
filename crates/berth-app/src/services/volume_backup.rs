//! Back up and restore a volume (P4-T4, D51).
//!
//! A backup reads the volume's tar stream through the helper container,
//! drops the helper's mount folder from every path (so the archive holds
//! the volume's own files, and a restore puts them back where they were),
//! and writes a `.tar.gz` beside the place the owner chose, renaming it into
//! place only when whole. It never stops a container: a database volume is
//! copied as it is, which the confirm text says. A restore reads the archive
//! whole (at most [`MAX_RESTORE_BYTES`] unpacked), counts what is in it for
//! the confirm, and uploads it into the volume's root.

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::services::files::FileSource;

/// The folder the helper mounts the volume on; the engine names the archive
/// root after it.
pub const MOUNT_DIR: &str = "__berth_vol";

/// The most a restore unpacks into memory.
pub const MAX_RESTORE_BYTES: u64 = 1024 * 1024 * 1024;

/// The path of an archive entry without the helper's mount folder; `None`
/// for the folder itself (nothing to store).
#[must_use]
pub fn strip_mount_prefix(path: &Path) -> Option<PathBuf> {
    let mut parts = path.components().filter(|c| {
        !matches!(
            c,
            std::path::Component::CurDir | std::path::Component::RootDir
        )
    });
    let first = parts.next()?;
    let rest: PathBuf = if first.as_os_str() == MOUNT_DIR {
        parts.collect()
    } else {
        std::iter::once(first).chain(parts).collect()
    };
    (!rest.as_os_str().is_empty()).then_some(rest)
}

/// What a finished backup holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupReport {
    /// Regular files stored.
    pub files: u64,
    /// Size of the `.tar.gz` on disk.
    pub bytes: u64,
}

fn io_text(what: &str, err: &dyn std::fmt::Display) -> String {
    format!("{what}: {err}")
}

fn sibling(dest: &Path, tag: &str) -> PathBuf {
    let name = dest.file_name().map_or_else(
        || "backup".to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    dest.with_file_name(format!(".{name}.{tag}"))
}

/// Write the volume behind `source` to `dest` as a `.tar.gz`.
///
/// # Errors
/// What went wrong, in words; `dest` is left alone (no partial file).
pub async fn backup(
    source: Arc<dyn FileSource>,
    dest: PathBuf,
    cancel: CancellationToken,
) -> Result<BackupReport, String> {
    let raw = sibling(&dest, "berth-tar");
    let part = sibling(&dest, "berth-part");
    let outcome = backup_via(&source, &dest, &raw, &part, &cancel).await;
    let _ = tokio::fs::remove_file(&raw).await;
    if outcome.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    outcome
}

async fn backup_via(
    source: &Arc<dyn FileSource>,
    dest: &Path,
    raw: &Path,
    part: &Path,
    cancel: &CancellationToken,
) -> Result<BackupReport, String> {
    // 1. The engine's tar stream to a scratch file.
    let mut file = tokio::fs::File::create(raw)
        .await
        .map_err(|e| io_text("Could not write the backup", &e))?;
    let mut stream = source.download("/", cancel.clone());
    loop {
        let chunk = tokio::select! {
            () = cancel.cancelled() => return Err("Cancelled".to_string()),
            chunk = stream.next() => chunk,
        };
        match chunk {
            Some(Ok(chunk)) => file
                .write_all(&chunk)
                .await
                .map_err(|e| io_text("Could not write the backup", &e))?,
            Some(Err(err)) => return Err(format!("Could not read the volume: {err}")),
            None => break,
        }
    }
    file.flush()
        .await
        .map_err(|e| io_text("Could not write the backup", &e))?;
    drop(file);
    if cancel.is_cancelled() {
        return Err("Cancelled".to_string());
    }
    // 2. Re-pack without the mount folder, gzipped.
    let (raw_path, part_path) = (raw.to_path_buf(), part.to_path_buf());
    let files = tokio::task::spawn_blocking(move || repack(&raw_path, &part_path))
        .await
        .map_err(|e| io_text("Could not pack the backup", &e))??;
    tokio::fs::rename(part, dest)
        .await
        .map_err(|e| io_text("Could not save the backup", &e))?;
    let bytes = tokio::fs::metadata(dest).await.map_or(0, |m| m.len());
    Ok(BackupReport { files, bytes })
}

/// Copy the entries of the tar at `raw` into a gzipped tar at `out`,
/// without the mount folder. Returns the number of regular files.
fn repack(raw: &Path, out: &Path) -> Result<u64, String> {
    let input = std::fs::File::open(raw).map_err(|e| io_text("Could not read the volume", &e))?;
    let mut archive = tar::Archive::new(BufReader::new(input));
    let output =
        std::fs::File::create(out).map_err(|e| io_text("Could not write the backup", &e))?;
    let gz = flate2::write::GzEncoder::new(BufWriter::new(output), flate2::Compression::default());
    let mut builder = tar::Builder::new(gz);
    let mut files = 0_u64;
    let entries = archive
        .entries()
        .map_err(|e| io_text("The volume's archive is damaged", &e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| io_text("The volume's archive is damaged", &e))?;
        let path = entry
            .path()
            .map_err(|e| io_text("The volume's archive is damaged", &e))?
            .into_owned();
        let Some(relative) = strip_mount_prefix(&path) else {
            continue;
        };
        let mut header = entry.header().clone();
        let kind = header.entry_type();
        let packed = if kind.is_hard_link() || kind.is_symlink() {
            let link = entry
                .link_name()
                .map_err(|e| io_text("The volume's archive is damaged", &e))?
                .map(std::borrow::Cow::into_owned)
                .unwrap_or_default();
            // A hard link names another entry, which lost its prefix too.
            let link = if kind.is_hard_link() {
                strip_mount_prefix(&link).unwrap_or(link)
            } else {
                link
            };
            builder.append_link(&mut header, &relative, &link)
        } else {
            builder.append_data(&mut header, &relative, &mut entry)
        };
        packed.map_err(|e| io_text("Could not pack the backup", &e))?;
        if kind.is_file() {
            files += 1;
        }
    }
    let gz = builder
        .into_inner()
        .map_err(|e| io_text("Could not pack the backup", &e))?;
    let mut writer = gz
        .finish()
        .map_err(|e| io_text("Could not pack the backup", &e))?;
    writer
        .flush()
        .map_err(|e| io_text("Could not write the backup", &e))?;
    Ok(files)
}

// ---------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------

/// What a backup file holds, for the confirm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInfo {
    /// Regular files.
    pub files: u64,
    /// Their total size unpacked.
    pub bytes: u64,
}

/// A backup file read and checked: the count for the confirm and the tar
/// to upload.
#[derive(Debug, Clone)]
pub struct Backup {
    pub info: ArchiveInfo,
    pub tar: Bytes,
}

/// Read a `.tar.gz` (or a plain `.tar`) and count what is in it.
///
/// # Errors
/// That the file is not an archive, is too big, or cannot be read.
pub async fn read_backup(path: PathBuf) -> Result<Backup, String> {
    tokio::task::spawn_blocking(move || read_backup_blocking(&path))
        .await
        .map_err(|e| io_text("Could not read the file", &e))?
}

fn read_backup_blocking(path: &Path) -> Result<Backup, String> {
    let mut file = std::fs::File::open(path).map_err(|e| io_text("Could not open the file", &e))?;
    let mut magic = [0_u8; 2];
    let gzipped = file.read(&mut magic).is_ok_and(|n| n == 2) && magic == [0x1f, 0x8b];
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(0))
        .map_err(|e| io_text("Could not read the file", &e))?;
    let reader: Box<dyn Read> = if gzipped {
        Box::new(flate2::read::GzDecoder::new(BufReader::new(file)))
    } else {
        Box::new(BufReader::new(file))
    };
    let mut tar = Vec::new();
    reader
        .take(MAX_RESTORE_BYTES + 1)
        .read_to_end(&mut tar)
        .map_err(|_| "That file is not a backup Berth can read".to_string())?;
    if tar.len() as u64 > MAX_RESTORE_BYTES {
        return Err(format!(
            "That backup is larger than {} unpacked",
            berth_core::fmt::bytes(bytesize::ByteSize::b(MAX_RESTORE_BYTES))
        ));
    }
    let info = count_entries(&tar)?;
    Ok(Backup {
        info,
        tar: Bytes::from(tar),
    })
}

fn count_entries(tar: &[u8]) -> Result<ArchiveInfo, String> {
    let not_a_backup = || "That file is not a tar backup".to_string();
    let mut archive = tar::Archive::new(std::io::Cursor::new(tar));
    let (mut files, mut bytes, mut seen) = (0_u64, 0_u64, 0_u64);
    for entry in archive.entries().map_err(|_| not_a_backup())? {
        let entry = entry.map_err(|_| not_a_backup())?;
        seen += 1;
        if entry.header().entry_type().is_file() {
            files += 1;
            bytes += entry.header().size().unwrap_or(0);
        }
    }
    if seen == 0 {
        return Err("That backup is empty".to_string());
    }
    Ok(ArchiveInfo { files, bytes })
}

/// Unpack `tar` into the root of the volume behind `source`.
///
/// # Errors
/// The engine's message.
pub async fn restore(source: Arc<dyn FileSource>, tar: Bytes) -> Result<(), String> {
    source
        .upload("/", tar)
        .await
        .map_err(|e| format!("Could not restore: {e}"))
}

#[cfg(test)]
#[path = "volume_backup_tests.rs"]
mod tests;
