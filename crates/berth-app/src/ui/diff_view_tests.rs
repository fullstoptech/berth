//! P5-T3 tests for the diff view: `.env` values are masked before anything is drawn, Reveal shows
//! them for that file only, a long file collapses.

use std::path::Path;

use dioxus::prelude::*;

use super::{
    added_key_lines, file_view, is_secret_file, key_of, mask_line, DiffCard, DiffFileList,
    COLLAPSE_AT, MASK,
};

const SECRET: &str = "pw-Zk39xQ7-do-not-print";

fn env_diff() -> berth_core::FileDiff {
    berth_rules::diff::file_diff(
        Path::new(".env"),
        Some("NODE_ENV=development\nDATABASE_URL=postgres://app:old@db/app\n"),
        Some(&format!(
            "NODE_ENV=development\nDATABASE_URL=postgres://app:old@db/app\nPOSTGRES_PASSWORD={SECRET}\n"
        )),
    )
}

/// Components with handler props need a runtime, so they are mounted in a small tree.
#[component]
fn CardHost(file: super::FileView, expanded: bool) -> Element {
    rsx! { DiffCard { file, index: 0, expanded } }
}

#[component]
fn ListHost(files: Vec<super::FileView>, selected: usize) -> Element {
    rsx! { DiffFileList { files, selected } }
}

fn render<P: Clone + 'static>(root: fn(P) -> Element, props: P) -> String {
    let mut dom = VirtualDom::new_with_props(root, props);
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

fn card(file: super::FileView, expanded: bool) -> String {
    render(CardHost, CardHostProps { file, expanded })
}

#[test]
fn review_changes_secret_files_are_dot_env_and_dot_env_variants_but_not_the_example() {
    for secret in [
        ".env",
        ".env.local",
        ".env.production",
        "app/.env",
        ".env.sample",
    ] {
        assert!(is_secret_file(Path::new(secret)), "{secret}");
    }
    for open in [
        ".env.example",
        "compose.yaml",
        "envfile",
        "Dockerfile",
        ".environment",
    ] {
        assert!(!is_secret_file(Path::new(open)), "{open}");
    }
}

#[test]
fn review_changes_mask_keeps_the_key_and_hides_everything_after_the_first_equals() {
    assert_eq!(mask_line("A=secret"), format!("A={MASK}"));
    assert_eq!(
        mask_line("URL=postgres://u:p@h/db?x=1"),
        format!("URL={MASK}")
    );
    assert_eq!(
        mask_line("export TOKEN=abc"),
        format!("export TOKEN={MASK}")
    );
    assert_eq!(mask_line("EMPTY="), format!("EMPTY={MASK}"));
    // The length of a value is not told.
    assert_eq!(mask_line("A=x"), mask_line("A=xxxxxxxxxxxxxxxxxxxxxxxx"));
    // A comment or blank line carries no value.
    assert_eq!(mask_line("# the database"), "# the database");
    assert_eq!(mask_line(""), "");
    // A comment that holds an assignment is masked like one.
    assert_eq!(mask_line("# API_KEY=abc123"), format!("# API_KEY={MASK}"));
    // The middle of a quoted multi-line value has no `=`: it is masked whole.
    assert_eq!(mask_line("MIIEvQIBADANBgkqhkiG9w0BAQEFAASC"), MASK);
}

#[test]
fn review_changes_key_of_reads_the_key_of_an_assignment_only() {
    assert_eq!(key_of("A=1"), Some("A"));
    assert_eq!(key_of("export B = 2"), Some("B"));
    assert_eq!(key_of("# C=3"), None);
    assert_eq!(key_of("no equals"), None);
    assert_eq!(key_of("=x"), None);
}

#[test]
fn review_changes_masked_file_view_never_holds_the_value_and_revealed_does() {
    let diff = env_diff();
    let masked = file_view(&diff, false);
    assert!(masked.masked && masked.secret);
    let everything: String = masked.lines.iter().map(|l| l.text.as_str()).collect();
    assert!(!everything.contains(SECRET), "{everything}");
    assert!(
        !everything.contains("postgres://app:old@db/app"),
        "{everything}"
    );
    assert!(
        everything.contains(&format!("POSTGRES_PASSWORD={MASK}")),
        "{everything}"
    );
    assert!(
        everything.contains(&format!("NODE_ENV={MASK}")),
        "{everything}"
    );

    let revealed = file_view(&diff, true);
    assert!(!revealed.masked && revealed.secret);
    let everything: String = revealed.lines.iter().map(|l| l.text.as_str()).collect();
    assert!(everything.contains(SECRET), "{everything}");
}

#[test]
fn review_changes_added_keys_are_masked_until_revealed() {
    let diff = env_diff();
    assert_eq!(
        added_key_lines(&diff, false),
        [format!("POSTGRES_PASSWORD={MASK}")]
    );
    assert_eq!(
        added_key_lines(&diff, true),
        [format!("POSTGRES_PASSWORD={SECRET}")]
    );
    // A file that is not secret is shown as it is.
    let compose = berth_rules::diff::file_diff(Path::new(".env.example"), None, Some("A=1\n"));
    assert_eq!(added_key_lines(&compose, false), ["A=1"]);
}

#[test]
fn review_changes_card_markup_hides_the_value_and_offers_reveal() {
    let html = card(file_view(&env_diff(), false), true);
    assert!(!html.contains(SECRET), "{html}");
    assert!(html.contains("Reveal values"), "{html}");
    assert!(html.contains("aria-pressed=\"false\""), "{html}");
    assert!(
        html.contains(&format!("POSTGRES_PASSWORD={MASK}")),
        "{html}"
    );

    let html = card(file_view(&env_diff(), true), true);
    assert!(html.contains(SECRET), "{html}");
    assert!(html.contains("Hide values"), "{html}");
    assert!(html.contains("aria-pressed=\"true\""), "{html}");
}

#[test]
fn review_changes_card_for_a_file_that_is_not_secret_has_no_reveal() {
    let diff =
        berth_rules::diff::file_diff(Path::new("compose.yaml"), Some("a: 1\n"), Some("a: 2\n"));
    let html = card(file_view(&diff, false), true);
    assert!(!html.contains("Reveal values"), "{html}");
    assert!(html.contains("a: 2"), "{html}");
    assert!(html.contains("dv-line-added"), "{html}");
    assert!(html.contains("dv-line-removed"), "{html}");
}

#[test]
fn review_changes_card_counts_and_new_file_tag() {
    let new = berth_rules::diff::file_diff(Path::new("compose.yaml"), None, Some("a: 1\nb: 2\n"));
    let html = card(file_view(&new, false), true);
    assert!(html.contains(">+2<"), "{html}");
    assert!(html.contains("new file"), "{html}");
    assert!(!html.contains('\u{2212}'), "no removed count: {html}");
    let changed = berth_rules::diff::file_diff(Path::new("a"), Some("x\ny\n"), Some("x\nz\nw\n"));
    let html = card(file_view(&changed, false), true);
    assert!(
        html.contains(">+2<") && html.contains(">\u{2212}1<"),
        "{html}"
    );
}

#[test]
fn review_changes_long_file_collapses_after_the_frame_height_and_expands() {
    let text: String = (1..=COLLAPSE_AT + 5)
        .map(|n| format!("line {n}\n"))
        .collect();
    let diff = berth_rules::diff::file_diff(Path::new("compose.yaml"), None, Some(&text));
    let view = file_view(&diff, false);
    let html = card(view.clone(), false);
    assert!(html.contains("Show 5 more lines"), "{html}");
    assert!(html.contains(&format!(">line {COLLAPSE_AT}<")), "{html}");
    assert!(
        !html.contains(&format!(">line {}<", COLLAPSE_AT + 1)),
        "{html}"
    );
    let html = card(view, true);
    assert!(!html.contains("more lines"), "{html}");
    assert!(
        html.contains(&format!(">line {}<", COLLAPSE_AT + 5)),
        "{html}"
    );
}

#[test]
fn review_changes_hunks_are_separated_by_a_gap_mark() {
    let old: String = (1..=30).map(|n| format!("l{n}\n")).collect();
    let new = old.replace("l2\n", "L2\n").replace("l28\n", "L28\n");
    let diff = berth_rules::diff::file_diff(Path::new("f"), Some(&old), Some(&new));
    assert_eq!(diff.hunks.len(), 2);
    let view = file_view(&diff, false);
    assert_eq!(view.lines.iter().filter(|l| l.gap_before).count(), 1);
    assert!(card(view, true).contains("dv-gap"));
}

#[test]
fn review_changes_file_list_shows_each_file_with_its_counts_and_marks_the_selected() {
    let a = file_view(
        &berth_rules::diff::file_diff(Path::new("compose.yaml"), Some("a\n"), Some("b\nc\n")),
        false,
    );
    let b = file_view(
        &berth_rules::diff::file_diff(Path::new(".env"), None, Some("A=1\n")),
        false,
    );
    let html = render(
        ListHost,
        ListHostProps {
            files: vec![a, b],
            selected: 1,
        },
    );
    assert!(
        html.contains("compose.yaml") && html.contains(".env"),
        "{html}"
    );
    assert!(
        html.contains(">+2<") && html.contains(">\u{2212}1<"),
        "{html}"
    );
    assert_eq!(html.matches("dv-file-selected").count(), 1, "{html}");
    let selected_at = html.find("dv-file-selected").expect("selected");
    assert!(
        html[selected_at..].contains(".env"),
        "the second row is selected: {html}"
    );
}

/// A `.env` where one key got a new value and another was added: with the values masked the
/// changed line and the line it replaces would read the same, so the view says so.
fn changed_env_diff() -> berth_core::FileDiff {
    berth_rules::diff::file_diff(
        Path::new(".env"),
        Some("NODE_ENV=development\nDATABASE_URL=postgres://app:old-secret@db/app\nPORT=3000\n"),
        Some("NODE_ENV=development\nDATABASE_URL=postgres://app:new-secret@db/app\nPORT=3000\nREDIS_URL=redis://cache\n"),
    )
}

#[test]
fn review_changes_a_masked_line_whose_value_changed_says_so_without_the_value() {
    let view = file_view(&changed_env_diff(), false);
    assert!(view.masked);
    let masked_line = format!("DATABASE_URL={MASK}");
    let noted: Vec<(&str, &str)> = view
        .lines
        .iter()
        .filter_map(|l| l.note.map(|n| (l.text.as_str(), n)))
        .collect();
    // Only the added side of the changed key; the added REDIS_URL is new, not changed.
    assert_eq!(noted, [(masked_line.as_str(), "value changed")]);
    let removed = view
        .lines
        .iter()
        .find(|l| l.kind == berth_core::DiffLineKind::Removed)
        .expect("the removed line");
    assert_eq!(removed.text, masked_line);
    assert_eq!(removed.note, None);
    // No value anywhere in what the view holds.
    let all = format!("{view:?}");
    assert!(
        !all.contains("old-secret") && !all.contains("new-secret"),
        "{all}"
    );
}

#[test]
fn review_changes_the_changed_value_note_is_drawn_and_goes_when_the_file_is_revealed() {
    let masked = card(file_view(&changed_env_diff(), false), true);
    assert_eq!(masked.matches("class=\"dv-note\"").count(), 1, "{masked}");
    assert!(masked.contains(">value changed<"), "{masked}");
    assert!(!masked.contains("new-secret"));
    // Revealed, the lines show the values themselves and need no note.
    let revealed = file_view(&changed_env_diff(), true);
    assert!(revealed.lines.iter().all(|l| l.note.is_none()));
    let html = card(revealed, true);
    assert!(!html.contains("class=\"dv-note\""), "{html}");
    assert!(html.contains("new-secret"));
}

#[test]
fn review_changes_a_file_that_is_not_secret_never_gets_the_note() {
    let diff = berth_rules::diff::file_diff(
        Path::new("compose.yaml"),
        Some("name=old\n"),
        Some("name=new\n"),
    );
    assert!(file_view(&diff, false)
        .lines
        .iter()
        .all(|l| l.note.is_none()));
}
