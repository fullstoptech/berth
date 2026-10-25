//! Level and error-mark detection for one line of text.

use std::sync::LazyLock;

use berth_core::Stream;
use regex::Regex;

use crate::{summarize_json, JsonSummary, Level};

/// Longest prefix of a line that is scanned. A minified 1 MB blob must not cost a millisecond.
const SCAN_LIMIT: usize = 16 * 1024;

/// The first level token in the line wins. Alternatives, in order of appearance in the pattern:
/// glog `E1024 12:00:00`, logfmt `level=info`, `[warn]`, a bare upper-case word (`ERROR`,
/// `WARNING:`), and Postgres `LOG:`.
const LEVEL_PATTERN: &str = r#"(?x)
    ^\s*(?P<glog>[EWIDF])\d{4}\s\d
  | (?i:\b(?:level|lvl|severity|loglevel)=["']?)(?P<kv>[A-Za-z0-9]+)
  | \[\s*(?P<br>[A-Za-z]+)\s*\]
  | \b(?P<bare>FATAL|CRITICAL|CRIT|PANIC|SEVERE|ERROR|ERR|WARNING|WARN|NOTICE|INFO|DEBUG[1-5]?|TRACE|FINEST|FINER|FINE)\b
  | \b(?P<log>LOG):
"#;

/// `error`, `panic`, `exception`, `fatal`, `traceback`; `ValueError` and `NullPointerException`
/// count, `errors` and `error_count` do not.
const ERROR_WORD_PATTERN: &str = r"(?i)\b(?:\w*error|\w*exception|panic|fatal|traceback)\b";

// A pattern that fails to compile leaves `None`; every caller then finds nothing. The patterns are
// constants and `patterns_compile` below fails if one stops compiling.
static LEVEL_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(LEVEL_PATTERN).ok());
static ERROR_WORD_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(ERROR_WORD_PATTERN).ok());

/// Maps a level word (any case) from a log format to a [`Level`].
pub(crate) fn level_from_word(word: &str) -> Option<Level> {
    const TABLE: &[(&str, Level)] = &[
        ("error", Level::Error),
        ("err", Level::Error),
        ("fatal", Level::Error),
        ("crit", Level::Error),
        ("critical", Level::Error),
        ("alert", Level::Error),
        ("emerg", Level::Error),
        ("emergency", Level::Error),
        ("panic", Level::Error),
        ("severe", Level::Error),
        ("warn", Level::Warn),
        ("warning", Level::Warn),
        ("info", Level::Info),
        ("notice", Level::Info),
        ("information", Level::Info),
        ("log", Level::Info),
        ("debug", Level::Debug),
        ("fine", Level::Debug),
        ("trace", Level::Trace),
        ("finer", Level::Trace),
        ("finest", Level::Trace),
    ];
    let word = word.trim_end_matches(|c: char| c.is_ascii_digit());
    TABLE
        .iter()
        .find(|(name, _)| word.eq_ignore_ascii_case(name))
        .map(|(_, level)| *level)
}

fn glog_level(letter: &str) -> Option<Level> {
    match letter {
        "E" | "F" => Some(Level::Error),
        "W" => Some(Level::Warn),
        "I" => Some(Level::Info),
        "D" => Some(Level::Debug),
        _ => None,
    }
}

fn head(text: &str) -> &str {
    let mut end = text.len().min(SCAN_LIMIT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn scan_level(text: &str) -> Option<Level> {
    let re = LEVEL_RE.as_ref()?;
    re.captures_iter(head(text)).find_map(|caps| {
        if let Some(m) = caps.name("glog") {
            glog_level(m.as_str())
        } else if let Some(m) = caps.name("kv").or_else(|| caps.name("br")) {
            level_from_word(m.as_str())
        } else if let Some(m) = caps.name("bare") {
            level_from_word(m.as_str())
        } else {
            caps.name("log").and(Some(Level::Info))
        }
    })
}

fn has_error_word(text: &str) -> bool {
    ERROR_WORD_RE
        .as_ref()
        .is_some_and(|re| re.is_match(head(text)))
}

/// Level and error mark from an already parsed JSON summary, or from the text.
///
/// A line is an error line when its level is `Error`. Otherwise an error word marks it when the
/// line is on stderr or carries no level; an explicit `INFO`/`WARN`/`DEBUG` on stdout is trusted
/// ("error budget is healthy" is not an error).
pub(crate) fn classify_with(
    text: &str,
    stream: Stream,
    json: Option<&JsonSummary>,
) -> (Option<Level>, bool) {
    let level = match json {
        Some(summary) => summary.level,
        None => scan_level(text),
    };
    let error_mark = match level {
        Some(Level::Error) => true,
        Some(_) => stream == Stream::Stderr && has_error_word(text),
        None => has_error_word(text),
    };
    (level, error_mark)
}

/// Level from common formats: `ERROR`, `[warn]`, `level=info`, `E1024`, JSON `level`/`severity`;
/// `error_mark` when the level is `Error`, or an error word appears on stderr or on a line with no
/// level.
#[must_use]
pub fn classify(text: &str, stream: Stream) -> (Option<Level>, bool) {
    let json = summarize_json(text);
    classify_with(text, stream, json.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(text: &str) -> Option<Level> {
        classify(text, Stream::Stdout).0
    }

    #[test]
    fn patterns_compile() {
        assert!(LEVEL_RE.is_some());
        assert!(ERROR_WORD_RE.is_some());
    }

    #[test]
    fn level_words_in_each_common_format() {
        let table = [
            ("2026-10-24 INFO  server listening", Some(Level::Info)),
            ("DEBUG cache warmed", Some(Level::Debug)),
            ("TRACE tick", Some(Level::Trace)),
            ("WARN slow request", Some(Level::Warn)),
            ("WARNING:app:token", Some(Level::Warn)),
            ("ERROR:root:declined", Some(Level::Error)),
            ("FATAL: out of memory", Some(Level::Error)),
            ("SEVERE: cannot bind", Some(Level::Error)),
            ("[warn] disk usage", Some(Level::Warn)),
            ("[error] worker exited", Some(Level::Error)),
            ("[INFO] scheduler", Some(Level::Info)),
            ("[notice] 1#1: signal", Some(Level::Info)),
            (
                "E1024 09:14:05.123456 1 c.go:88] sync failed",
                Some(Level::Error),
            ),
            (
                "W1024 09:14:05.223456 1 c.go:91] retrying",
                Some(Level::Warn),
            ),
            ("I1024 09:14:05.323456 1 c.go:95] synced", Some(Level::Info)),
            ("level=warn msg=\"x\"", Some(Level::Warn)),
            ("ts=1 level=\"error\" msg=x", Some(Level::Error)),
            ("severity=WARNING msg=x", Some(Level::Warn)),
            (
                "2026 UTC [1] LOG:  database system is ready",
                Some(Level::Info),
            ),
            (
                "2026 UTC [9] DEBUG1:  checkpoint starting",
                Some(Level::Debug),
            ),
            ("Listening on http://0.0.0.0:3000", None),
            (
                "the cache is fully warmed and information is plentiful",
                None,
            ),
            ("retrying after an error in the upstream", None),
            ("[Node] started", None),
        ];
        for (text, want) in table {
            assert_eq!(level(text), want, "{text}");
        }
    }

    #[test]
    fn first_level_token_wins() {
        assert_eq!(
            level("level=info msg=\"ERROR in template\""),
            Some(Level::Info)
        );
    }

    #[test]
    fn error_mark_follows_level_stream_and_words() {
        use Stream::{Stderr, Stdout};
        let table = [
            ("ERROR boom", Stdout, true),
            ("[error] worker exited", Stdout, true),
            ("panic: runtime error: index out of range", Stderr, true),
            ("Traceback (most recent call last):", Stdout, true),
            ("ValueError: bad payload", Stderr, true),
            ("java.lang.IllegalStateException: pool", Stdout, true),
            ("INFO error budget is healthy", Stdout, false),
            ("INFO error budget is healthy", Stderr, true),
            ("WARN retrying after fatal glitch", Stdout, false),
            ("plain line", Stderr, false),
            ("started 0 errors, error_count=0", Stdout, false),
        ];
        for (text, stream, want) in table {
            assert_eq!(classify(text, stream).1, want, "{text} on {stream:?}");
        }
    }

    #[test]
    fn json_level_decides_and_text_is_not_scanned_for_tokens() {
        assert_eq!(
            classify(r#"{"level":"warn","msg":"ERROR in text"}"#, Stream::Stdout),
            (Some(Level::Warn), false)
        );
        assert_eq!(
            classify(r#"{"severity":"CRITICAL","message":"x"}"#, Stream::Stdout),
            (Some(Level::Error), true)
        );
    }

    #[test]
    fn long_lines_are_cut_on_a_char_boundary() {
        let text = format!("{}é WARN tail", "x".repeat(SCAN_LIMIT - 1));
        assert_eq!(level(&text), None);
        assert_eq!(level("é WARN"), Some(Level::Warn));
    }
}
