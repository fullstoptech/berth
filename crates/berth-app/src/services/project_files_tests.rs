//! P5-T6 tests for `services::project_files`: the listing order, the env
//! rules (names only, no preview), paths that stay inside the folder, the
//! Compose error reader and the commands that hand the folder to other apps.

use std::path::{Path, PathBuf};

use super::*;

fn names(listing: &DirListing) -> Vec<&str> {
    listing.entries.iter().map(|e| e.name.as_str()).collect()
}

fn frame_folder() -> MemoryFs {
    MemoryFs::new()
        .file("package.json", "{}")
        .file("src/index.js", "x")
        .file("worker/Dockerfile", "FROM scratch")
        .file("Dockerfile", "FROM scratch")
        .file(".env.example", "A=")
        .file(".env", "A=1")
        .file("compose.yaml", "services: {}")
}

#[test]
fn project_files_listing_puts_compose_then_env_then_dockerfile_then_folders_then_files() {
    let listing = frame_folder().list("").expect("lists");
    assert_eq!(
        names(&listing),
        [
            "compose.yaml",
            ".env",
            ".env.example",
            "Dockerfile",
            "src",
            "worker",
            "package.json"
        ]
    );
}

#[test]
fn project_files_listing_order_does_not_depend_on_the_order_things_were_added() {
    let reversed = MemoryFs::new()
        .file("compose.yaml", "services: {}")
        .file(".env", "A=1")
        .file(".env.example", "A=")
        .file("Dockerfile", "FROM scratch")
        .file("worker/Dockerfile", "FROM scratch")
        .file("src/index.js", "x")
        .file("package.json", "{}");
    assert_eq!(
        reversed.list("").expect("lists"),
        frame_folder().list("").expect("lists")
    );
}

#[test]
fn project_files_listing_orders_names_without_regard_to_case_inside_a_group() {
    let fs = MemoryFs::new()
        .file("Zeta.txt", "")
        .file("alpha.txt", "")
        .file("Beta.txt", "")
        .dir("Docs")
        .dir("api");
    let listing = fs.list("").expect("lists");
    assert_eq!(
        names(&listing),
        ["api", "Docs", "alpha.txt", "Beta.txt", "Zeta.txt"]
    );
}

#[test]
fn project_files_listing_of_a_subfolder_has_paths_from_the_project_folder() {
    let listing = frame_folder().list("worker").expect("lists");
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].rel, "worker/Dockerfile");
    assert_eq!(listing.entries[0].role, FileRole::Dockerfile);
}

#[test]
fn project_files_listing_is_capped_and_says_how_many_are_left() {
    let mut fs = MemoryFs::new();
    for i in 0..(LIST_CAP + 7) {
        fs = fs.file(&format!("f{i:04}.txt"), "");
    }
    let listing = fs.list("").expect("lists");
    assert_eq!(listing.entries.len(), LIST_CAP);
    assert_eq!(listing.more, 7);
    assert_eq!(listing.entries[0].name, "f0000.txt");
}

#[test]
fn project_files_classify_names() {
    for (name, role) in [
        ("compose.yaml", FileRole::Compose),
        ("compose.yml", FileRole::Compose),
        ("docker-compose.yml", FileRole::Compose),
        ("compose.override.yaml", FileRole::Compose),
        ("Compose.YAML", FileRole::Compose),
        (".env", FileRole::Env),
        (".env.local", FileRole::Env),
        (".env.production", FileRole::Env),
        (".env.example", FileRole::EnvTemplate),
        (".env.sample", FileRole::EnvTemplate),
        ("Dockerfile", FileRole::Dockerfile),
        ("Dockerfile.dev", FileRole::Dockerfile),
        ("api.dockerfile", FileRole::Dockerfile),
        ("package.json", FileRole::Other),
        ("compose.md", FileRole::Other),
        ("environment", FileRole::Other),
    ] {
        assert_eq!(classify(name), role, "{name}");
    }
}

#[test]
fn project_files_env_keys_are_names_in_file_order_each_once() {
    let text = "# comment\n\nDATABASE_URL=postgres://u:p@db/x\nexport REDIS_URL = redis://r\nA=1\nA=2\nnot a line\n=nokey\n1BAD=x\nQUOTED=\"a b\"\n";
    assert_eq!(
        env_keys_of(text),
        ["DATABASE_URL", "REDIS_URL", "A", "QUOTED"]
    );
}

#[test]
fn project_files_env_value_is_the_last_assignment_without_quotes_or_comment() {
    let text = "A=1\nB=\"two words\"\nA=3 # note\nC='x # y'\nD=\n";
    assert_eq!(env_value_of(text, "A").as_deref(), Some("3"));
    assert_eq!(env_value_of(text, "B").as_deref(), Some("two words"));
    assert_eq!(env_value_of(text, "C").as_deref(), Some("x # y"));
    assert_eq!(env_value_of(text, "D").as_deref(), Some(""));
    assert_eq!(env_value_of(text, "MISSING"), None);
}

#[test]
fn project_files_env_files_answer_with_names_and_never_with_text() {
    let fs = MemoryFs::new().file(".env", "SECRET_TOKEN=hunter2-not-real\n");
    assert_eq!(fs.env_keys(".env").expect("keys"), ["SECRET_TOKEN"]);
    assert_eq!(
        fs.preview(".env"),
        Err(FsError::Masked),
        "the text of an env file is refused"
    );
    assert_eq!(
        fs.env_value(".env", "SECRET_TOKEN")
            .expect("value")
            .as_deref(),
        Some("hunter2-not-real")
    );
}

#[test]
fn project_files_disk_folder_lists_in_tab_order_and_hides_git() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join(".git")).expect("git");
    std::fs::create_dir_all(root.join("src")).expect("src");
    std::fs::write(root.join("compose.yaml"), "services: {}\n").expect("compose");
    std::fs::write(root.join(".env"), "KEY=value\n").expect("env");
    std::fs::write(root.join("Dockerfile"), "FROM scratch\n").expect("dockerfile");
    std::fs::write(root.join("notes.txt"), "hi\n").expect("notes");
    std::fs::write(root.join(".DS_Store"), "x").expect("ds");
    let fs = DiskFs::new(root);
    let listing = fs.list("").expect("lists");
    assert_eq!(
        names(&listing),
        ["compose.yaml", ".env", "Dockerfile", "src", "notes.txt"]
    );
    let compose = &listing.entries[0];
    assert_eq!(compose.size, Some(13));
    assert_eq!(compose.rel, "compose.yaml");
}

#[test]
fn project_files_disk_folder_refuses_paths_that_leave_the_folder() {
    let outer = tempfile::tempdir().expect("tempdir");
    let root = outer.path().join("project");
    std::fs::create_dir_all(&root).expect("project");
    std::fs::write(outer.path().join("secret.txt"), "outside\n").expect("secret");
    let fs = DiskFs::new(&root);
    for rel in ["../secret.txt", "a/../../secret.txt", "..", "x\\y"] {
        assert_eq!(fs.preview(rel), Err(FsError::Refused), "{rel}");
        assert_eq!(fs.list(rel), Err(FsError::Refused), "{rel}");
        assert_eq!(fs.env_keys(rel), Err(FsError::Refused), "{rel}");
    }
}

#[test]
fn project_files_disk_preview_reads_text_flags_binary_and_cuts_long_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::write(root.join("a.txt"), "one\ntwo\nthree").expect("a");
    std::fs::write(root.join("b.bin"), [0u8, 1, 2, 3]).expect("b");
    let long: String = (0..(PREVIEW_MAX_LINES + 50))
        .map(|i| format!("line {i}\n"))
        .collect();
    std::fs::write(root.join("long.txt"), &long).expect("long");
    let fs = DiskFs::new(root);
    assert_eq!(
        fs.preview("a.txt").expect("text"),
        Preview::Text {
            lines: vec!["one".into(), "two".into(), "three".into()],
            total_lines: 3,
            bytes: 13,
            truncated: false
        }
    );
    assert_eq!(
        fs.preview("b.bin").expect("bin"),
        Preview::Binary { bytes: 4 }
    );
    let Preview::Text {
        lines, truncated, ..
    } = fs.preview("long.txt").expect("long")
    else {
        panic!("text expected");
    };
    assert_eq!(lines.len(), PREVIEW_MAX_LINES);
    assert!(truncated);
}

#[test]
fn project_files_disk_reads_say_why_they_failed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fs = DiskFs::new(dir.path());
    assert_eq!(fs.list("missing"), Err(FsError::NotFound));
    assert_eq!(fs.preview("missing.txt"), Err(FsError::NotFound));
    // A folder is not a file.
    std::fs::create_dir(dir.path().join("sub")).expect("sub");
    assert_eq!(fs.preview("sub"), Err(FsError::Refused));
    assert!(FsError::Denied
        .describe("this folder")
        .contains("Files and Folders"));
}

#[cfg(unix)]
#[test]
fn project_files_disk_lists_a_symlink_as_a_link_and_does_not_read_through_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("real.txt"), "x").expect("real");
    std::os::unix::fs::symlink(dir.path().join("real.txt"), dir.path().join("link.txt"))
        .expect("symlink");
    let fs = DiskFs::new(dir.path());
    let listing = fs.list("").expect("lists");
    let link = listing
        .entries
        .iter()
        .find(|e| e.name == "link.txt")
        .expect("link");
    assert_eq!(link.kind, EntryKind::Link);
    assert_eq!(fs.preview("link.txt"), Err(FsError::Refused));
}

#[test]
fn project_files_disk_env_keys_come_from_the_file_and_values_only_on_ask() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(".env"), "A=one\nB=two\n").expect("env");
    let fs = DiskFs::new(dir.path());
    assert_eq!(fs.env_keys(".env").expect("keys"), ["A", "B"]);
    assert_eq!(fs.preview(".env"), Err(FsError::Masked));
    assert_eq!(
        fs.env_value(".env", "B").expect("value").as_deref(),
        Some("two")
    );
}

#[test]
fn project_files_compose_problem_reads_line_and_message() {
    let folder = Path::new("/home/me/shop");
    let problem = parse_compose_problem(
        "yaml: line 4: mapping values are not allowed in this context",
        folder,
    );
    assert_eq!(problem.line, Some(4));
    assert_eq!(
        problem.message,
        "mapping values are not allowed in this context"
    );
    assert_eq!(problem.file, None);
    assert_eq!(
        problem.text(),
        "line 4: mapping values are not allowed in this context"
    );
}

#[test]
fn project_files_compose_problem_names_the_file_and_drops_the_folder() {
    let folder = Path::new("/home/me/shop");
    let problem = parse_compose_problem(
        "validating /home/me/shop/compose.yaml: services.api.image must be a string\n",
        folder,
    );
    assert_eq!(problem.file.as_deref(), Some("compose.yaml"));
    assert_eq!(problem.line, None);
    assert_eq!(problem.message, "services.api.image must be a string");
}

#[test]
fn project_files_compose_problem_reads_the_unmarshal_form_and_falls_back() {
    let folder = Path::new("/x");
    let unmarshal = parse_compose_problem(
        "yaml: unmarshal errors:\n  line 3: cannot unmarshal !!str `x` into map",
        folder,
    );
    assert_eq!(unmarshal.line, Some(3));
    assert!(
        unmarshal.message.starts_with("cannot unmarshal"),
        "{unmarshal:?}"
    );
    let empty = parse_compose_problem("", folder);
    assert_eq!(empty.line, None);
    assert_eq!(empty.message, "The Compose file does not load");
}

fn finder<'a>(found: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<PathBuf> + 'a {
    move |name| {
        found
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, path)| PathBuf::from(path))
    }
}

#[test]
fn project_files_open_folder_and_reveal_go_through_open() {
    let find = finder(&[("open", "/usr/bin/open")]);
    let open = open_folder_command(&find, Path::new("/p/shop")).expect("command");
    assert_eq!(open.program, Path::new("/usr/bin/open"));
    assert_eq!(open.args, [std::ffi::OsString::from("/p/shop")]);
    let reveal = reveal_command(&find, Path::new("/p/shop/compose.yaml")).expect("command");
    assert_eq!(
        reveal.args,
        [
            std::ffi::OsString::from("-R"),
            std::ffi::OsString::from("/p/shop/compose.yaml")
        ]
    );
    assert!(open_folder_command(&finder(&[]), Path::new("/p")).is_none());
}

#[test]
fn project_files_terminal_here_uses_terminal_app_unless_settings_name_another() {
    let find = finder(&[("open", "/usr/bin/open")]);
    let default = terminal_command(&find, None, Path::new("/p")).expect("command");
    assert_eq!(
        default.args,
        [
            std::ffi::OsString::from("-a"),
            std::ffi::OsString::from("Terminal"),
            std::ffi::OsString::from("/p")
        ]
    );
    let other = terminal_command(&find, Some("iTerm"), Path::new("/p")).expect("command");
    assert_eq!(other.args[1], std::ffi::OsString::from("iTerm"));
    let blank = terminal_command(&find, Some("  "), Path::new("/p")).expect("command");
    assert_eq!(blank.args[1], std::ffi::OsString::from("Terminal"));
}

#[test]
fn project_files_editor_is_vs_code_when_code_is_found_else_the_default_app() {
    let with_code = finder(&[("code", "/usr/local/bin/code"), ("open", "/usr/bin/open")]);
    let editor = editor_for(&with_code, None);
    assert_eq!(editor.program, Some(PathBuf::from("/usr/local/bin/code")));
    assert_eq!(editor.label, "Visual Studio Code (detected)");
    let without = finder(&[("open", "/usr/bin/open")]);
    let editor = editor_for(&without, None);
    assert_eq!(editor.program, None);
    assert_eq!(editor.label, "the default app");
}

#[test]
fn project_files_editor_setting_wins_when_it_is_found() {
    let find = finder(&[
        ("code", "/usr/local/bin/code"),
        ("zed", "/opt/bin/zed"),
        ("open", "/usr/bin/open"),
    ]);
    let editor = editor_for(&find, Some("zed"));
    assert_eq!(editor.program, Some(PathBuf::from("/opt/bin/zed")));
    assert_eq!(editor.label, "zed");
    // A name that is not installed does not break Open in editor.
    let editor = editor_for(&find, Some("nonesuch"));
    assert_eq!(editor.label, "Visual Studio Code (detected)");
}

#[test]
fn project_files_open_in_editor_gives_code_the_folder_and_the_file() {
    let find = finder(&[("code", "/usr/local/bin/code"), ("open", "/usr/bin/open")]);
    let command = open_in_editor_command(
        &find,
        None,
        Path::new("/p/shop"),
        Path::new("/p/shop/compose.yaml"),
    )
    .expect("command");
    assert_eq!(command.program, Path::new("/usr/local/bin/code"));
    assert_eq!(
        command.args,
        [
            std::ffi::OsString::from("/p/shop"),
            std::ffi::OsString::from("/p/shop/compose.yaml")
        ]
    );
}

#[test]
fn project_files_open_in_editor_falls_back_to_open_on_the_file() {
    let find = finder(&[("open", "/usr/bin/open")]);
    let command = open_in_editor_command(
        &find,
        None,
        Path::new("/p/shop"),
        Path::new("/p/shop/compose.yaml"),
    )
    .expect("command");
    assert_eq!(command.program, Path::new("/usr/bin/open"));
    assert_eq!(
        command.args,
        [std::ffi::OsString::from("/p/shop/compose.yaml")]
    );
    assert!(
        open_in_editor_command(&finder(&[]), None, Path::new("/p"), Path::new("/p/a")).is_none()
    );
}

#[test]
fn project_files_default_target_is_the_first_compose_file_else_the_folder() {
    let folder = Path::new("/p/shop");
    let files = [
        PathBuf::from("/p/shop/compose.yaml"),
        PathBuf::from("/p/shop/b.yaml"),
    ];
    assert_eq!(
        default_editor_target(&files, folder),
        Some(PathBuf::from("/p/shop/compose.yaml"))
    );
    assert_eq!(
        default_editor_target(&[], folder),
        Some(folder.to_path_buf())
    );
}
