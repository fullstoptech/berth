//! P4-T4 tests for backing up and restoring a volume, against a source
//! that speaks the archive API the way Docker does for the helper mount
//! (every path under `__berth_vol/`).

use super::*;

use berth_core::FileEntry;
use berth_engine::{Bs, EngineError};

use crate::services::files::FilesError;

/// A tar named the way the engine names a volume read through the helper.
fn engine_tar() -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut dir = tar::Header::new_gnu();
    dir.set_entry_type(tar::EntryType::Directory);
    dir.set_mode(0o700);
    dir.set_size(0);
    builder
        .append_data(&mut dir, "__berth_vol/", std::io::empty())
        .expect("root dir");
    let mut sub = tar::Header::new_gnu();
    sub.set_entry_type(tar::EntryType::Directory);
    sub.set_mode(0o700);
    sub.set_size(0);
    builder
        .append_data(&mut sub, "__berth_vol/base/", std::io::empty())
        .expect("sub dir");
    for (path, body) in [
        ("__berth_vol/PG_VERSION", b"17\n".as_slice()),
        ("__berth_vol/base/1259", b"relation".as_slice()),
    ] {
        let mut file = tar::Header::new_gnu();
        file.set_size(body.len() as u64);
        file.set_mode(0o600);
        file.set_mtime(1_700_000_000);
        builder.append_data(&mut file, path, body).expect("file");
    }
    let mut link = tar::Header::new_gnu();
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_size(0);
    builder
        .append_link(&mut link, "__berth_vol/current", "PG_VERSION")
        .expect("symlink");
    let mut hard = tar::Header::new_gnu();
    hard.set_entry_type(tar::EntryType::Link);
    hard.set_size(0);
    builder
        .append_link(
            &mut hard,
            "__berth_vol/PG_VERSION.copy",
            "__berth_vol/PG_VERSION",
        )
        .expect("hard link");
    builder.into_inner().expect("tar")
}

struct Volume {
    tar: Vec<u8>,
    fail: Option<String>,
    uploads: std::sync::Mutex<Vec<(String, Bytes)>>,
}

impl Volume {
    fn new(tar: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            tar,
            fail: None,
            uploads: std::sync::Mutex::new(Vec::new()),
        })
    }
    fn failing(message: &str) -> Arc<Self> {
        Arc::new(Self {
            tar: Vec::new(),
            fail: Some(message.to_string()),
            uploads: std::sync::Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl FileSource for Volume {
    async fn list(&self, _dir: &str) -> Result<Vec<FileEntry>, FilesError> {
        Ok(Vec::new())
    }

    fn download(&self, _path: &str, _cancel: CancellationToken) -> Bs<Bytes> {
        let items: Vec<Result<Bytes, EngineError>> = match &self.fail {
            Some(message) => vec![Err(EngineError::Docker(message.clone()))],
            // Small chunks, so a header is split across reads.
            None => self
                .tar
                .chunks(100)
                .map(|c| Ok(Bytes::copy_from_slice(c)))
                .collect(),
        };
        futures::stream::iter(items).boxed()
    }

    async fn upload(&self, dir: &str, tar: Bytes) -> Result<(), FilesError> {
        self.uploads
            .lock()
            .expect("lock")
            .push((dir.to_string(), tar));
        Ok(())
    }
}

/// (path, type, body) of every entry in a `.tar.gz`.
fn read_gz(path: &Path) -> Vec<(String, tar::EntryType, String)> {
    let file = std::fs::File::open(path).expect("backup file");
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    archive
        .entries()
        .expect("entries")
        .map(|e| {
            let mut e = e.expect("entry");
            let path = e.path().expect("path").to_string_lossy().into_owned();
            let kind = e.header().entry_type();
            let mut body = String::new();
            if kind.is_file() {
                e.read_to_string(&mut body).expect("body");
            }
            (path, kind, body)
        })
        .collect()
}

#[test]
fn volumes_strip_removes_the_helper_mount_folder_only() {
    for (given, want) in [
        ("__berth_vol/PG_VERSION", Some("PG_VERSION")),
        ("__berth_vol/base/1259", Some("base/1259")),
        ("__berth_vol/", None),
        ("__berth_vol", None),
        ("./PG_VERSION", Some("PG_VERSION")),
        ("PG_VERSION", Some("PG_VERSION")),
        ("data/__berth_vol/x", Some("data/__berth_vol/x")),
    ] {
        assert_eq!(
            strip_mount_prefix(Path::new(given)),
            want.map(PathBuf::from),
            "{given}"
        );
    }
}

#[tokio::test]
async fn volumes_backup_writes_a_tar_gz_of_the_volumes_own_files() {
    let dir = tempfile::tempdir().expect("dir");
    let dest = dir.path().join("pgdata.tar.gz");
    let report = backup(
        Volume::new(engine_tar()),
        dest.clone(),
        CancellationToken::new(),
    )
    .await
    .expect("backup");
    assert_eq!(report.files, 2);
    assert_eq!(report.bytes, std::fs::metadata(&dest).expect("meta").len());

    let entries = read_gz(&dest);
    let paths: Vec<&str> = entries.iter().map(|(p, _, _)| p.as_str()).collect();
    // The helper's folder is gone: the volume's files sit at the top.
    assert!(paths.contains(&"PG_VERSION"), "{paths:?}");
    assert!(paths.contains(&"base/1259"), "{paths:?}");
    assert!(
        paths.iter().all(|p| !p.contains("__berth_vol")),
        "{paths:?}"
    );
    let version = entries
        .iter()
        .find(|(p, _, _)| p == "PG_VERSION")
        .expect("file");
    assert_eq!(version.2, "17\n");
    // Links stay links, the hard link still pointing at an entry that exists.
    let symlink = entries
        .iter()
        .find(|(p, _, _)| p == "current")
        .expect("symlink");
    assert!(symlink.1.is_symlink());
    assert!(entries
        .iter()
        .any(|(p, k, _)| p == "PG_VERSION.copy" && k.is_hard_link()));
    let file = std::fs::File::open(&dest).expect("open");
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let hard_target = archive
        .entries()
        .expect("entries")
        .map(|e| e.expect("entry"))
        .find(|e| e.header().entry_type().is_hard_link())
        .and_then(|e| e.link_name().ok().flatten().map(|l| l.into_owned()));
    assert_eq!(hard_target, Some(PathBuf::from("PG_VERSION")));
}

#[tokio::test]
async fn volumes_backup_leaves_no_scratch_files_beside_the_result() {
    let dir = tempfile::tempdir().expect("dir");
    let dest = dir.path().join("b.tar.gz");
    backup(Volume::new(engine_tar()), dest, CancellationToken::new())
        .await
        .expect("backup");
    let left: Vec<String> = std::fs::read_dir(dir.path())
        .expect("dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["b.tar.gz"]);
}

#[tokio::test]
async fn volumes_backup_that_fails_leaves_no_file_and_keeps_an_older_one() {
    let dir = tempfile::tempdir().expect("dir");
    let dest = dir.path().join("b.tar.gz");
    std::fs::write(&dest, b"older backup").expect("seed");
    let err = backup(
        Volume::failing("helper image alpine:3 is not on this engine"),
        dest.clone(),
        CancellationToken::new(),
    )
    .await
    .expect_err("fails");
    assert!(err.contains("alpine:3"), "{err}");
    assert_eq!(std::fs::read(&dest).expect("older"), b"older backup");
    let count = std::fs::read_dir(dir.path()).expect("dir").count();
    assert_eq!(count, 1, "no .part files left");
}

#[tokio::test]
async fn volumes_backup_cancelled_writes_nothing() {
    let dir = tempfile::tempdir().expect("dir");
    let dest = dir.path().join("b.tar.gz");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let err = backup(Volume::new(engine_tar()), dest.clone(), cancel)
        .await
        .expect_err("cancelled");
    assert_eq!(err, "Cancelled");
    assert!(!dest.exists());
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);
}

#[tokio::test]
async fn volumes_backup_of_an_empty_volume_is_a_valid_empty_archive() {
    // Only the mount folder: nothing to store.
    let mut builder = tar::Builder::new(Vec::new());
    let mut dir_header = tar::Header::new_gnu();
    dir_header.set_entry_type(tar::EntryType::Directory);
    dir_header.set_size(0);
    builder
        .append_data(&mut dir_header, "__berth_vol/", std::io::empty())
        .expect("dir");
    let tar_bytes = builder.into_inner().expect("tar");
    let dir = tempfile::tempdir().expect("dir");
    let dest = dir.path().join("empty.tar.gz");
    let report = backup(
        Volume::new(tar_bytes),
        dest.clone(),
        CancellationToken::new(),
    )
    .await
    .expect("backup");
    assert_eq!(report.files, 0);
    assert!(read_gz(&dest).is_empty());
}

#[tokio::test]
async fn volumes_backup_then_read_back_counts_what_restore_will_write() {
    let dir = tempfile::tempdir().expect("dir");
    let dest = dir.path().join("pgdata.tar.gz");
    backup(
        Volume::new(engine_tar()),
        dest.clone(),
        CancellationToken::new(),
    )
    .await
    .expect("backup");
    let read = read_backup(dest).await.expect("reads");
    assert_eq!(
        read.info,
        ArchiveInfo {
            files: 2,
            bytes: 11
        }
    );
}

#[tokio::test]
async fn volumes_restore_uploads_the_archive_into_the_volume_root() {
    let dir = tempfile::tempdir().expect("dir");
    let dest = dir.path().join("pgdata.tar.gz");
    backup(
        Volume::new(engine_tar()),
        dest.clone(),
        CancellationToken::new(),
    )
    .await
    .expect("backup");
    let read = read_backup(dest).await.expect("reads");

    let target = Volume::new(Vec::new());
    restore(target.clone(), read.tar.clone())
        .await
        .expect("restore");
    let uploads = target.uploads.lock().expect("lock");
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].0, "/");
    // What went up is a plain tar of the volume's files, root-relative.
    let mut archive = tar::Archive::new(std::io::Cursor::new(uploads[0].1.clone()));
    let names: Vec<String> = archive
        .entries()
        .expect("entries")
        .map(|e| {
            e.expect("e")
                .path()
                .expect("p")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(names.contains(&"PG_VERSION".to_string()), "{names:?}");
}

#[tokio::test]
async fn volumes_restore_reads_a_plain_tar_too() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("plain.tar");
    std::fs::write(&path, engine_tar()).expect("write");
    let read = read_backup(path).await.expect("reads");
    assert_eq!(read.info.files, 2);
}

#[tokio::test]
async fn volumes_restore_refuses_files_that_are_not_archives() {
    let dir = tempfile::tempdir().expect("dir");
    let garbage = dir.path().join("notes.tar.gz");
    std::fs::write(&garbage, b"this is not an archive at all, just words").expect("write");
    let err = read_backup(garbage).await.expect_err("garbage");
    assert!(err.contains("not a"), "{err}");

    let empty = dir.path().join("empty.tar");
    std::fs::write(
        &empty,
        tar::Builder::new(Vec::new()).into_inner().expect("tar"),
    )
    .expect("write");
    assert_eq!(
        read_backup(empty).await.expect_err("empty"),
        "That backup is empty"
    );

    let missing = dir.path().join("gone.tar.gz");
    assert!(read_backup(missing)
        .await
        .expect_err("missing")
        .starts_with("Could not open"));
}
