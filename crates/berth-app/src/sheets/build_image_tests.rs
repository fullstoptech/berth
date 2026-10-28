//! P8-T3 tests for the Build image sheet: the pure form rules, the log
//! view, and the scenes (form, Dockerfile, running, failed, done, pushing)
//! read as the markup a person would see.

use super::*;
use crate::scenes;
use crate::state::use_app_state;

fn markup(frame: &str) -> String {
    let mut out = scenes::render_scene_html(frame).expect("the frame has a scene");
    // Class names in assertions mean elements, not rules of a stylesheet.
    while let (Some(a), Some(b)) = (out.find("<style>"), out.find("</style>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</style>".len(), "");
    }
    out
}

/// The sheet alone: everything from its dialog on.
fn sheet(frame: &str) -> String {
    let html = markup(frame);
    let start = html.find("class=\"bi-sheet\"").expect("the sheet is drawn");
    html[start..].to_string()
}

fn row(id: u32, name: &str, value: &str) -> ArgRow {
    ArgRow {
        id,
        name: name.to_string(),
        value: value.to_string(),
    }
}

fn line(kind: BuildLineKind, text: &str) -> BuildLine {
    BuildLine {
        kind,
        text: text.to_string(),
    }
}

// -- pure rules ---------------------------------------------------------

#[test]
fn build_image_tags_split_on_spaces_and_commas() {
    assert_eq!(
        parse_tags("  myapp:dev, myapp:1  ghcr.io/me/app:1,, "),
        ["myapp:dev", "myapp:1", "ghcr.io/me/app:1"]
    );
    assert!(parse_tags(" , ").is_empty());
}

#[test]
fn build_image_a_digest_is_not_a_name_for_a_new_image() {
    let digest = format!("app@sha256:{}", "a".repeat(64));
    assert!(tags_problem(&digest).is_some());
    assert!(tags_problem("myapp:dev other:1").is_none());
    assert!(tags_problem("My App").is_some());
}

#[test]
fn build_image_arguments_skip_empty_rows_and_never_print_values() {
    let rows = [
        row(0, " TOKEN ", "s3cret value"),
        row(1, "", ""),
        row(2, "", "orphan"),
    ];
    assert_eq!(
        build_args(&rows),
        [
            ("TOKEN".to_string(), "s3cret value".to_string()),
            (String::new(), "orphan".to_string()),
        ]
    );
    let shown = format!("{:?}", rows[0]);
    assert!(shown.contains("TOKEN"), "{shown}");
    assert!(!shown.contains("s3cret"), "{shown}");
}

#[test]
fn build_image_spec_follows_the_form() {
    let folder = Path::new("/work/app");
    let spec = form_spec(
        Source::Folder,
        Some(folder),
        "Dockerfile",
        "ignored",
        "app:1 app:latest",
        &[row(0, "A", "1")],
        "linux/amd64",
    );
    assert_eq!(spec.context, BuildContext::Folder(folder.to_path_buf()));
    assert_eq!(spec.dockerfile, None, "the default name is not sent");
    assert_eq!(spec.tags, ["app:1", "app:latest"]);
    assert_eq!(spec.build_args, [("A".to_string(), "1".to_string())]);
    assert_eq!(spec.platform.as_deref(), Some("linux/amd64"));

    let named = form_spec(
        Source::Folder,
        Some(folder),
        " docker/Dev.Dockerfile ",
        "",
        "a",
        &[],
        "",
    );
    assert_eq!(named.dockerfile.as_deref(), Some("docker/Dev.Dockerfile"));
    assert_eq!(named.platform, None);

    let inline = form_spec(
        Source::Dockerfile,
        Some(folder),
        "Other",
        "FROM x\n",
        "a",
        &[],
        "",
    );
    assert_eq!(inline.context, BuildContext::Inline("FROM x\n".to_string()));
    assert_eq!(inline.dockerfile, None, "an inline context has one file");
}

#[test]
fn build_image_blocker_names_the_first_missing_piece() {
    let folder = Path::new("/work/app");
    let blocker =
        |source, folder, inline, tags, engine| form_blocker(source, folder, inline, tags, engine);
    assert_eq!(
        blocker(Source::Folder, Some(folder), "", "a:1", false).as_deref(),
        Some("No running engine to build on")
    );
    assert_eq!(
        blocker(Source::Folder, None, "", "a:1", true).as_deref(),
        Some("Choose the folder to build")
    );
    assert_eq!(
        blocker(Source::Dockerfile, None, "  \n", "a:1", true).as_deref(),
        Some("Write the Dockerfile first")
    );
    assert_eq!(
        blocker(Source::Folder, Some(folder), "", " ", true).as_deref(),
        Some("Give the image a name such as myapp:dev")
    );
    assert!(blocker(Source::Folder, Some(folder), "", "Bad Name", true).is_some());
    assert_eq!(blocker(Source::Folder, Some(folder), "", "a:1", true), None);
    assert_eq!(
        blocker(Source::Dockerfile, None, "FROM x", "a:1", true),
        None
    );
}

#[test]
fn build_image_failing_step_is_the_last_step_before_the_first_failure() {
    use BuildLineKind::{Failed, Log, Step};
    let lines = [
        line(Step, "Step 1/3 : FROM x"),
        line(Log, "ok"),
        line(Step, "Step 2/3 : RUN y"),
        line(Log, "boom"),
        line(Failed, "returned a non-zero code: 1"),
        line(Failed, "error again"),
    ];
    assert_eq!(failing_step_line(&lines), Some(2));
    assert_eq!(failing_step_line(&lines[..2]), None, "no failure, no mark");
    assert_eq!(failing_step_line(&lines[3..]), None, "no step before it");
}

#[test]
fn build_image_view_draws_only_the_newest_lines_and_keeps_the_failing_mark() {
    use berth_core::BuildEvent::{Failed, Log, Step};
    let spec = BuildSpec {
        context: BuildContext::Inline("FROM x".to_string()),
        dockerfile: None,
        tags: vec!["app".to_string()],
        build_args: Vec::new(),
        platform: None,
    };
    let mut build = BuildState::new(EngineId("e".to_string()), &spec);
    build.apply(&Step {
        index: 1,
        total: 2,
        text: "Step 1/2 : FROM x".to_string(),
    });
    for i in 0..LOG_SHOWN + 50 {
        build.apply(&Log(format!("line {i}")));
    }
    build.apply(&Step {
        index: 2,
        total: 2,
        text: "Step 2/2 : RUN y".to_string(),
    });
    build.apply(&Failed {
        text: "error".to_string(),
    });

    let view = BuildView::of(&build);
    assert_eq!(view.lines.len(), LOG_SHOWN);
    assert_eq!(view.hidden, build.lines.len() - LOG_SHOWN);
    assert_eq!(
        view.lines[view.failing.expect("step 2 is drawn")].text,
        "Step 2/2 : RUN y"
    );
    assert_eq!(view.lines.last().map(|l| l.text.as_str()), Some("error"));
    assert_eq!(
        view.first_tag(),
        "app:latest",
        "the image list names it with its tag"
    );

    // A failing step that scrolled out of the window is not marked.
    let mut long = build.clone();
    for i in 0..LOG_SHOWN {
        long.apply(&Log(format!("after {i}")));
    }
    assert_eq!(BuildView::of(&long).failing, None);
}

#[test]
fn build_image_size_is_read_from_the_first_tag_on_the_builds_engine() {
    let world = berth_engine::MockWorld::bundled();
    let (engine, images) = world
        .images_by_engine
        .iter()
        .next()
        .expect("an engine with images");
    let engine = EngineId(engine.clone());
    let image = images.first().expect("an image");
    let tag = image.repo_tags.first().expect("a tag").clone();
    let all: Vec<Image> = images
        .iter()
        .cloned()
        .map(|mut i| {
            i.engine = engine.clone();
            i
        })
        .collect();
    assert_eq!(
        built_size(&all, &engine, &tag),
        Some(berth_core::fmt::bytes(image.size))
    );
    assert_eq!(built_size(&all, &EngineId("other".to_string()), &tag), None);
    assert_eq!(built_size(&all, &engine, "nothing:here"), None);
}

#[test]
fn build_image_export_offers_a_safe_file_name() {
    assert_eq!(
        export_file_name("postgres:17-alpine"),
        "postgres-17-alpine.tar"
    );
    assert_eq!(export_file_name("ghcr.io/me/app:1"), "ghcr.io-me-app-1.tar");
    assert_eq!(export_file_name("sha256:abc"), "sha256-abc.tar");
    assert_eq!(export_file_name("///"), "image.tar");
}

#[test]
fn build_image_export_panel_opens_in_downloads_with_the_default_file_name() {
    let downloads = std::path::PathBuf::from("/Users/someone/Downloads");
    let request = export_panel_request("postgres:17-alpine", downloads.clone());
    assert_eq!(request.directory, downloads);
    assert_eq!(request.file_name, "postgres-17-alpine.tar");
    assert_eq!(request.title, "Export postgres:17-alpine");
}

#[test]
fn build_image_import_toast_names_what_was_loaded() {
    let names = |n: &[&str]| n.iter().map(ToString::to_string).collect::<Vec<_>>();
    assert_eq!(imported_text(&[], "a.tar"), "Imported a.tar");
    assert_eq!(imported_text(&names(&["x:1"]), "a.tar"), "Imported x:1");
    assert_eq!(
        imported_text(&names(&["x:1", "y:2"]), "a.tar"),
        "Imported x:1 and y:2"
    );
    assert_eq!(
        imported_text(&names(&["x:1", "y:2", "z:3", "w:4"]), "a.tar"),
        "Imported x:1, y:2 and 2 more"
    );
}

// -- entry points -------------------------------------------------------

thread_local! {
    static SLOT: std::cell::RefCell<Option<AppState>> = const { std::cell::RefCell::new(None) };
}

#[component]
fn Harness() -> Element {
    let state = use_app_state();
    use_hook(move || SLOT.with(|slot| *slot.borrow_mut() = Some(state)));
    rsx! { div {} }
}

#[test]
fn build_image_open_adds_the_sheet_once() {
    let mut dom = VirtualDom::new(Harness);
    let _ = dom.rebuild_to_vec();
    let state = SLOT.with(|s| s.borrow_mut().take()).expect("captured");
    dom.in_runtime(|| {
        let mut state = state;
        open(&mut state);
        open(&mut state);
        let count = state
            .overlays
            .peek()
            .iter()
            .filter(|o| matches!(o, Overlay::BuildImage))
            .count();
        assert_eq!(count, 1);
        close(&mut state);
        assert!(state.overlays.peek().is_empty());
    });
}

#[test]
fn build_image_images_screen_offers_build_and_import() {
    let html = markup("Images / on this engine / 1440");
    assert!(html.contains(">Build image\u{2026}<"), "{html}");
    assert!(html.contains(">Import image\u{2026}<"), "{html}");
    assert!(html.contains(">Pull image<"), "{html}");
}

#[test]
fn build_image_inspector_offers_push_and_export() {
    let html = markup("Images / selected / 1440");
    assert!(html.contains(">Push\u{2026}<"), "{html}");
    assert!(html.contains(">Export\u{2026}<"), "{html}");
    // The tagged image can be pushed: its button is not locked.
    let (before, _) = html.split_once(">Push\u{2026}<").expect("the button");
    let button = before.rsplit("<button").next().expect("its element");
    assert!(!button.contains("disabled"), "{button}");
}

// -- scenes -------------------------------------------------------------

#[test]
fn build_image_scene_form_asks_for_a_folder_tags_arguments_and_platform() {
    let html = sheet("Images / build / 1440");
    assert!(html.contains("aria-label=\"Build image\""), "{html}");
    assert!(html.contains("No folder chosen"), "{html}");
    assert!(html.contains(">Choose folder\u{2026}<"), "{html}");
    for label in ["Tags", "Platform", "Build arguments", "Dockerfile"] {
        assert!(html.contains(label), "{label}: {html}");
    }
    assert!(html.contains("aria-label=\"Image tags\""), "{html}");
    // Nothing chosen yet: Build is locked and says why.
    let (before, _) = html
        .rsplit_once("class=\"btn-label\">Build<")
        .expect("the Build button");
    let button = before.rsplit("<button").next().expect("its element");
    assert!(button.contains("disabled"), "{button}");
    assert!(
        button.contains("data-tooltip=\"Choose the folder to build\""),
        "{button}"
    );
    assert!(html.contains("Builds on "), "{html}");
    assert!(!html.contains("berth-editor-build-image"), "{html}");
}

#[test]
fn build_image_scene_dockerfile_swaps_the_folder_for_the_editor() {
    let html = sheet("Images / build dockerfile / 1440");
    assert!(html.contains("id=\"berth-editor-build-image\""), "{html}");
    assert!(!html.contains("Choose folder"), "{html}");
    // The editor's own starter text is not in the markup (the page draws
    // it), but the Dockerfile's blocker is "Write it", not "Choose folder".
    assert!(
        html.contains("data-tooltip=\"Give the image a name such as myapp:dev\""),
        "{html}"
    );
}

#[test]
fn build_image_scene_running_shows_the_step_the_log_and_cancel() {
    let html = sheet("Images / build running / 1440");
    assert!(html.contains("Building shop-api-worker:latest"), "{html}");
    assert!(html.contains("Step 4 of 6"), "{html}");
    // Three steps done of six.
    assert!(html.contains("aria-valuenow=\"50\""), "{html}");
    assert!(html.contains("role=\"log\""), "{html}");
    assert!(html.contains("Step 4/6 : RUN npm ci"), "{html}");
    assert!(html.contains(">Cancel build<"), "{html}");
    assert!(
        html.contains("Closing this keeps the build running"),
        "{html}"
    );
    assert!(!html.contains("bi-line-failed"), "{html}");
    assert!(!html.contains("Push next"), "{html}");
}

#[test]
fn build_image_scene_failed_marks_the_failing_step_and_links_to_problems() {
    let html = sheet("Images / build failed / 1440");
    assert!(
        html.contains("Could not build shop-api-worker:latest"),
        "{html}"
    );
    // Step 4 and the error are marked; step 3 is a plain step.
    assert!(
        html.contains("class=\"bi-line bi-line-step bi-line-failed\">Step 4/6 : RUN npm ci<"),
        "{html}"
    );
    assert!(
        html.contains(
            "class=\"bi-line bi-line-failed\">The command &#39;/bin/sh -c npm ci&#39; returned a non-zero code: 1<"
        ),
        "{html}"
    );
    assert!(
        html.contains("class=\"bi-line bi-line-step\">Step 3/6"),
        "{html}"
    );
    assert_eq!(html.matches("bi-line-failed").count(), 2, "{html}");
    // The error also stands above the log, as a message.
    assert!(html.contains("role=\"alert\""), "{html}");
    assert!(html.contains(">Open Problems<"), "{html}");
    assert!(html.contains(">Edit and rebuild<"), "{html}");
    assert!(!html.contains("Push next"), "{html}");
}

#[test]
fn build_image_scene_done_reports_the_size_and_offers_push_next() {
    let html = sheet("Images / build done / 1440");
    assert!(html.contains("Built shop-api-worker:latest"), "{html}");
    let world = berth_engine::MockWorld::bundled();
    let size = world
        .images_by_engine
        .values()
        .flatten()
        .find(|i| i.repo_tags.iter().any(|t| t == "shop-api-worker:latest"))
        .map(|i| berth_core::fmt::bytes(i.size))
        .expect("the mock world lists the tag");
    assert!(html.contains(&format!(">{size}<")), "{size}: {html}");
    assert!(html.contains("shop-api-worker:2"), "the second tag: {html}");
    assert!(html.contains(">Push next<"), "{html}");
    assert!(html.contains(">Build another<"), "{html}");
    assert!(!html.contains("bi-line-failed"), "{html}");
    assert!(!html.contains("Open Problems"), "{html}");
}

#[test]
fn build_image_scene_pushing_shows_the_push_card_under_the_log() {
    let html = sheet("Images / build pushing / 1440");
    assert!(html.contains("Pushing shop-api-worker:latest"), "{html}");
    assert!(html.contains(">Cancel push<"), "{html}");
    assert!(html.contains("The push refers to repository"), "{html}");
}
