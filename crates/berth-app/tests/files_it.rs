#![allow(clippy::expect_used)]
//! The Files tab's by-hand check as a test (P3-T8 done-when): Files of
//! `berthfx-web-1` lists `/usr/share/nginx/html`; downloading `index.html`
//! through the service and the tar extraction gives the bytes `docker exec
//! berthfx-web-1 cat /usr/share/nginx/html/index.html` prints; a small file
//! uploaded to `/tmp/berthfx-upload.txt` reads back.
//!
//! Run with `BERTH_IT=1 cargo nextest run -p berth-app --run-ignored only
//! files_it`. Only the fixture container `berthfx-web-1` is read, and the
//! one file written into its `/tmp` is removed again on every exit path.

mod common;

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use berth_app::services::files::{
    list_dir, load_local, read_file, save_local, write_file, EngineFiles, FileSource, FilesError,
};
use berth_engine::EngineClient;

const WEB: &str = "berthfx-web-1";
const UPLOADED: &str = "/tmp/berthfx-upload.txt";

fn docker(path: &std::path::Path, host: &str, args: &[&str]) -> std::process::Output {
    std::process::Command::new(path)
        .env("DOCKER_HOST", host)
        .args(args)
        .output()
        .expect("docker CLI runs")
}

/// Removes the uploaded file when the test ends, whatever happened.
struct Uploaded {
    docker: std::path::PathBuf,
    host: String,
}

impl Drop for Uploaded {
    fn drop(&mut self) {
        let _ = docker(
            &self.docker,
            &self.host,
            &["exec", WEB, "rm", "-f", UPLOADED],
        );
    }
}

#[tokio::test]
#[ignore = "needs the live Colima engine; run with BERTH_IT=1"]
async fn files_it_list_download_and_upload_on_berthfx_web() {
    if std::env::var("BERTH_IT").is_err() {
        eprintln!("BERTH_IT not set, skipping");
        return;
    }
    common::settle_fixture().await;

    let search = berth_sys::SearchPath::from_process()
        .with_login_shell()
        .await;
    let colima_cli = berth_engine::ColimaCli::new(search.clone());
    let engines = berth_engine::discovery::discover(&colima_cli, &[]).await;
    let engine = engines
        .iter()
        .find(|e| e.id.0 == "colima:default" && e.state == berth_core::EngineState::Running)
        .expect("colima:default running")
        .clone();
    let host = engine.docker_host.clone().expect("docker host");
    let docker_bin = search
        .find("docker")
        .unwrap_or_else(|| std::path::PathBuf::from("docker"));

    let client: Arc<dyn EngineClient> =
        Arc::new(berth_engine::BollardEngine::connect(&engine).expect("connect"));
    let web = client
        .containers()
        .await
        .expect("containers")
        .into_iter()
        .find(|c| c.name == WEB)
        .expect("the fixture web container");
    assert_eq!(
        web.labels.get("dev.berth.fixture").map(String::as_str),
        Some("1"),
        "refusing to write into a container without the fixture label"
    );
    let _cleanup = Uploaded {
        docker: docker_bin.clone(),
        host: host.clone(),
    };
    let source: Arc<dyn FileSource> = Arc::new(EngineFiles::new(Arc::clone(&client), web.id));

    // Files of berthfx-web-1 lists /usr/share/nginx/html.
    let html = list_dir(
        Arc::clone(&source),
        "/usr/share/nginx/html".to_string(),
        CancellationToken::new(),
    )
    .await
    .expect("list the nginx html folder");
    let names: Vec<&str> = html.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"index.html"), "{names:?}");
    assert!(names.contains(&"50x.html"), "{names:?}");
    let index = html
        .iter()
        .find(|e| e.name == "index.html")
        .expect("index.html");
    assert_eq!(index.kind, berth_core::FileKind::File);
    assert!(index.size.is_some_and(|s| s.0 > 0), "{index:?}");
    assert!(index.modified.is_some(), "{index:?}");

    // The root lists too (the engine's own list_files cannot).
    let root = list_dir(
        Arc::clone(&source),
        "/".to_string(),
        CancellationToken::new(),
    )
    .await
    .expect("list the root");
    let root_names: Vec<&str> = root.iter().map(|e| e.name.as_str()).collect();
    for expected in ["etc", "usr", "tmp"] {
        assert!(root_names.contains(&expected), "{root_names:?}");
    }

    // A path that is not there says so.
    let missing = list_dir(
        Arc::clone(&source),
        "/berthfx-no-such-folder".to_string(),
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(missing, FilesError::NotFound(_)), "{missing:?}");

    // Download index.html: identical to `docker exec ... cat`.
    let expected = docker(
        &docker_bin,
        &host,
        &["exec", WEB, "cat", "/usr/share/nginx/html/index.html"],
    );
    assert!(expected.status.success(), "docker exec cat failed");
    let downloaded = read_file(
        Arc::clone(&source),
        "/usr/share/nginx/html/index.html".to_string(),
        CancellationToken::new(),
    )
    .await
    .expect("download index.html");
    assert!(!downloaded.is_empty());
    assert_eq!(
        downloaded, expected.stdout,
        "bytes differ from docker exec cat"
    );
    // And through a file on disk, the way the save panel's path is written.
    let dir = tempfile::tempdir().expect("tempdir");
    let saved = dir.path().join("index.html");
    save_local(saved.clone(), downloaded.clone())
        .await
        .expect("save");
    assert_eq!(std::fs::read(&saved).expect("read saved"), expected.stdout);

    // Upload a small file to /tmp, then read it back.
    let contents = b"uploaded by the berth files test\n".to_vec();
    let local = dir.path().join("berthfx-upload.txt");
    std::fs::write(&local, &contents).expect("write local");
    let loaded = load_local(local).await.expect("load local");
    write_file(
        Arc::clone(&source),
        "/tmp".to_string(),
        "berthfx-upload.txt".to_string(),
        loaded,
        chrono::Utc::now(),
    )
    .await
    .expect("upload");
    let back = read_file(
        Arc::clone(&source),
        UPLOADED.to_string(),
        CancellationToken::new(),
    )
    .await
    .expect("read the upload back");
    assert_eq!(back, contents);
    let via_exec = docker(&docker_bin, &host, &["exec", WEB, "cat", UPLOADED]);
    assert_eq!(via_exec.stdout, contents, "docker exec cat sees the upload");

    // Uploading again overwrites.
    write_file(
        source.clone(),
        "/tmp".to_string(),
        "berthfx-upload.txt".to_string(),
        b"second\n".to_vec(),
        chrono::Utc::now(),
    )
    .await
    .expect("overwrite");
    let again = read_file(source, UPLOADED.to_string(), CancellationToken::new())
        .await
        .expect("read again");
    assert_eq!(again, b"second\n");
}
