//! Tests for the Files tab (P3-T8): the states of the browser (empty,
//! large folder, read error), the tar in and out, the paths, and the
//! panel end to end on an in-memory container.

use std::cell::RefCell;
use std::sync::Arc;

use berth_core::{FileEntry, FileKind};
use chrono::{TimeZone, Utc};
use dioxus::prelude::*;
use tokio_util::sync::CancellationToken;

use super::{item_count, modified_text_in, size_text, thousands, FileBrowser, Listing};
use crate::services::confirm;
use crate::services::files::{
    self, breadcrumbs, build_upload_tar, extract_file, join_path, list_dir, normalize_dir,
    parent_dir, read_file, write_file, FileSource, FilesBackend, FilesError, MemoryFiles,
};
use crate::state::use_app_state;
use crate::ui::files_panel::{start_dir, FilesPanel};

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

/// One archive entry the way Docker writes it: the name as given, even with
/// a leading slash. `None` data makes a folder.
fn tar_of(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, data) in entries {
        let mut header = tar::Header::new_gnu();
        let bytes = name.as_bytes();
        if let Some(gnu) = header.as_gnu_mut() {
            gnu.name[..bytes.len()].copy_from_slice(bytes);
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

fn names(entries: &[FileEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.name.as_str()).collect()
}

fn entry(name: &str, kind: FileKind, size: Option<u64>) -> FileEntry {
    FileEntry {
        name: name.to_string(),
        kind,
        size: size.map(bytesize::ByteSize::b),
        modified: Utc.timestamp_opt(1_700_000_000, 0).single(),
    }
}

#[component]
fn Host(dir: String, listing: Listing, busy: Option<String>, notice: Option<String>) -> Element {
    rsx! {
        FileBrowser {
            dir,
            listing,
            listing_key: 1_u64,
            busy,
            notice,
            on_navigate: move |_: String| {},
            on_download: move |_: String| {},
            on_upload: move |()| {},
            on_refresh: move |()| {},
        }
    }
}

fn render(dir: &str, listing: Listing) -> String {
    render_with(dir, listing, None, None)
}

fn render_with(dir: &str, listing: Listing, busy: Option<&str>, notice: Option<&str>) -> String {
    let mut dom = VirtualDom::new_with_props(
        Host,
        HostProps {
            dir: dir.to_string(),
            listing,
            busy: busy.map(str::to_string),
            notice: notice.map(str::to_string),
        },
    );
    let _ = dom.rebuild_to_vec();
    markup(&dom)
}

/// The page without its `<style>` text, so a class name in the CSS does not
/// count as an element.
fn markup(dom: &VirtualDom) -> String {
    let html = dioxus_ssr::render(dom);
    let mut out = String::new();
    let mut rest = html.as_str();
    while let Some(start) = rest.find("<style>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</style>") {
            Some(end) => rest = &rest[start + end + "</style>".len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------
// The browser's states
// ---------------------------------------------------------------------

#[test]
fn file_browser_empty_folder_says_so_and_offers_upload() {
    let html = render("/data", Listing::Ready(Vec::new()));
    assert!(html.contains("This folder is empty"), "{html}");
    assert!(html.contains("0 items"), "{html}");
    assert!(html.contains("Upload"), "{html}");
    assert!(!html.contains("vt-row"), "no rows for no entries: {html}");
}

#[test]
fn file_browser_large_folder_renders_only_a_window_of_rows() {
    let entries: Vec<FileEntry> = (0..5_000)
        .map(|i| entry(&format!("file-{i:04}.log"), FileKind::File, Some(1_000 + i)))
        .collect();
    let html = render("/var/log", Listing::Ready(entries));
    let rows = html.matches("vt-row").count();
    assert!(rows > 0 && rows <= 60, "rendered {rows} rows");
    assert!(html.contains("file-0000.log"), "{html}");
    assert!(!html.contains("file-4999.log"), "{html}");
    assert!(html.contains("5,000 items"), "{html}");
    assert!(html.contains("aria-rowcount=\"5000\""), "{html}");
}

#[test]
fn file_browser_read_error_shows_the_engine_words_and_retry() {
    let html = render(
        "/root",
        Listing::Failed("permission denied while reading /root".to_string()),
    );
    assert!(
        html.contains("Could not read /root: permission denied while reading /root"),
        "{html}"
    );
    assert!(html.contains("Retry"), "{html}");
    assert!(!html.contains("This folder is empty"), "{html}");
    assert!(!html.contains("vt-row"), "{html}");
}

#[test]
fn file_browser_loading_state_names_the_folder() {
    let html = render("/etc", Listing::Loading);
    assert!(html.contains("Reading /etc"), "{html}");
    assert!(html.contains("aria-busy=\"true\""), "{html}");
}

#[test]
fn file_browser_rows_show_name_size_and_modified_with_folders_openable() {
    let html = render(
        "/usr/share/nginx/html",
        Listing::Ready(vec![
            entry("assets", FileKind::Dir, None),
            entry("index.html", FileKind::File, Some(896)),
            entry("latest", FileKind::Link, None),
        ]),
    );
    // A folder is a button; a file is plain text with a download button.
    assert!(html.contains("class=\"fb-open\""), "{html}");
    assert!(html.contains(">assets<"), "{html}");
    assert!(html.contains(">index.html<"), "{html}");
    assert!(html.contains("896 B"), "{html}");
    assert!(html.contains("Download index.html"), "{html}");
    assert!(
        !html.contains("Download assets"),
        "no download for a folder"
    );
    // A link is listed, tagged, and its download is disabled with a reason.
    assert!(html.contains(">link<"), "{html}");
    assert!(html.contains("Links cannot be downloaded"), "{html}");
    assert!(html.contains("3 items"), "{html}");
    // Breadcrumbs: root and each folder.
    for crumb in ["usr", "share", "nginx", "html"] {
        assert!(html.contains(&format!(">{crumb}<")), "{crumb}: {html}");
    }
}

#[test]
fn file_browser_busy_and_notice_are_shown() {
    let html = render_with(
        "/",
        Listing::Ready(vec![entry("a", FileKind::File, Some(1))]),
        Some("Downloading a\u{2026}"),
        Some("Could not download a: disk full"),
    );
    assert!(html.contains("Downloading a"), "{html}");
    assert!(html.contains("Could not download a: disk full"), "{html}");
    // A transfer in flight blocks a second one at the button's reason.
    assert!(
        !html.contains("1 item<"),
        "busy text replaces the count: {html}"
    );
}

#[test]
fn file_browser_up_is_disabled_at_the_root() {
    let at_root = render("/", Listing::Ready(Vec::new()));
    let below = render("/etc", Listing::Ready(Vec::new()));
    assert!(at_root.contains("Already at the top"), "{at_root}");
    assert!(below.contains("Up to /"), "{below}");
}

// ---------------------------------------------------------------------
// Formatting and paths
// ---------------------------------------------------------------------

#[test]
fn file_browser_counts_use_thousands_separators() {
    assert_eq!(thousands(0), "0");
    assert_eq!(thousands(999), "999");
    assert_eq!(thousands(1_000), "1,000");
    assert_eq!(thousands(1_234_567), "1,234,567");
    assert_eq!(item_count(1), "1 item");
    assert_eq!(item_count(2), "2 items");
}

#[test]
fn file_browser_modified_reads_by_year() {
    let now = Utc.with_ymd_and_hms(2026, 10, 25, 12, 0, 0).single();
    let now = now.expect("a date");
    let this_year = Utc.with_ymd_and_hms(2026, 3, 7, 14, 3, 0).single();
    let last_year = Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 0).single();
    assert_eq!(modified_text_in(this_year, now, &Utc), "Mar 7 14:03");
    assert_eq!(modified_text_in(last_year, now, &Utc), "Dec 31 2025");
    assert_eq!(modified_text_in(None, now, &Utc), "\u{2014}");
}

#[test]
fn file_browser_size_text_is_a_dash_for_folders() {
    assert_eq!(size_text(&entry("d", FileKind::Dir, None)), "\u{2014}");
    assert_eq!(size_text(&entry("f", FileKind::File, Some(2048))), "2 KB");
}

#[test]
fn file_browser_paths_normalize_join_and_climb() {
    assert_eq!(normalize_dir(""), "/");
    assert_eq!(normalize_dir("usr//share/./nginx/"), "/usr/share/nginx");
    assert_eq!(normalize_dir("/a/b/../c"), "/a/c");
    assert_eq!(normalize_dir("/../.."), "/");
    assert_eq!(join_path("/", "etc"), "/etc");
    assert_eq!(join_path("/etc", "hosts"), "/etc/hosts");
    assert_eq!(parent_dir("/usr/share"), "/usr");
    assert_eq!(parent_dir("/usr"), "/");
    assert_eq!(parent_dir("/"), "/");
    let trail: Vec<(String, String)> = breadcrumbs("/usr/share")
        .into_iter()
        .map(|c| (c.label, c.path))
        .collect();
    assert_eq!(
        trail,
        [
            ("/".to_string(), "/".to_string()),
            ("usr".to_string(), "/usr".to_string()),
            ("share".to_string(), "/usr/share".to_string()),
        ]
    );
}

#[test]
fn file_browser_start_dir_is_the_first_mounted_folder() {
    let mount = |dest: &str| berth_core::Mount {
        kind: berth_core::MountKind::Volume,
        source: "src".to_string(),
        destination: dest.to_string(),
        read_only: false,
        volume_name: None,
    };
    assert_eq!(start_dir(&[]), "/");
    assert_eq!(
        start_dir(&[mount("/var/run/docker.sock"), mount("/var/lib/data/")]),
        "/var/lib/data"
    );
}

// ---------------------------------------------------------------------
// Download: the file out of the tar
// ---------------------------------------------------------------------

#[test]
fn file_browser_extract_file_returns_the_exact_bytes() {
    let content: Vec<u8> = (0..=255).cycle().take(5_000).collect();
    let tar = tar_of(&[("blob.bin", Some(&content))]);
    assert_eq!(extract_file(&tar, "blob.bin").expect("bytes"), content);
}

#[test]
fn file_browser_extract_file_of_an_empty_file_is_empty() {
    let tar = tar_of(&[("empty", Some(b""))]);
    assert_eq!(
        extract_file(&tar, "empty").expect("bytes"),
        Vec::<u8>::new()
    );
}

#[test]
fn file_browser_extract_file_picks_the_named_entry() {
    let tar = tar_of(&[
        ("dir/", None),
        ("dir/other.txt", Some(b"other")),
        ("dir/wanted.txt", Some(b"wanted")),
    ]);
    // The archive of a file holds one entry named by its basename; a longer
    // path in the name never matches it.
    assert!(matches!(
        extract_file(&tar, "wanted.txt"),
        Err(FilesError::NotFound(_))
    ));
    let single = tar_of(&[("wanted.txt", Some(b"wanted"))]);
    assert_eq!(
        extract_file(&single, "wanted.txt").expect("bytes"),
        b"wanted"
    );
}

#[test]
fn file_browser_extract_file_refuses_a_folder_and_a_missing_name() {
    let tar = tar_of(&[("html/", None), ("html/index.html", Some(b"x"))]);
    let err = extract_file(&tar, "html").unwrap_err();
    assert_eq!(
        err,
        FilesError::NotAFile {
            name: "html".to_string(),
            kind: "folder"
        }
    );
    assert_eq!(err.to_string(), "html is a folder, not a file");
    assert!(matches!(
        extract_file(&tar, "nope"),
        Err(FilesError::NotFound(_))
    ));
}

// ---------------------------------------------------------------------
// Upload: the tar built in memory
// ---------------------------------------------------------------------

#[test]
fn file_browser_upload_tar_round_trips_the_bytes() {
    let content: Vec<u8> = (0..=255).rev().cycle().take(3_000).collect();
    let modified = Utc
        .timestamp_opt(1_700_000_123, 0)
        .single()
        .expect("a date");
    let tar = build_upload_tar("berthfx-upload.txt", &content, modified).expect("tar");
    assert_eq!(
        extract_file(&tar, "berthfx-upload.txt").expect("bytes"),
        content
    );
    // And the header says what the engine will write.
    let mut archive = tar::Archive::new(std::io::Cursor::new(&tar));
    let entry = archive
        .entries()
        .expect("entries")
        .next()
        .expect("one entry")
        .expect("entry");
    assert_eq!(entry.header().size().expect("size"), 3_000);
    assert_eq!(entry.header().mode().expect("mode"), 0o644);
    assert_eq!(entry.header().mtime().expect("mtime"), 1_700_000_123);
    assert_eq!(entry.header().entry_type(), tar::EntryType::Regular);
}

#[test]
fn file_browser_upload_tar_of_nothing_is_an_empty_file() {
    let tar = build_upload_tar("empty.txt", b"", Utc::now()).expect("tar");
    assert_eq!(
        extract_file(&tar, "empty.txt").expect("bytes"),
        Vec::<u8>::new()
    );
}

#[test]
fn file_browser_upload_tar_refuses_names_that_could_escape_the_folder() {
    for bad in ["", ".", "..", "a/b", "/etc/passwd", "../x", "nul\0name"] {
        assert_eq!(
            build_upload_tar(bad, b"x", Utc::now()).unwrap_err(),
            FilesError::InvalidName(bad.to_string()),
            "{bad:?}"
        );
    }
}

// ---------------------------------------------------------------------
// The confirm
// ---------------------------------------------------------------------

#[test]
fn file_browser_upload_effect_reads_the_same_in_button_title_and_result() {
    let size = bytesize::ByteSize::b(2_048);
    let plain = confirm::upload_effect("/tmp", "notes.txt", size, false);
    assert_eq!(confirm::button_label(&plain), "Upload file");
    assert_eq!(confirm::result_label(&plain, 1), "Uploaded 1 file");
    assert_eq!(
        confirm::upload_title("/tmp", "notes.txt"),
        "Upload notes.txt to /tmp?"
    );
    assert_eq!(plain.lines.len(), 1);
    assert_eq!(plain.lines[0].kind, berth_core::EffectKind::Writes);
    assert_eq!(plain.lines[0].text, "notes.txt (2 KB) to /tmp");
    assert_eq!(
        plain.bytes, None,
        "bytes would print as 'Removes' in the footer"
    );
    assert_eq!(plain.typed_name, None);

    // A file of that name is there: the dialog says it is overwritten.
    let over = confirm::upload_effect("/tmp", "notes.txt", size, true);
    assert_eq!(over.lines.len(), 2);
    assert_eq!(over.lines[1].kind, berth_core::EffectKind::Changes);
    assert!(
        over.lines[1].text.contains("overwritten"),
        "{:?}",
        over.lines
    );
}

// ---------------------------------------------------------------------
// The service on an in-memory container
// ---------------------------------------------------------------------

fn nginx() -> Arc<MemoryFiles> {
    Arc::new(
        MemoryFiles::new()
            .with_file(
                "/usr/share/nginx/html/index.html",
                b"<h1>Welcome</h1>\n",
                1_700_000_000,
            )
            .with_file(
                "/usr/share/nginx/html/50x.html",
                b"<h1>oops</h1>\n",
                1_700_000_100,
            )
            .with_file(
                "/usr/share/nginx/html/img/logo.png",
                b"\x89PNG",
                1_700_000_200,
            )
            .with_file("/etc/hosts", b"127.0.0.1 localhost\n", 1_700_000_300),
    )
}

#[tokio::test]
async fn file_browser_service_lists_a_folder_from_a_stream_in_small_chunks() {
    let source: Arc<dyn FileSource> = nginx();
    let listed = list_dir(
        source,
        "/usr/share/nginx/html".to_string(),
        CancellationToken::new(),
    )
    .await
    .expect("a listing");
    assert_eq!(names(&listed), ["img", "50x.html", "index.html"]);
}

#[tokio::test]
async fn file_browser_service_lists_the_root() {
    let source: Arc<dyn FileSource> = nginx();
    let listed = list_dir(source, "/".to_string(), CancellationToken::new())
        .await
        .expect("a listing");
    assert_eq!(names(&listed), ["etc", "usr"]);
}

#[tokio::test]
async fn file_browser_service_a_file_path_is_not_a_folder() {
    let source: Arc<dyn FileSource> = nginx();
    let err = list_dir(source, "/etc/hosts".to_string(), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err, FilesError::NotADirectory("/etc/hosts".to_string()));
}

#[tokio::test]
async fn file_browser_service_reports_a_missing_folder_in_the_engines_words() {
    let source: Arc<dyn FileSource> = nginx();
    let err = list_dir(source, "/nope".to_string(), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(err, FilesError::NotFound(_)), "{err:?}");
    assert!(err.to_string().contains("/nope"), "{err}");
}

#[tokio::test]
async fn file_browser_service_surfaces_an_engine_failure() {
    let source: Arc<dyn FileSource> = Arc::new(MemoryFiles::failing("engine went away"));
    let err = list_dir(source, "/".to_string(), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err, FilesError::Engine("engine went away".to_string()));
}

#[tokio::test]
async fn file_browser_service_a_cancelled_listing_is_cancelled() {
    let source: Arc<dyn FileSource> = nginx();
    let token = CancellationToken::new();
    token.cancel();
    let err = list_dir(source, "/".to_string(), token).await.unwrap_err();
    assert_eq!(err, FilesError::Cancelled);
}

#[tokio::test]
async fn file_browser_service_reads_a_file_byte_for_byte() {
    let source: Arc<dyn FileSource> = nginx();
    let bytes = read_file(
        source,
        "/usr/share/nginx/html/index.html".to_string(),
        CancellationToken::new(),
    )
    .await
    .expect("the file");
    assert_eq!(bytes, b"<h1>Welcome</h1>\n");
}

#[tokio::test]
async fn file_browser_service_will_not_read_a_folder_as_a_file() {
    let source: Arc<dyn FileSource> = nginx();
    let err = read_file(
        source,
        "/usr/share/nginx/html".to_string(),
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        err,
        FilesError::NotAFile {
            name: "html".to_string(),
            kind: "folder"
        }
    );
}

#[tokio::test]
async fn file_browser_service_upload_then_read_back_gives_the_same_bytes() {
    let memory = nginx();
    let source: Arc<dyn FileSource> = memory.clone();
    write_file(
        Arc::clone(&source),
        "/tmp".to_string(),
        "berthfx-upload.txt".to_string(),
        b"uploaded by berth".to_vec(),
        Utc::now(),
    )
    .await
    .expect("uploaded");
    assert_eq!(
        memory.contents("/tmp/berthfx-upload.txt"),
        Some(b"uploaded by berth".to_vec())
    );
    let back = read_file(
        source,
        "/tmp/berthfx-upload.txt".to_string(),
        CancellationToken::new(),
    )
    .await
    .expect("read back");
    assert_eq!(back, b"uploaded by berth");
}

#[tokio::test]
async fn file_browser_service_upload_overwrites_a_file_of_the_same_name() {
    let memory = nginx();
    let source: Arc<dyn FileSource> = memory.clone();
    write_file(
        source,
        "/etc".to_string(),
        "hosts".to_string(),
        b"10.0.0.1 db\n".to_vec(),
        Utc::now(),
    )
    .await
    .expect("uploaded");
    assert_eq!(
        memory.contents("/etc/hosts"),
        Some(b"10.0.0.1 db\n".to_vec())
    );
}

#[tokio::test]
async fn file_browser_service_upload_refuses_a_bad_name_before_any_engine_call() {
    let memory = nginx();
    let source: Arc<dyn FileSource> = memory.clone();
    let err = write_file(
        source,
        "/etc".to_string(),
        "../shadow".to_string(),
        b"x".to_vec(),
        Utc::now(),
    )
    .await
    .unwrap_err();
    assert_eq!(err, FilesError::InvalidName("../shadow".to_string()));
    assert_eq!(memory.contents("/shadow"), None);
}

#[tokio::test]
async fn file_browser_local_save_and_load_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("saved.bin");
    files::save_local(path.clone(), vec![1, 2, 3, 4])
        .await
        .expect("saved");
    assert_eq!(files::load_local(path).await.expect("loaded"), [1, 2, 3, 4]);
    let missing = files::load_local(dir.path().join("missing")).await;
    assert!(matches!(missing, Err(FilesError::Local(_))), "{missing:?}");
}

// ---------------------------------------------------------------------
// The panel, end to end
// ---------------------------------------------------------------------

thread_local! {
    static PANEL: RefCell<Option<(berth_core::Container, Arc<dyn FileSource>)>> =
        const { RefCell::new(None) };
}

#[component]
fn PanelHost() -> Element {
    let state = use_app_state();
    let (container, source) = PANEL
        .with(|slot| slot.borrow().clone())
        .expect("test config");
    let id = container.id.clone();
    use_hook(move || {
        let mut containers = state.containers;
        containers.set(vec![container]);
        provide_context(state);
        provide_context(FilesBackend(source));
    });
    rsx! { FilesPanel { container: id } }
}

fn container_with_mounts(mounts: Vec<berth_core::Mount>) -> berth_core::Container {
    let world = crate::services::mock_world_at(Utc::now());
    let mut container = world
        .containers
        .into_iter()
        .next()
        .expect("a mock container");
    container.mounts = mounts;
    container
}

/// Mount the panel on `source` and wait until it stops reading.
async fn panel_html(source: Arc<dyn FileSource>, mounts: Vec<berth_core::Mount>) -> String {
    PANEL.with(|slot| {
        *slot.borrow_mut() = Some((container_with_mounts(mounts), source));
    });
    let mut dom = VirtualDom::new(PanelHost);
    let _ = dom.rebuild_to_vec();
    let waited = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let html = markup(&dom);
            if !html.contains("Reading") {
                return html;
            }
            let _ = tokio::time::timeout(std::time::Duration::from_millis(50), dom.wait_for_work())
                .await;
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
        }
    })
    .await;
    waited.expect("the panel stopped reading")
}

fn volume_mount(dest: &str) -> berth_core::Mount {
    berth_core::Mount {
        kind: berth_core::MountKind::Volume,
        source: "vol".to_string(),
        destination: dest.to_string(),
        read_only: false,
        volume_name: Some("vol".to_string()),
    }
}

#[tokio::test]
async fn file_browser_panel_opens_on_the_mounted_folder_and_lists_it() {
    let html = panel_html(nginx(), vec![volume_mount("/usr/share/nginx/html")]).await;
    assert!(html.contains(">index.html<"), "{html}");
    assert!(html.contains(">50x.html<"), "{html}");
    assert!(html.contains(">img<"), "{html}");
    assert!(html.contains("3 items"), "{html}");
    assert!(html.contains("17 B"), "the size of index.html: {html}");
}

#[tokio::test]
async fn file_browser_panel_shows_a_read_error_with_the_engines_words() {
    let html = panel_html(Arc::new(MemoryFiles::failing("socket closed")), vec![]).await;
    assert!(html.contains("Could not read /: socket closed"), "{html}");
    assert!(html.contains("Retry"), "{html}");
}

#[tokio::test]
async fn file_browser_panel_shows_an_empty_folder() {
    let memory = MemoryFiles::new().with_dir("/data", 1_700_000_000);
    let html = panel_html(Arc::new(memory), vec![volume_mount("/data")]).await;
    assert!(html.contains("This folder is empty"), "{html}");
}

#[test]
fn file_browser_scene_mounts_the_panel_on_the_apps_mount() {
    let html = crate::scenes::render_scene_html("Files / panel").expect("scene renders");
    assert!(html.contains("shop-api-api-1"), "{html}");
    // The panel opens on the container's first mount and starts reading.
    assert!(html.contains("Reading /app"), "{html}");
    assert!(html.contains("aria-label=\"Files\""), "{html}");
}
