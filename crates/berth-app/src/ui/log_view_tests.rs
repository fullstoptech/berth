use std::cell::RefCell;
use std::collections::BTreeMap;

use berth_core::{ContainerId, LogLine, Stream};
use berth_logs::LogFilter;
use chrono::{TimeZone, Utc};

use super::*;
use crate::services::logs::{use_log_session, LogCore, SeededLogs};
use crate::state::use_app_state;

fn line(second: i64, stderr: bool, text: &str) -> LogLine {
    LogLine {
        container: ContainerId("web".to_string()),
        ts: Utc
            .timestamp_opt(1_792_850_000 + second, 0)
            .single()
            .expect("ts"),
        stream: if stderr {
            Stream::Stderr
        } else {
            Stream::Stdout
        },
        text: text.to_string(),
    }
}

fn view_of(lines: Vec<LogLine>) -> LogView {
    let mut view = LogView::new(1000, BTreeMap::new());
    view.push_batch(lines);
    view
}

fn click(seq: u64, shift: bool, toggle: bool) -> RowClick {
    RowClick { seq, shift, toggle }
}

// ---- selection ----

#[test]
fn logs_view_a_plain_click_selects_one_row_and_sets_the_anchor() {
    let (spans, anchor) = click_selection(&[(1, 9)], Some(1), click(4, false, false));
    assert_eq!(spans, vec![(4, 4)]);
    assert_eq!(anchor, Some(4));
}

#[test]
fn logs_view_shift_click_selects_from_the_anchor_either_way() {
    let (spans, anchor) = click_selection(&[(4, 4)], Some(4), click(9, true, false));
    assert_eq!(spans, vec![(4, 9)]);
    assert_eq!(anchor, Some(4));
    let (spans, _) = click_selection(&[(4, 9)], Some(9), click(2, true, false));
    assert_eq!(spans, vec![(2, 9)]);
    // No anchor yet: the click is its own anchor.
    let (spans, anchor) = click_selection(&[], None, click(5, true, false));
    assert_eq!(spans, vec![(5, 5)]);
    assert_eq!(anchor, Some(5));
}

#[test]
fn logs_view_command_click_adds_one_row_or_splits_a_range() {
    let (spans, _) = click_selection(&[(1, 1)], Some(1), click(7, false, true));
    assert_eq!(spans, vec![(1, 1), (7, 7)]);
    // The same row again takes it out.
    let (spans, _) = click_selection(&spans, Some(7), click(7, false, true));
    assert_eq!(spans, vec![(1, 1)]);
    // A row inside a range leaves the rest on both sides.
    let (spans, _) = click_selection(&[(2, 8)], Some(2), click(5, false, true));
    assert_eq!(spans, vec![(2, 4), (6, 8)]);
    assert!(is_selected(&spans, 4) && !is_selected(&spans, 5) && is_selected(&spans, 6));
}

#[test]
fn logs_view_selected_rows_count_and_copy_follow_the_visible_rows() {
    // Rows with a gap in their sequence numbers: a filter hid seq 2.
    let mut view = view_of(vec![
        line(1, false, "INFO one"),
        line(2, true, "ERROR two"),
        line(3, false, "INFO three"),
        line(4, false, "INFO four"),
    ]);
    view.set_filter(LogFilter {
        stdout: true,
        stderr: false,
        ..LogFilter::default()
    });
    assert_eq!(view.len(), 3);
    // seq 0..=3 selected: three visible rows.
    assert_eq!(selected_rows(&view, &[(0, 3)]), 3);
    // Two separate rows.
    assert_eq!(selected_rows(&view, &[(0, 0), (3, 3)]), 2);
    assert_eq!(
        selected_text(&view, &[(0, 0), (3, 3)], false),
        "INFO one\nINFO four"
    );
    assert_eq!(
        selected_text(&view, &[(0, 3)], false),
        "INFO one\nINFO three\nINFO four"
    );
    assert_eq!(selected_rows(&view, &[]), 0);
    // A span past the end covers what is there.
    assert_eq!(selected_rows(&view, &[(2, 99)]), 2);
}

// ---- rows, tones, columns ----

#[test]
fn logs_view_trace_members_take_the_tone_of_their_head() {
    let mut view = view_of(vec![
        line(1, false, "INFO ok"),
        line(2, true, "Error: connect ECONNREFUSED"),
        line(2, true, "    at Object.connect (node:net:1139:12)"),
        line(2, true, "    at Pool.connect (/app/db.js:41:19)"),
        line(3, false, "WARN slow"),
    ]);
    // Open the trace so its members are rows.
    view.toggle_fold(1);
    let rows = view.rows(0..view.len());
    let tones = window_tones(&rows);
    let got: Vec<(RowTone, bool)> = tones;
    assert_eq!(
        got,
        vec![
            (RowTone::Plain, false),
            (RowTone::Error, true),
            (RowTone::Error, false),
            (RowTone::Error, false),
            (RowTone::Warn, false),
        ]
    );
    // Closed again: the head is not "expanded".
    view.toggle_fold(1);
    let closed = window_tones(&view.rows(0..view.len()));
    assert_eq!(closed[1], (RowTone::Error, false));
}

#[test]
fn logs_view_grid_has_a_column_per_visible_part() {
    let all = grid_columns(true, true);
    assert_eq!(all.matches("calc(").count(), 4);
    assert!(all.ends_with("minmax(0, 1fr)"));
    assert_eq!(grid_columns(false, false).matches("calc(").count(), 2);
    assert_eq!(grid_columns(true, false).matches("calc(").count(), 3);
}

#[test]
fn logs_view_row_height_scales_and_compact_is_smaller() {
    assert_eq!(row_px(false, 1.0), 22);
    assert_eq!(row_px(true, 1.0), 18);
    assert_eq!(row_px(false, 1.5), 33);
    assert!(row_px(true, 1.0) < row_px(false, 1.0));
}

#[test]
fn logs_view_grouped_counts_read_with_commas() {
    assert_eq!(grouped(0), "0");
    assert_eq!(grouped(999), "999");
    assert_eq!(grouped(1_000), "1,000");
    assert_eq!(grouped(1_234_567), "1,234,567");
}

#[test]
fn logs_view_follow_script_pins_the_scroller_it_names() {
    let script = follow_script(7);
    assert!(script.contains("log-scroll-7"));
    assert!(script.contains("MutationObserver"));
    assert!(script.contains("dioxus.send(\"unfollow\")"));
    assert_eq!(scroller_id(7), "log-scroll-7");
}

// ---- F8 ----

#[test]
fn logs_view_f8_goes_to_the_next_error_and_shift_f8_to_the_previous_with_wrap() {
    let view = view_of(vec![
        line(1, false, "INFO a"),
        line(2, true, "ERROR b"),
        line(3, false, "INFO c"),
        line(4, true, "ERROR d"),
        line(5, false, "INFO e"),
    ]);
    // From nothing on screen: the first error at or below the top.
    assert_eq!(error_target(&view, true, None, 0.0, 22), Some((1, 1)));
    // From the focused error: the next one, then wrapping.
    assert_eq!(error_target(&view, true, Some(1), 0.0, 22), Some((3, 3)));
    assert_eq!(error_target(&view, true, Some(3), 0.0, 22), Some((1, 1)));
    // Backwards.
    assert_eq!(error_target(&view, false, Some(3), 0.0, 22), Some((1, 1)));
    assert_eq!(error_target(&view, false, Some(1), 0.0, 22), Some((3, 3)));
    // Scrolled below the first error with nothing focused: the next error from the top of the
    // screen (row 2 is the first visible row, so row 3 is the answer).
    assert_eq!(error_target(&view, true, None, 44.0, 22), Some((3, 3)));
    // No errors at all.
    let quiet = view_of(vec![line(1, false, "INFO a")]);
    assert_eq!(error_target(&quiet, true, None, 0.0, 22), None);
}

// ---- component output ----

thread_local! {
    static VIEW: RefCell<Option<(LogView, ViewPrefs)>> = const { RefCell::new(None) };
}

#[component]
fn Host() -> Element {
    let state = use_app_state();
    use_context_provider(|| state);
    let (view, prefs) = VIEW
        .with(|slot| slot.borrow_mut().take())
        .expect("a view to render");
    use_hook(move || provide_context(SeededLogs(LogCore::seeded(view))));
    let handle = use_log_session(Vec::new());
    rsx! { LogViewer { handle, view_name: "web".to_string(), prefs } }
}

fn render(view: LogView, prefs: ViewPrefs) -> String {
    VIEW.with(|slot| *slot.borrow_mut() = Some((view, prefs)));
    let mut dom = VirtualDom::new(Host);
    let _ = dom.rebuild_to_vec();
    dioxus_ssr::render(&dom)
}

#[test]
fn logs_view_shows_the_dropped_notice_only_when_lines_were_dropped() {
    let mut view = view_of(vec![line(1, false, "INFO a")]);
    let quiet = render(
        view_of(vec![line(1, false, "INFO a")]),
        ViewPrefs::default(),
    );
    assert!(!quiet.contains("Older lines dropped"), "{quiet}");
    view.add_dropped(1234);
    let html = render(view, ViewPrefs::default());
    assert!(
        html.contains("Older lines dropped; Save to keep all"),
        "{html}"
    );
    assert!(html.contains("data-dropped=\"1234\""));
}

#[test]
fn logs_view_raw_shows_json_as_written_and_default_collapses_it() {
    let json = r#"{"level":"info","msg":"GET /health","status":200}"#;
    let collapsed = render(view_of(vec![line(1, false, json)]), ViewPrefs::default());
    assert!(
        collapsed.contains("{ level: info, msg: GET /health, status: 200 }"),
        "{collapsed}"
    );
    let raw = render(
        view_of(vec![line(1, false, json)]),
        ViewPrefs {
            raw: true,
            ..ViewPrefs::default()
        },
    );
    assert!(raw.contains("&#34;msg&#34;:&#34;GET /health&#34;"), "{raw}");
    assert!(!raw.contains("{ level: info"));
}

#[test]
fn logs_view_wrap_and_time_toggles_change_the_list_and_columns() {
    let plain = render(
        view_of(vec![line(1, false, "INFO a")]),
        ViewPrefs::default(),
    );
    assert!(plain.contains("class=\"log-list\""), "{plain}");
    assert!(plain.contains("log-ts"));
    let wrapped = render(
        view_of(vec![line(1, false, "INFO a")]),
        ViewPrefs {
            wrap: true,
            time: false,
            ..ViewPrefs::default()
        },
    );
    assert!(wrapped.contains("class=\"log-list log-wrap\""), "{wrapped}");
    assert!(!wrapped.contains("class=\"log-ts\""));
}

#[test]
fn logs_view_an_empty_view_says_so() {
    let html = render(view_of(Vec::new()), ViewPrefs::default());
    assert!(html.contains("No lines yet"), "{html}");
}

#[test]
fn logs_view_search_shows_the_count_and_a_missing_match() {
    let mut view = view_of(vec![
        line(1, false, "INFO one needle"),
        line(2, false, "INFO two needle"),
    ]);
    let _ = view.set_search(Some(Query {
        text: "needle".to_string(),
        regex: false,
        case_sensitive: false,
    }));
    let html = render(
        view,
        ViewPrefs {
            query: "needle".to_string(),
            ..ViewPrefs::default()
        },
    );
    assert!(html.contains(">2 matches<"), "{html}");
    assert_eq!(html.matches("class=\"log-match\"").count(), 2);

    let mut none = view_of(vec![line(1, false, "INFO one")]);
    let _ = none.set_search(Some(Query {
        text: "absent".to_string(),
        regex: false,
        case_sensitive: false,
    }));
    let html = render(
        none,
        ViewPrefs {
            query: "absent".to_string(),
            ..ViewPrefs::default()
        },
    );
    assert!(html.contains("No matches"), "{html}");
    assert!(html.contains("search-count search-count-none"));
}

#[test]
fn logs_view_the_stream_choice_filters_the_rows() {
    let mut view = view_of(vec![line(1, false, "INFO out"), line(2, true, "WARN err")]);
    view.set_filter(build_filter(
        &BTreeSet::new(),
        StreamChoice::Stderr,
        TimeRange::All,
        Utc::now(),
    ));
    let html = render(
        view,
        ViewPrefs {
            stream: StreamChoice::Stderr,
            ..ViewPrefs::default()
        },
    );
    assert!(html.contains("WARN err"));
    assert!(!html.contains("INFO out"));
    assert!(
        html.contains("<option value=\"stderr\" selected=true>"),
        "{html}"
    );
}

#[test]
fn logs_view_level_chips_show_counts_and_the_active_one_is_pressed() {
    let view = view_of(vec![
        line(1, true, "ERROR a"),
        line(2, true, "ERROR b"),
        line(3, false, "WARN c"),
        line(4, false, "INFO d"),
    ]);
    let html = render(
        view,
        ViewPrefs {
            levels: [Level::Error].into_iter().collect(),
            ..ViewPrefs::default()
        },
    );
    assert!(
        html.contains("<span>Error</span><span class=\"segment-count\">2</span>"),
        "{html}"
    );
    assert!(html.contains("<span>Warn</span><span class=\"segment-count\">1</span>"));
    assert!(
        html.contains("class=\"segment segment-selected\" type=\"button\" aria-pressed=\"true\""),
        "{html}"
    );
}

thread_local! {
    static JUMPS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

#[component]
fn Presser() -> Element {
    use_hook(|| {
        let before = ERROR_JUMP.peek().0;
        dispatch("logs.next_error");
        let after_next = *ERROR_JUMP.peek();
        dispatch("logs.prev_error");
        let after_prev = *ERROR_JUMP.peek();
        dispatch("logs.unknown");
        let unchanged = *ERROR_JUMP.peek();
        JUMPS.with(|j| {
            *j.borrow_mut() = vec![
                before,
                after_next.0,
                u64::from(after_next.1),
                after_prev.0,
                u64::from(after_prev.1),
                unchanged.0,
            ];
        });
    });
    rsx! { div {} }
}

#[test]
fn logs_view_f8_presses_reach_the_view_through_the_key_dispatch() {
    let mut dom = VirtualDom::new(Presser);
    let _ = dom.rebuild_to_vec();
    let got = JUMPS.with(|j| j.borrow().clone());
    // [count before, count after F8, forward?, count after Shift+F8, forward?, count after
    // an action that is not for the log view]
    assert_eq!(got, vec![0, 1, 1, 2, 0, 2]);
}
