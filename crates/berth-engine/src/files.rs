//! Folder listings from the archive API (D51's fallback, PLAN §6.5
//! `list_files`). Docker answers a request for a folder with a tar of the
//! folder and everything below it, so a listing reads the stream and keeps
//! only the headers of the folder's direct children; payloads are skipped
//! as they arrive and the archive is never held whole.
//!
//! What Docker puts in the archive depends on the path asked for:
//! - a folder `/usr/share/nginx/html` is archived as `html/`, `html/index.html`,
//!   `html/img/`, `html/img/a.png`: the root is the basename, children sit
//!   one level below it;
//! - `/` has no basename: the root is `/` itself and its children are
//!   `/bin/`, `/bin/ls`, `/etc/`: children sit at the first level.

use std::io::Read;

use berth_core::{FileEntry, FileKind};
use bytes::Bytes;
use futures::StreamExt;

use crate::client::Bs;
use crate::EngineError;

/// `path` made absolute and tidy: a leading `/`, no empty, `.` or `..`
/// parts, no trailing `/`; the root is `/`.
pub(crate) fn normalize(path: &str) -> String {
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

fn parse_error(err: &std::io::Error) -> EngineError {
    EngineError::Parse {
        what: "archive listing".to_string(),
        message: err.to_string(),
    }
}

/// The direct children of `dir` (the path as the engine sees it), by name.
/// `shown` is how the caller names the folder in an error.
///
/// # Errors
/// The archive is damaged, or `dir` turns out to be a file or a link.
pub(crate) fn entries_from_reader(
    reader: impl Read,
    dir: &str,
    shown: &str,
) -> Result<Vec<FileEntry>, EngineError> {
    let dir = normalize(dir);
    let at_root = dir == "/";
    let base = dir.rsplit('/').next().unwrap_or("").to_string();
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|e| parse_error(&e))?;
    let mut out: Vec<FileEntry> = Vec::new();
    let mut root_kind: Option<FileKind> = None;
    for entry in entries {
        let entry = entry.map_err(|e| parse_error(&e))?;
        let path = entry.path().map_err(|e| parse_error(&e))?;
        let raw = path.to_string_lossy().into_owned();
        let parts: Vec<&str> = raw
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .collect();
        let header = entry.header();
        let kind = kind_of(header.entry_type());
        let name = if at_root {
            match parts.as_slice() {
                [name] => (*name).to_string(),
                _ => continue,
            }
        } else {
            match parts.as_slice() {
                [only] if *only == base => {
                    root_kind = Some(kind);
                    continue;
                }
                [first, name] if *first == base => (*name).to_string(),
                _ => continue,
            }
        };
        out.push(FileEntry {
            name,
            kind,
            size: (kind == FileKind::File)
                .then(|| bytesize::ByteSize::b(header.size().unwrap_or(0))),
            modified: header
                .mtime()
                .ok()
                .and_then(|t| chrono::DateTime::from_timestamp(i64::try_from(t).unwrap_or(0), 0)),
        });
    }
    if root_kind.is_some_and(|kind| kind != FileKind::Dir) {
        return Err(EngineError::Docker(format!("{shown} is not a folder")));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// A blocking `Read` over a channel of chunks, for the tar parser, which
/// reads synchronously. Ends when the sender is gone; a failed stream
/// surfaces as the stream's own error, kept in `failed`.
struct ChannelReader {
    rx: tokio::sync::mpsc::Receiver<Result<Bytes, EngineError>>,
    chunk: Bytes,
    failed: Option<EngineError>,
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.chunk.is_empty() {
            match self.rx.blocking_recv() {
                Some(Ok(chunk)) => self.chunk = chunk,
                Some(Err(err)) => {
                    let message = err.to_string();
                    self.failed = Some(err);
                    return Err(std::io::Error::other(message));
                }
                None => return Ok(0),
            }
        }
        let n = buf.len().min(self.chunk.len());
        buf[..n].copy_from_slice(&self.chunk[..n]);
        bytes::Buf::advance(&mut self.chunk, n);
        Ok(n)
    }
}

/// List `dir` from the tar stream `download` returned for it. The stream is
/// pumped through a small channel into the parser on a blocking thread, so
/// memory stays at a few chunks however large the folder is. Dropping the
/// future stops both ends (the caller's stream ends when its receiver does).
///
/// # Errors
/// The stream's own error, a damaged archive, or `dir` not being a folder.
pub(crate) async fn list_from_stream(
    mut stream: Bs<Bytes>,
    dir: String,
    shown: String,
) -> Result<Vec<FileEntry>, EngineError> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, EngineError>>(8);
    let pump = tokio::spawn(async move {
        while let Some(item) = stream.next().await {
            let failed = item.is_err();
            if tx.send(item).await.is_err() || failed {
                return;
            }
        }
    });
    let parsed = tokio::task::spawn_blocking(move || {
        let mut reader = ChannelReader {
            rx,
            chunk: Bytes::new(),
            failed: None,
        };
        let result = entries_from_reader(&mut reader, &dir, &shown);
        match (result, reader.failed.take()) {
            (_, Some(err)) => Err(err),
            (result, None) => result,
        }
    })
    .await;
    pump.abort();
    parsed.unwrap_or_else(|e| Err(EngineError::Docker(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One archive entry the way Docker writes it: the name as given, even
    /// with a leading slash. `None` data makes a folder.
    fn tar_of(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            if let Some(gnu) = header.as_gnu_mut() {
                gnu.name[..name.len()].copy_from_slice(name.as_bytes());
            }
            header.set_mode(0o644);
            header.set_mtime(1_700_000_000);
            match data {
                Some(data) => {
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_size(data.len() as u64);
                    header.set_cksum();
                    builder.append(&header, *data).expect("append file");
                }
                None => {
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_size(0);
                    header.set_cksum();
                    builder
                        .append(&header, std::io::empty())
                        .expect("append dir");
                }
            }
        }
        builder.into_inner().expect("tar")
    }

    fn list(tar: Vec<u8>, dir: &str) -> Result<Vec<FileEntry>, EngineError> {
        entries_from_reader(std::io::Cursor::new(tar), dir, dir)
    }

    fn names(entries: &[FileEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn list_files_root_lists_the_top_level_with_their_names() {
        // Docker's archive of `/`: a root entry, then everything absolute.
        let tar = tar_of(&[
            ("/", None),
            ("/.dockerenv", Some(b"")),
            ("/bin/", None),
            ("/bin/ls", Some(b"elf")),
            ("/etc/", None),
            ("/etc/hosts", Some(b"127.0.0.1")),
            ("/etc/nginx/", None),
            ("/etc/nginx/nginx.conf", Some(b"x")),
        ]);
        let listed = list(tar, "/").expect("a listing");
        assert_eq!(names(&listed), [".dockerenv", "bin", "etc"]);
        assert_eq!(listed[0].kind, FileKind::File);
        assert_eq!(listed[1].kind, FileKind::Dir);
        assert_eq!(listed[1].size, None);
    }

    #[test]
    fn list_files_one_level_down_lists_the_folders_children() {
        let tar = tar_of(&[
            ("usr/", None),
            ("usr/bin/", None),
            ("usr/bin/env", Some(b"elf")),
            ("usr/share/", None),
            ("usr/share/doc/", None),
            ("usr/lib", Some(b"not a folder, a file")),
        ]);
        let listed = list(tar, "/usr").expect("a listing");
        assert_eq!(names(&listed), ["bin", "lib", "share"]);
        assert_eq!(listed[1].size, Some(bytesize::ByteSize::b(20)));
    }

    #[test]
    fn list_files_nested_folder_skips_deeper_entries() {
        let tar = tar_of(&[
            ("html/", None),
            ("html/index.html", Some(b"<h1>hi</h1>")),
            ("html/img/", None),
            ("html/img/logo.png", Some(b"png")),
            ("html/50x.html", Some(b"")),
        ]);
        let listed = list(tar, "/usr/share/nginx/html").expect("a listing");
        assert_eq!(names(&listed), ["50x.html", "img", "index.html"]);
        assert_eq!(listed[2].size, Some(bytesize::ByteSize::b(11)));
        assert_eq!(
            listed[2].modified,
            chrono::DateTime::from_timestamp(1_700_000_000, 0)
        );
    }

    #[test]
    fn list_files_empty_folder_is_empty() {
        assert!(list(tar_of(&[("data/", None)]), "/data")
            .expect("a listing")
            .is_empty());
    }

    #[test]
    fn list_files_a_file_path_is_not_a_folder() {
        let err = list(tar_of(&[("hosts", Some(b"x"))]), "/etc/hosts").unwrap_err();
        assert_eq!(err.to_string(), "/etc/hosts is not a folder");
    }

    #[test]
    fn list_files_garbage_is_a_parse_error() {
        let err = list(vec![7_u8; 700], "/x").unwrap_err();
        assert!(matches!(err, EngineError::Parse { .. }), "{err:?}");
    }

    #[test]
    fn list_files_a_volume_folder_sits_under_the_helper_mount() {
        // A volume's `/` is the helper's mount point, an ordinary folder.
        let tar = tar_of(&[
            ("__berth_vol/", None),
            ("__berth_vol/berth-vol.txt", Some(b"in-the-volume")),
        ]);
        let listed = list(tar, "/__berth_vol").expect("a listing");
        assert_eq!(names(&listed), ["berth-vol.txt"]);
    }

    fn chunked(tar: Vec<u8>, size: usize) -> Bs<Bytes> {
        let items: Vec<Result<Bytes, EngineError>> = tar
            .chunks(size)
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect();
        futures::stream::iter(items).boxed()
    }

    #[tokio::test]
    async fn list_files_stream_in_small_chunks_gives_the_same_listing() {
        let tar = tar_of(&[
            ("/", None),
            ("/bin/", None),
            ("/bin/ls", Some(b"elf")),
            ("/etc/", None),
        ]);
        let listed = list_from_stream(chunked(tar, 7), "/".to_string(), "/".to_string())
            .await
            .expect("a listing");
        assert_eq!(names(&listed), ["bin", "etc"]);
    }

    #[tokio::test]
    async fn list_files_stream_error_is_the_streams_own_error() {
        let tar = tar_of(&[("d/", None), ("d/a", Some(&[1_u8; 4096]))]);
        let mut items: Vec<Result<Bytes, EngineError>> = tar
            .chunks(512)
            .take(3)
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect();
        items.push(Err(EngineError::NotFound("gone mid-read".to_string())));
        let err = list_from_stream(
            futures::stream::iter(items).boxed(),
            "/d".to_string(),
            "/d".to_string(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, EngineError::NotFound(m) if m == "gone mid-read"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn list_files_a_huge_folder_streams_without_being_held_whole() {
        // 512 MiB of payload from a generator that never holds more than one
        // chunk, then a stream that never ends; the listing finishes at the
        // archive's own end marker.
        const PAYLOAD: u64 = 512 * 1024 * 1024;
        const CHUNK: usize = 1024 * 1024;
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(PAYLOAD);
        header.set_mode(0o644);
        header.set_path("d/big.bin").expect("path");
        header.set_cksum();
        let mut root = tar::Header::new_gnu();
        root.set_entry_type(tar::EntryType::Directory);
        root.set_size(0);
        root.set_mode(0o755);
        root.set_path("d/").expect("path");
        root.set_cksum();
        let mut head = root.as_bytes().to_vec();
        head.extend_from_slice(header.as_bytes());
        let state = (Some(head), 0_u64);
        let stream = futures::stream::unfold(state, |(head, sent)| async move {
            if let Some(head) = head {
                return Some((Ok(Bytes::from(head)), (None, sent)));
            }
            if sent < PAYLOAD {
                return Some((
                    Ok(Bytes::from(vec![0_u8; CHUNK])),
                    (None, sent + CHUNK as u64),
                ));
            }
            if sent == PAYLOAD {
                // End of archive: two empty blocks.
                return Some((Ok(Bytes::from(vec![0_u8; 1024])), (None, sent + 1)));
            }
            // The stream never ends: a listing that waited for the whole
            // archive before parsing would hang here.
            std::future::pending().await
        })
        .boxed();
        let listed = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            list_from_stream(stream, "/d".to_string(), "/d".to_string()),
        )
        .await
        .expect("the listing finished at the end-of-archive marker")
        .expect("a listing");
        assert_eq!(names(&listed), ["big.bin"]);
        assert_eq!(listed[0].size, Some(bytesize::ByteSize::b(PAYLOAD)));
    }

    #[test]
    fn list_files_normalize_makes_paths_absolute_and_tidy() {
        assert_eq!(normalize(""), "/");
        assert_eq!(normalize("usr//share/./nginx/"), "/usr/share/nginx");
        assert_eq!(normalize("/a/b/../c"), "/a/c");
        assert_eq!(normalize("/../.."), "/");
    }
}
