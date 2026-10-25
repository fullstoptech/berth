//! P5-T3 tests for Review changes: the wording, the footer, the frames, every state, the masking,
//! and that nothing reaches a project's files except through the sheet's Write.

use std::path::{Path, PathBuf};

use berth_core::{ComposeCommand, GenerationPlan, HarnessId, PlanSource};

use super::model::{after_write_words, footer_labels, source_line, summary};
use crate::scenes::{self, NEW_APP_PASSWORD};
use crate::ui::diff_view::MASK;

const DEFAULT: &str = "Review changes / default / 1440";
const NEW_PROJECT: &str = "Review changes / new project / 1440";

fn up(services: &[&str]) -> Option<ComposeCommand> {
    Some(ComposeCommand::Up {
        services: services.iter().map(ToString::to_string).collect(),
        build: false,
    })
}

fn plan(files: Vec<berth_core::FileDiff>, after: Option<ComposeCommand>) -> GenerationPlan {
    GenerationPlan {
        root: PathBuf::from("/Users/me/Dev/new-app"),
        files,
        source: PlanSource::Editor,
        notes: Vec::new(),
        after_write: after,
    }
}

fn created(path: &str) -> berth_core::FileDiff {
    berth_rules::diff::file_diff(Path::new(path), None, Some("a\n"))
}

fn changed(path: &str) -> berth_core::FileDiff {
    berth_rules::diff::file_diff(Path::new(path), Some("a\n"), Some("b\n"))
}

fn env_with_new_keys() -> berth_core::FileDiff {
    berth_rules::diff::file_diff(Path::new(".env"), Some("A=1\n"), Some("A=1\nB=2\nC=3\n"))
}

// ---------------------------------------------------------------------
// The footer label is a pure function of `after_write`
// ---------------------------------------------------------------------

#[test]
fn review_changes_footer_up_with_no_services_says_start() {
    let labels = footer_labels(&plan(
        vec![created("a"), created("b"), created("c"), created("d")],
        up(&[]),
    ));
    assert_eq!(labels.primary, "Write 4 files and start");
    assert_eq!(labels.write_only, "Write files only");
    assert!(labels.runs);
}

#[test]
fn review_changes_footer_up_with_services_says_recreate_and_names_them() {
    let labels = footer_labels(&plan(
        vec![changed("a"), changed("b")],
        up(&["db", "worker"]),
    ));
    assert_eq!(labels.primary, "Write 2 files and recreate db, worker");
}

#[test]
fn review_changes_footer_without_a_command_is_a_single_write_button() {
    let labels = footer_labels(&plan(vec![changed("a")], None));
    assert_eq!(labels.primary, "Write 1 file");
    assert!(!labels.runs);
}

#[test]
fn review_changes_footer_words_cover_every_compose_command() {
    let words = |c: ComposeCommand| after_write_words(Some(&c)).expect("words");
    assert_eq!(
        words(ComposeCommand::Restart { services: vec![] }),
        "restart"
    );
    assert_eq!(
        words(ComposeCommand::Restart {
            services: vec!["api".to_string()]
        }),
        "restart api"
    );
    assert_eq!(
        words(ComposeCommand::Pull { services: vec![] }),
        "pull images"
    );
    assert_eq!(
        words(ComposeCommand::Build {
            services: vec![],
            pull: false
        }),
        "build"
    );
    assert_eq!(after_write_words(None), None);
}

// ---------------------------------------------------------------------
// The sentence under the title
// ---------------------------------------------------------------------

#[test]
fn review_changes_summary_counts_new_files_and_keys_added_to_env() {
    let p = plan(
        vec![
            created("compose.yaml"),
            created(".dockerignore"),
            created("Dockerfile"),
            env_with_new_keys(),
        ],
        None,
    );
    assert_eq!(
        summary(&p, "~/Dev/side/new-app"),
        "Adds 3 files and 2 keys to .env in ~/Dev/side/new-app. Writes nothing outside this folder."
    );
}

#[test]
fn review_changes_summary_of_changes_and_mixed_plans() {
    let p = plan(vec![changed("compose.yaml"), changed(".env")], None);
    assert_eq!(
        summary(&p, "~/x"),
        "Changes 2 files in ~/x. Writes nothing outside this folder."
    );
    let p = plan(vec![created("a"), changed("b")], None);
    assert_eq!(
        summary(&p, "~/x"),
        "Adds 1 file and changes 1 file in ~/x. Writes nothing outside this folder."
    );
    let p = plan(vec![], None);
    assert!(summary(&p, "~/x").starts_with("Nothing to write in ~/x."));
}

#[test]
fn review_changes_source_lines_say_where_the_plan_came_from() {
    let name = |id: &HarnessId| {
        if id.0 == "claude" {
            "Claude Code".to_string()
        } else {
            id.0.clone()
        }
    };
    assert_eq!(
        source_line(&PlanSource::Recipe("Next.js".to_string()), name),
        "From the Next.js recipe."
    );
    assert_eq!(
        source_line(&PlanSource::Template("Postgres".to_string()), name),
        "From the Postgres template."
    );
    assert_eq!(
        source_line(&PlanSource::Harness(HarnessId("claude".to_string())), name),
        "Proposed by Claude Code."
    );
    assert_eq!(
        source_line(&PlanSource::Harness(HarnessId("muse".to_string())), name),
        "Proposed by muse."
    );
}

// ---------------------------------------------------------------------
// The frames
// ---------------------------------------------------------------------

fn scene(frame: &str) -> String {
    scenes::render_scene_html(frame).unwrap_or_else(|| panic!("{frame} renders"))
}

#[test]
fn review_changes_default_frame_shows_a_harness_plan_and_recreates_the_named_services() {
    let html = scene(DEFAULT);
    assert!(html.contains("Review changes \u{b7} Claude Code"), "{html}");
    assert!(
        html.contains(
            "Changes 2 files in ~/Dev/clients/acme/shop-api. Writes nothing outside this folder."
        ),
        "{html}"
    );
    assert!(html.contains("Proposed by Claude Code."), "{html}");
    assert!(
        html.contains("Volume shop-api_pgdata (1.2 GB) is kept."),
        "{html}"
    );
    for label in [
        "Cancel",
        "Write files only",
        "Write 2 files and recreate db, worker",
    ] {
        assert!(html.contains(&format!(">{label}<")), "{label}: {html}");
    }
    // Both files are listed with their counts, and the diffs are stacked.
    assert!(
        html.contains(">+9<") && html.contains(">\u{2212}2<"),
        "{html}"
    );
    assert!(
        html.contains("Changes to compose.yaml") && html.contains("Changes to .env"),
        "{html}"
    );
    assert!(html.contains("ports: [&#34;5433:5432&#34;]"), "{html}");
    assert!(
        !html.contains("more lines"),
        "a short plan is not collapsed: {html}"
    );
}

#[test]
fn review_changes_new_project_frame_adds_files_and_keys_and_starts() {
    let html = scene(NEW_PROJECT);
    assert!(html.contains("Review changes \u{b7} new-app"), "{html}");
    assert!(
        html.contains("Adds 3 files and 2 keys to .env in ~/Dev/side/new-app. Writes nothing outside this folder."),
        "{html}"
    );
    assert!(html.contains(">Write 4 files and start<"), "{html}");
    assert!(html.contains(">Write files only<"), "{html}");
    assert!(html.contains("From the Next.js recipe."), "{html}");
    // The long compose file collapses as the frame does.
    assert!(html.contains("Show 2 more lines"), "{html}");
    for name in ["compose.yaml", ".dockerignore", ".env.example", ".env"] {
        assert!(html.contains(name), "{name}: {html}");
    }
    assert!(
        html.contains("Adds 2 keys. Existing keys unchanged."),
        "{html}"
    );
    assert!(html.contains("rc-sheet-tall"), "{html}");
}

#[test]
fn review_changes_new_project_frame_masks_the_generated_password_everywhere() {
    let html = scene(NEW_PROJECT);
    assert!(
        !html.contains(NEW_APP_PASSWORD),
        "the generated password reached the markup"
    );
    assert!(
        html.contains(&format!("POSTGRES_PASSWORD={MASK}")),
        "{html}"
    );
    // The key preview under the list is masked too, and .env.example (documented as empty of
    // secrets) is shown as it is.
    assert!(
        html.contains(&format!("+ POSTGRES_PASSWORD={MASK}")),
        "{html}"
    );
    assert!(html.contains("Reveal values"), "{html}");
}

#[test]
fn review_changes_default_frame_masks_every_env_value_but_not_compose() {
    let html = scene(DEFAULT);
    assert!(!html.contains("postgres://shop@localhost"), "{html}");
    assert!(!html.contains("NODE_ENV=development"), "{html}");
    assert!(html.contains(&format!("DATABASE_URL={MASK}")), "{html}");
    assert!(html.contains(&format!("NODE_ENV={MASK}")), "{html}");
    // compose.yaml is not a secret file: its ports are plain text.
    assert!(html.contains("ports: [&#34;5432:5432&#34;]"), "{html}");
}

// ---------------------------------------------------------------------
// States
// ---------------------------------------------------------------------

fn button_tag<'a>(html: &'a str, label: &str) -> &'a str {
    let at = html
        .find(&format!(">{label}<"))
        .unwrap_or_else(|| panic!("no {label} button: {html}"));
    let start = html[..at].rfind("<button").expect("button tag");
    &html[start..at]
}

#[test]
fn review_changes_conflict_state_refuses_the_write_and_offers_regenerate() {
    let html = scene("Review changes / conflict / 1440");
    assert!(
        html.contains(".env changed since this plan was made. Nothing was written."),
        "{html}"
    );
    assert!(html.contains(">Regenerate<"), "{html}");
    // Neither write button works until the plan is regenerated.
    for label in ["Write files only", "Write 4 files and start"] {
        let tag = button_tag(&html, label);
        assert!(tag.contains("disabled"), "{label}: {tag}");
        assert!(tag.contains("Regenerate the plan first"), "{label}: {tag}");
    }
}

#[test]
fn review_changes_write_error_state_says_nothing_was_changed() {
    let html = scene("Review changes / write error / 1440");
    assert!(html.contains("Could not write the files."), "{html}");
    assert!(html.contains("permission denied"), "{html}");
    assert!(html.contains("Nothing was changed."), "{html}");
    assert!(
        !button_tag(&html, "Write 4 files and start").contains("disabled"),
        "{html}"
    );
}

#[test]
fn review_changes_start_error_state_says_the_files_are_written_and_shows_the_reason() {
    let html = scene("Review changes / start error / 1440");
    assert!(
        html.contains("The files are written, but it did not start."),
        "{html}"
    );
    assert!(html.contains("port is already allocated"), "{html}");
    // Writing again would be wrong: only Close is left.
    assert!(html.contains(">Close<"), "{html}");
    assert!(!html.contains("Write files only"), "{html}");
}

#[test]
fn review_changes_invalid_compose_text_disables_write_and_quotes_compose() {
    let html = scene("Review changes / invalid compose / 1440");
    assert!(html.contains("The compose file is not valid."), "{html}");
    assert!(html.contains("services.app.ports must be a list"), "{html}");
    let tag = button_tag(&html, "Write 4 files and start");
    assert!(tag.contains("disabled"), "{tag}");
}

#[test]
fn review_changes_empty_plan_says_so_and_offers_only_close() {
    let html = scene("Review changes / empty / 1440");
    assert!(html.contains("There is nothing to write."), "{html}");
    assert!(
        html.contains("This folder already has a compose file."),
        "{html}"
    );
    assert!(html.contains(">Close<"), "{html}");
    assert!(!html.contains("Write files only"), "{html}");
    assert!(!html.contains(">Cancel<"), "{html}");
}

#[test]
fn review_changes_mock_data_disables_write_with_the_reason() {
    let html = scene("Review changes / mock / 1440");
    for label in ["Write files only", "Write 2 files and recreate db, worker"] {
        let tag = button_tag(&html, label);
        assert!(tag.contains("disabled"), "{label}: {tag}");
        assert!(
            tag.contains(
                "Writing project files need a real Docker engine. You&#39;re viewing mock data."
            ),
            "{label}: {tag}"
        );
    }
    // The review itself is still readable.
    assert!(html.contains("Changes to compose.yaml"), "{html}");
}

#[test]
fn review_changes_a_plan_that_is_gone_says_so_instead_of_crashing() {
    let html = scene("Review changes / gone / 1440");
    assert!(html.contains("This plan is no longer available."), "{html}");
    assert!(!html.contains("Write"), "{html}");
}

// ---------------------------------------------------------------------
// Nothing changes without a click
// ---------------------------------------------------------------------

/// Every `.rs` file under `crates/berth-app/src`, with its text.
fn sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).expect("dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("read");
                out.push((path, text));
            }
        }
    }
    let mut out = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

#[test]
fn review_changes_only_the_plan_store_calls_write_apply() {
    let mut callers = Vec::new();
    for (path, text) in sources() {
        let in_tests = path.to_string_lossy().contains("tests");
        if !in_tests && text.contains("write::apply(") {
            callers.push(path.file_name().map(|n| n.to_string_lossy().into_owned()));
        }
    }
    assert_eq!(callers, [Some("plans.rs".to_string())], "{callers:?}");
}

/// Whether `text` calls `name(` as a function of its own, not as the tail of a longer name
/// (`compose_write_plan(` is not `write_plan(`).
fn calls(text: &str, name: &str) -> bool {
    text.match_indices(&format!("{name}(")).any(|(at, _)| {
        !text[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

#[test]
fn review_changes_only_the_sheet_and_its_service_call_commit_or_write_plan() {
    for (path, text) in sources() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let lives_here =
            name == "plans.rs" || name == "review_changes.rs" || name.contains("tests");
        if lives_here {
            continue;
        }
        assert!(
            !calls(&text, "write_plan") && !calls(&text, "commit"),
            "{} writes a plan outside Review changes",
            path.display()
        );
    }
}

#[test]
fn review_changes_whatever_pins_a_folder_tells_the_engines_service() {
    // A folder pinned at runtime is a project at once only when the engines service is told.
    for (path, text) in sources() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let pins = calls(&text, "record_open") || calls(&text, "remember_folder");
        let defines = name == "folders.rs" || name == "plans.rs" || name.contains("tests");
        if pins && !defines {
            assert!(
                text.contains("pins_changed()"),
                "{} pins a folder without EngineLink::pins_changed",
                path.display()
            );
        }
    }
}

#[test]
fn review_changes_no_log_line_carries_a_plan() {
    // A plan holds the generated passwords (CLAUDE.md: secrets, logging). No tracing call may take
    // one as a field, by Debug, Display or name.
    for (path, text) in sources() {
        if path.to_string_lossy().contains("tests") {
            continue;
        }
        for (n, line) in text.lines().enumerate() {
            let logs = [
                "tracing::",
                "warn!(",
                "info!(",
                "debug!(",
                "error!(",
                "trace!(",
            ]
            .iter()
            .any(|m| line.contains(m));
            let names_a_plan = [
                "?plan",
                "%plan",
                "plan =",
                "?pending",
                "%pending",
                "pending =",
            ]
            .iter()
            .any(|m| line.contains(m));
            assert!(
                !(logs && names_a_plan),
                "{}:{} logs a plan: {line}",
                path.display(),
                n + 1
            );
        }
    }
}

#[test]
fn review_changes_opening_the_sheet_writes_nothing() {
    use dioxus::prelude::*;

    let dir = tempfile::tempdir().expect("dir");
    std::fs::write(dir.path().join(".env"), "A=1\n").expect("env");
    let before = std::fs::read_dir(dir.path()).expect("list").count();
    #[component]
    fn Root(dir: PathBuf) -> Element {
        let state = crate::state::use_app_state();
        use_context_provider(|| state);
        let store = use_context_provider(crate::services::plans::PlanStore::default);
        let mut pending = scenes::new_app_pending();
        pending.plan.root = dir;
        pending.plan.files = vec![
            berth_rules::diff::file_diff(Path::new("compose.yaml"), None, Some("services: {}\n")),
            berth_rules::diff::file_diff(Path::new(".env"), Some("A=1\n"), Some("A=1\nB=2\n")),
        ];
        let id = store.insert(pending);
        rsx! { super::ReviewChangesSheet { id } }
    }
    let mut dom = VirtualDom::new_with_props(
        Root,
        RootProps {
            dir: dir.path().to_path_buf(),
        },
    );
    let _ = dom.rebuild_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Write 2 files and start"), "{html}");

    assert_eq!(std::fs::read_dir(dir.path()).expect("list").count(), before);
    assert!(!dir.path().join("compose.yaml").exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".env")).expect("env"),
        "A=1\n"
    );
}

#[test]
fn review_changes_compose_failure_text_keeps_errors_and_drops_json_warnings() {
    use super::model::compose_failure_text;
    let raw = "{\"level\":\"warning\",\"msg\":\"buildx Docker CLI plugin not found\",\"time\":\"t\"}\n\
               {\"error\":true,\"message\":\"The command '/bin/sh -c pnpm install' returned a non-zero code: 1\"}";
    assert_eq!(
        compose_failure_text(raw),
        "The command '/bin/sh -c pnpm install' returned a non-zero code: 1"
    );
    // Plain text stays; text that is only warnings stays too rather than saying nothing.
    assert_eq!(
        compose_failure_text("Bind for 0.0.0.0:3000 failed: port is already allocated"),
        "Bind for 0.0.0.0:3000 failed: port is already allocated"
    );
    let only_warning = "{\"level\":\"warning\",\"msg\":\"x\"}";
    assert_eq!(compose_failure_text(only_warning), only_warning);
}
