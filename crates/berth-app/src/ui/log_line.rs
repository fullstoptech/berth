//! One row of a log view (PLAN §7.6, frames "Logs / errors / 1440" and "Logs / paused / 1440").
//! Rows are a fixed height so the list can be virtualized; error and warn lines carry a gutter
//! bar and the whole-row tint, JSON lines collapse to `{ level: info, msg: … }`, and a stack
//! trace folds under its head with "N more lines".

use berth_core::Stream;
use berth_logs::{Fold, JsonSummary, Level, RowModel};
use dioxus::prelude::*;

use super::icon::{Icon, IconSize};

/// How a row is tinted. A trace member takes the tone of its head, which the list works out
/// (a row alone cannot know it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowTone {
    #[default]
    Plain,
    Warn,
    Error,
}

impl RowTone {
    fn class(self) -> &'static str {
        match self {
            Self::Plain => "",
            Self::Warn => " log-warn",
            Self::Error => " log-error",
        }
    }
}

/// The tone a row has on its own: error when it carries the error mark or an error level, warn
/// for a warn level.
#[must_use]
pub fn row_tone(row: &RowModel) -> RowTone {
    if row.error_mark || row.level == Some(Level::Error) {
        RowTone::Error
    } else if row.level == Some(Level::Warn) {
        RowTone::Warn
    } else {
        RowTone::Plain
    }
}

/// What the list knows about a row that the row does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowOpts {
    pub tone: RowTone,
    /// Timestamp column on.
    pub time: bool,
    /// Container column on (merged views).
    pub container: bool,
    /// Raw: JSON lines are not collapsed.
    pub raw: bool,
    /// Long lines wrap instead of being cut with an ellipsis.
    pub wrap: bool,
    pub selected: bool,
    /// The row F8 or search last jumped to.
    pub focused: bool,
    /// This JSON line was opened to its raw text.
    pub json_open: bool,
    /// This head's trace is open (its members follow as rows).
    pub expanded: bool,
}

/// A click on a row, with the modifier keys that decide how the selection changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowClick {
    pub seq: u64,
    pub shift: bool,
    /// ⌘ or ⌃: add or remove this row.
    pub toggle: bool,
}

/// `{ level: info, msg: GET /health, status: 200, ms: 2.1 }`
#[must_use]
pub fn json_summary_text(summary: &JsonSummary) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(level) = summary.level {
        parts.push(format!("level: {}", level_word(level)));
    }
    if let Some(message) = &summary.message {
        parts.push(format!("msg: {message}"));
    }
    for (key, value) in &summary.fields {
        parts.push(format!("{key}: {value}"));
    }
    format!("{{ {} }}", parts.join(", "))
}

fn level_word(level: Level) -> &'static str {
    match level {
        Level::Error => "error",
        Level::Warn => "warn",
        Level::Info => "info",
        Level::Debug => "debug",
        Level::Trace => "trace",
    }
}

/// "11 more lines", "1 more line".
#[must_use]
pub fn more_lines(hidden: u32) -> String {
    if hidden == 1 {
        "1 more line".to_string()
    } else {
        format!("{hidden} more lines")
    }
}

/// Does this row show the collapsed JSON summary? Not in Raw, not when opened, not for
/// attention lines (they show their text, as in the frame), and not while a search matches the
/// raw text.
#[must_use]
pub fn shows_json_summary(row: &RowModel, opts: &RowOpts) -> bool {
    row.json.is_some()
        && !opts.raw
        && !opts.json_open
        && opts.tone == RowTone::Plain
        && row.matches.is_empty()
}

/// Can this row's JSON be opened or closed with a chevron?
#[must_use]
pub fn has_json_chevron(row: &RowModel, opts: &RowOpts) -> bool {
    row.json.is_some() && !opts.raw && opts.tone == RowTone::Plain && row.matches.is_empty()
}

/// `text` cut at the match ranges: `(piece, is_match)`. Ranges past the end or off a character
/// boundary are skipped, so a stale range can never panic.
#[must_use]
pub fn segments<'a>(text: &'a str, matches: &[std::ops::Range<usize>]) -> Vec<(&'a str, bool)> {
    let mut out = Vec::new();
    let mut at = 0;
    for range in matches {
        if range.start < at
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            continue;
        }
        if range.start > at {
            out.push((&text[at..range.start], false));
        }
        if range.end > range.start {
            out.push((&text[range.start..range.end], true));
        }
        at = range.end;
    }
    if at < text.len() || out.is_empty() {
        out.push((&text[at..], false));
    }
    out
}

/// One log line.
#[component]
pub fn LogLine(
    row: RowModel,
    opts: RowOpts,
    #[props(default)] onselect: Option<EventHandler<RowClick>>,
    #[props(default)] ontoggle_fold: Option<EventHandler<u64>>,
    #[props(default)] ontoggle_json: Option<EventHandler<u64>>,
) -> Element {
    let seq = row.seq;
    let member = matches!(row.fold, Fold::Member { .. });
    let mut class = format!("log-row{}", opts.tone.class());
    if member {
        class.push_str(" log-member");
    }
    if opts.selected {
        class.push_str(" log-selected");
    }
    if opts.focused {
        class.push_str(" log-focused");
    }
    let summary = shows_json_summary(&row, &opts)
        .then(|| row.json.as_ref().map(json_summary_text))
        .flatten();
    let chevron = has_json_chevron(&row, &opts);
    let hidden = match row.fold {
        Fold::Head { hidden } if hidden > 0 => Some(hidden),
        _ => None,
    };
    let stderr = row.stream == Stream::Stderr;
    let json_label = if opts.json_open {
        "Collapse JSON"
    } else {
        "Expand JSON"
    };
    let json_icon = if opts.json_open {
        "chevron-down"
    } else {
        "chevron-right"
    };
    let fold_icon = if opts.expanded {
        "chevron-down"
    } else {
        "chevron-right"
    };
    let series = row.series.clamp(1, 5);
    let pieces = if summary.is_some() {
        Vec::new()
    } else {
        segments(&row.text, &row.matches)
    };
    rsx! {
        div {
            class: "{class}",
            role: "row",
            "aria-selected": "{opts.selected}",
            "data-seq": "{seq}",
            onclick: move |e| {
                use dioxus::html::ModifiersInteraction;
                let mods = e.data.modifiers();
                if let Some(handler) = &onselect {
                    handler.call(RowClick {
                        seq,
                        shift: mods.contains(dioxus::html::Modifiers::SHIFT),
                        toggle: mods.contains(dioxus::html::Modifiers::META)
                            || mods.contains(dioxus::html::Modifiers::CONTROL),
                    });
                }
            },
            span { class: "log-gutter", "aria-hidden": "true" }
            if opts.time {
                span { class: "log-ts", "{row.ts}" }
            }
            if opts.container {
                span {
                    class: "log-src",
                    style: "color: var(--color-series-{series});",
                    title: "{row.container}",
                    "{row.container}"
                }
            }
            span { class: "log-stream", if stderr { "stderr" } }
            span { class: "log-text",
                if chevron {
                    button {
                        class: "log-chev",
                        r#type: "button",
                        "aria-label": "{json_label}",
                        onclick: move |e| {
                            e.stop_propagation();
                            if let Some(handler) = &ontoggle_json {
                                handler.call(seq);
                            }
                        },
                        Icon {
                            name: json_icon,
                            size: IconSize::Sm,
                        }
                    }
                }
                if let Some(text) = &summary {
                    span { class: "log-json", "{text}" }
                } else {
                    for (index, (piece, is_match)) in pieces.iter().enumerate() {
                        if *is_match {
                            mark { key: "{index}", class: "log-match", "{piece}" }
                        } else {
                            span { key: "{index}", "{piece}" }
                        }
                    }
                }
                if let Some(hidden) = hidden {
                    button {
                        class: "log-fold",
                        r#type: "button",
                        "aria-expanded": "{opts.expanded}",
                        onclick: move |e| {
                            e.stop_propagation();
                            if let Some(handler) = &ontoggle_fold {
                                handler.call(seq);
                            }
                        },
                        Icon {
                            name: fold_icon,
                            size: IconSize::Sm,
                        }
                        span { "{more_lines(hidden)}" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(text: &str) -> RowModel {
        RowModel {
            seq: 1,
            ts: "14:02:12.031".to_string(),
            container: "shop-api-worker-1".to_string(),
            series: 2,
            stream: Stream::Stdout,
            level: None,
            error_mark: false,
            text: text.to_string(),
            json: None,
            fold: Fold::None,
            matches: Vec::new(),
        }
    }

    #[test]
    fn logs_line_json_summary_reads_level_message_then_fields() {
        let summary = berth_logs::summarize_json(
            r#"{"level":"info","msg":"GET /health","status":200,"ms":2.1}"#,
        )
        .expect("json");
        assert_eq!(
            json_summary_text(&summary),
            "{ level: info, msg: GET /health, status: 200, ms: 2.1 }"
        );
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn logs_line_segments_split_at_matches_and_ignore_bad_ranges() {
        let text = "Error: connect ECONNREFUSED 172.19.0.3:5432";
        assert_eq!(
            segments(text, &[15..27]),
            vec![
                ("Error: connect ", false),
                ("ECONNREFUSED", true),
                (" 172.19.0.3:5432", false)
            ]
        );
        assert_eq!(segments(text, &[100..120]), vec![(text, false)]);
        // Inside a multi-byte character: skipped, not a panic.
        assert_eq!(segments("é!", &[1..2]), vec![("é!", false)]);
        assert_eq!(segments("", &[]), vec![("", false)]);
    }

    #[test]
    fn logs_line_tone_follows_the_error_mark_and_level() {
        let mut r = row("x");
        assert_eq!(row_tone(&r), RowTone::Plain);
        r.level = Some(Level::Warn);
        assert_eq!(row_tone(&r), RowTone::Warn);
        r.level = None;
        r.error_mark = true;
        assert_eq!(row_tone(&r), RowTone::Error);
        r.error_mark = false;
        r.level = Some(Level::Error);
        assert_eq!(row_tone(&r), RowTone::Error);
    }

    #[test]
    fn logs_line_more_lines_is_singular_for_one() {
        assert_eq!(more_lines(1), "1 more line");
        assert_eq!(more_lines(11), "11 more lines");
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn logs_line_json_summary_shows_only_for_plain_unmatched_non_raw_rows() {
        let mut r = row(r#"{"level":"info","msg":"ok"}"#);
        r.json = berth_logs::summarize_json(&r.text);
        let plain = RowOpts::default();
        assert!(shows_json_summary(&r, &plain));
        assert!(!shows_json_summary(&r, &RowOpts { raw: true, ..plain }));
        assert!(!shows_json_summary(
            &r,
            &RowOpts {
                json_open: true,
                ..plain
            }
        ));
        assert!(!shows_json_summary(
            &r,
            &RowOpts {
                tone: RowTone::Warn,
                ..plain
            }
        ));
        r.matches = vec![0..2];
        assert!(!shows_json_summary(&r, &plain));
    }
}
