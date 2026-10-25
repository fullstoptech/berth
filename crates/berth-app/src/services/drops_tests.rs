use std::path::{Path, PathBuf};

use super::*;

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn touch(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "x").expect("write");
    path
}

#[test]
fn drops_compose_and_dockerfile_names_are_told_apart_from_other_files() {
    for name in [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.prod.yaml",
        "Compose.YAML",
    ] {
        assert!(is_compose_name(name), "{name}");
    }
    for name in [
        "composer.json",
        "compose.txt",
        "composed.yaml",
        "app.yaml",
        "notes.yml",
    ] {
        assert!(!is_compose_name(name), "{name}");
    }
    for name in [
        "Dockerfile",
        "dockerfile",
        "Dockerfile.dev",
        "api.Dockerfile",
    ] {
        assert!(is_dockerfile_name(name), "{name}");
    }
    for name in [
        "Dockerfiles",
        "docker-compose.yml",
        "README.md",
        "Dockerfile-old",
    ] {
        assert!(!is_dockerfile_name(name), "{name}");
    }
}

#[test]
fn drops_a_folder_stands_for_itself_and_a_compose_file_or_dockerfile_for_its_folder() {
    let root = tmp();
    let app = root.path().join("new-app");
    let api = root.path().join("api");
    let web = root.path().join("web");
    for dir in [&app, &api, &web] {
        std::fs::create_dir(dir).expect("dir");
    }
    let compose = touch(&api, "compose.yaml");
    let dockerfile = touch(&web, "Dockerfile");

    let dropped = classify(&[app.clone(), compose, dockerfile]);
    assert_eq!(dropped.folders, [app, api, web]);
    assert!(dropped.rejected.is_empty());
}

#[test]
fn drops_a_folder_dropped_with_its_own_file_is_opened_once() {
    let root = tmp();
    let compose = touch(root.path(), "docker-compose.yml");
    let dockerfile = touch(root.path(), "Dockerfile");
    let dropped = classify(&[root.path().to_path_buf(), compose, dockerfile]);
    assert_eq!(dropped.folders, [root.path().to_path_buf()]);
}

#[test]
fn drops_any_other_file_is_rejected_by_name_and_opens_nothing() {
    let root = tmp();
    let notes = touch(root.path(), "notes.txt");
    let package = touch(root.path(), "package.json");
    let dropped = classify(&[notes, package]);
    assert!(dropped.folders.is_empty());
    assert_eq!(dropped.rejected, ["notes.txt", "package.json"]);
}

#[test]
fn drops_a_path_that_does_not_exist_is_not_a_folder() {
    let dropped = classify(&[PathBuf::from("/nonexistent/berthfx/thing")]);
    assert!(dropped.folders.is_empty());
    assert_eq!(dropped.rejected, ["thing"]);
}

#[test]
fn drops_hover_text_names_the_folder_and_says_nothing_is_written() {
    let home = Path::new("/Users/me");
    let dropped = Dropped {
        folders: vec![PathBuf::from("/Users/me/Dev/side/new-app")],
        rejected: Vec::new(),
    };
    let text = hover_text(&dropped, home);
    assert_eq!(text.title, "Drop to open new-app");
    assert_eq!(text.detail, "~/Dev/side/new-app");
    assert_eq!(
        text.body,
        "Berth reads the folder and shows what it finds. Nothing is written yet."
    );
    assert!(!text.refuse);
}

#[test]
fn drops_hover_text_for_several_folders_counts_them_and_notes_skipped_files() {
    let home = Path::new("/Users/me");
    let dropped = Dropped {
        folders: vec![PathBuf::from("/Users/me/a"), PathBuf::from("/Users/me/b")],
        rejected: vec!["x.txt".to_string()],
    };
    let text = hover_text(&dropped, home);
    assert_eq!(text.title, "Drop to open 2 folders");
    assert_eq!(text.detail, "a, b");
    assert!(
        text.body.ends_with("One other file is skipped."),
        "{}",
        text.body
    );
}

#[test]
fn drops_hover_text_for_the_wrong_file_type_refuses() {
    let dropped = Dropped {
        folders: Vec::new(),
        rejected: vec!["notes.txt".to_string()],
    };
    let text = hover_text(&dropped, Path::new("/Users/me"));
    assert!(text.refuse);
    assert_eq!(text.title, "Berth cannot open notes.txt");
    assert_eq!(text.body, "Drop a folder, a Compose file or a Dockerfile.");
}
