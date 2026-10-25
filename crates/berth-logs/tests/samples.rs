//! The sample logs in `fixtures/logs/`: one table per format, and one folding check per trace kind.
#![allow(clippy::expect_used)] // clippy.toml allows expect in #[test] fns, not in helpers

use std::collections::BTreeMap;

use berth_core::{ContainerId, LogLine, Stream};
use berth_logs::{
    classify, is_stack_continuation, summarize_json, Fold, Level, LogBuffer, LogFilter, LogView,
};
use chrono::DateTime;

const PLAIN: &str = include_str!("../../../fixtures/logs/plain_levels.txt");
const LOGFMT: &str = include_str!("../../../fixtures/logs/logfmt.txt");
const JSON: &str = include_str!("../../../fixtures/logs/json.txt");
const NODE: &str = include_str!("../../../fixtures/logs/node_trace.txt");
const PYTHON: &str = include_str!("../../../fixtures/logs/python_trace.txt");
const JAVA: &str = include_str!("../../../fixtures/logs/java_trace.txt");
const GO: &str = include_str!("../../../fixtures/logs/go_panic.txt");
const NGINX: &str = include_str!("../../../fixtures/logs/nginx_access.txt");
const POSTGRES: &str = include_str!("../../../fixtures/logs/postgres.txt");

fn log_line(text: &str) -> LogLine {
    LogLine {
        container: ContainerId("sample".to_string()),
        ts: DateTime::from_timestamp(1_761_297_240, 0).expect("ts"),
        stream: Stream::Stdout,
        text: text.to_string(),
    }
}

/// (level, error_mark) for every line of a sample, read as stdout.
fn table(sample: &str) -> Vec<(Option<Level>, bool)> {
    sample
        .lines()
        .map(|l| classify(l, Stream::Stdout))
        .collect()
}

fn buffer_of(sample: &str) -> LogBuffer {
    let mut buf = LogBuffer::new(1000);
    for l in sample.lines() {
        buf.push(log_line(l), 1);
    }
    buf
}

fn folds(sample: &str) -> Vec<Fold> {
    let buf = buffer_of(sample);
    (0..buf.len() as u64)
        .map(|seq| buf.by_seq(seq).expect("entry").fold)
        .collect()
}

use Level::{Debug, Error, Info, Trace, Warn};

#[test]
fn plain_level_formats_classify_per_line() {
    let want = vec![
        (Some(Info), false),
        (Some(Debug), false),
        (Some(Trace), false),
        (Some(Warn), false),
        (Some(Error), true),
        (Some(Warn), false),
        (Some(Error), true),
        (Some(Info), false),
        (Some(Error), true),
        (Some(Error), true),
        (Some(Warn), false),
        (Some(Info), false),
        (Some(Error), true),
        (Some(Warn), false),
        (Some(Info), false),
        (Some(Error), true),
        (None, false),
        (None, true),
        (None, false),
    ];
    assert_eq!(table(PLAIN), want);
}

#[test]
fn logfmt_lines_classify_per_line() {
    let want = vec![
        (Some(Info), false),
        (Some(Debug), false),
        (Some(Warn), false),
        (Some(Error), true),
        (Some(Error), true),
        (Some(Info), false),
        (Some(Warn), false),
        (Some(Info), false),
    ];
    assert_eq!(table(LOGFMT), want);
}

#[test]
fn json_lines_classify_by_level_severity_and_pino_numbers() {
    let want = vec![
        (Some(Info), false),
        (Some(Error), true),
        (Some(Warn), false),
        (Some(Error), true),
        (Some(Debug), false),
        (None, false),
        (Some(Info), false),
        (None, false),
        (None, false),
    ];
    assert_eq!(table(JSON), want);
}

#[test]
fn json_summaries_for_the_sample() {
    let lines: Vec<&str> = JSON.lines().collect();
    let first = summarize_json(lines[0]).expect("json");
    assert_eq!(first.message.as_deref(), Some("server started"));
    assert_eq!(first.fields[0], ("port".to_string(), "8080".to_string()));

    let severity = summarize_json(lines[1]).expect("json");
    assert_eq!(severity.message.as_deref(), Some("payment failed"));

    let nested = summarize_json(lines[4]).expect("json");
    assert_eq!(nested.fields[0].1, r#"{"user":"anna","roles":["a","b"]}"#);
    assert_eq!(nested.fields[1], ("ok".to_string(), "null".to_string()));

    let wide = summarize_json(lines[5]).expect("json");
    assert_eq!(wide.fields.len(), 6, "eight other fields, six kept");

    assert!(summarize_json(lines[7]).is_none(), "broken JSON");
    assert!(summarize_json(lines[8]).is_none(), "an array");
}

#[test]
fn nginx_access_and_error_lines() {
    let want = vec![
        (None, false),
        (None, false),
        (None, false),
        (Some(Error), true),
        (Some(Warn), false),
        (Some(Info), false),
    ];
    assert_eq!(table(NGINX), want);
}

#[test]
fn postgres_lines() {
    let want = vec![
        (Some(Info), false),
        (Some(Error), true),
        (None, false),
        (Some(Warn), false),
        (Some(Error), true),
        (Some(Info), false),
        (Some(Debug), false),
    ];
    assert_eq!(table(POSTGRES), want);
}

#[test]
fn stderr_marks_error_words_even_under_an_info_level() {
    assert_eq!(
        classify("INFO retry after error", Stream::Stderr),
        (Some(Info), true)
    );
    assert_eq!(
        classify("INFO retry after error", Stream::Stdout),
        (Some(Info), false)
    );
}

#[test]
fn node_trace_folds_under_the_error_line() {
    assert_eq!(
        folds(NODE),
        vec![
            Fold::None,
            Fold::Head { hidden: 3 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::None,
        ]
    );
    let head = buffer_of(NODE).by_seq(1).map(|e| e.error_mark);
    assert_eq!(head, Some(true));
}

#[test]
fn python_traceback_folds_frames_and_source_lines_but_not_the_exception_line() {
    assert_eq!(
        folds(PYTHON),
        vec![
            Fold::None,
            Fold::Head { hidden: 4 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::None,
            Fold::None,
        ]
    );
    let buf = buffer_of(PYTHON);
    assert!(
        buf.by_seq(6).is_some_and(|e| e.error_mark),
        "ValueError line"
    );
}

#[test]
fn java_trace_folds_at_lines_caused_by_and_more() {
    assert_eq!(
        folds(JAVA),
        vec![
            Fold::None,
            Fold::Head { hidden: 5 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::None,
        ]
    );
}

#[test]
fn go_panic_folds_goroutine_frames_and_stops_at_exit_status() {
    assert_eq!(
        folds(GO),
        vec![
            Fold::None,
            Fold::Head { hidden: 6 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::Member { head: 1 },
            Fold::None,
            Fold::None,
        ]
    );
}

#[test]
fn trace_detection_agrees_with_the_samples() {
    let is_trace =
        |sample: &str| -> usize { sample.lines().filter(|l| is_stack_continuation(l)).count() };
    assert_eq!(is_trace(NODE), 3);
    assert_eq!(
        is_trace(PYTHON),
        2,
        "File lines only; source lines need context"
    );
    assert_eq!(is_trace(JAVA), 5);
    assert_eq!(is_trace(PLAIN) + is_trace(NGINX) + is_trace(POSTGRES), 0);
}

#[test]
fn every_sample_through_one_view_reports_what_the_chips_need() {
    let mut view = LogView::new(1000, BTreeMap::new());
    let all = [PLAIN, LOGFMT, JSON, NODE, PYTHON, JAVA, GO, NGINX, POSTGRES];
    for sample in all {
        view.push_batch(sample.lines().map(log_line).collect());
    }
    let total: usize = all.iter().map(|s| s.lines().count()).sum();
    let hidden = 3 + 4 + 5 + 6;
    assert_eq!(view.len(), total - hidden);
    assert_eq!(view.dropped(), 0);
    assert!(view.counts().error >= 10);
    view.set_filter(LogFilter {
        stdout: true,
        stderr: true,
        levels: [Error].into_iter().collect(),
        ..LogFilter::default()
    });
    assert!(view.rows(0..view.len()).iter().all(|r| r.error_mark));
}
