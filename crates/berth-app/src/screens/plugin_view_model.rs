//! The pure half of the plugin view screen (P9-T5): what a protocol `View`
//! turns into before it is drawn. Column tracks, row filtering, the header of
//! a selected row, the shape of a confirm, and the rows of a log. Nothing
//! here reads a signal, so each rule is tested without a DOM.

use std::collections::BTreeMap;

use berth_core::{ContainerId, LogLine, Stream};
use berth_logs::{LogView, RowModel};
use berth_plugin_protocol::{Badge, Cell, Column, Confirm, Row, Tone, View};
use chrono::{DateTime, TimeZone, Utc};

use crate::services::plugin_views::{LogState, MAX_LOG_LINES};
use crate::ui::log_line::{row_tone, RowTone};
use crate::ui::BannerTone;

// ---------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------

/// The text a cell shows (a badge shows its text; a time is shown by [`time_text`]).
#[must_use]
pub fn cell_text(cell: &Cell) -> &str {
    match cell {
        Cell::Text(t) | Cell::Mono(t) | Cell::Time(t) => t,
        Cell::Badge(b) => &b.text,
    }
}

/// A time cell as a person reads it: `Oct 24, 09:28` in `tz`, or the plugin's
/// own text when it is not an RFC 3339 time.
#[must_use]
pub fn time_text<Tz: TimeZone>(iso: &str, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match DateTime::parse_from_rfc3339(iso) {
        Ok(at) => at.with_timezone(tz).format("%b %-d, %H:%M").to_string(),
        Err(_) => iso.to_string(),
    }
}

/// Does `row` match every word of `filter` in some cell? An empty filter
/// matches every row.
#[must_use]
pub fn row_matches(row: &Row, filter: &str) -> bool {
    let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    words.iter().all(|word| {
        row.cells
            .values()
            .any(|cell| cell_text(cell).to_lowercase().contains(word))
    })
}

/// The rows that pass the filter, in the plugin's order.
#[must_use]
pub fn filter_rows<'a>(rows: &'a [Row], filter: &str) -> Vec<&'a Row> {
    rows.iter().filter(|r| row_matches(r, filter)).collect()
}

/// Longest cell text of a column, in characters (the label counts too).
fn widest(column: &Column, rows: &[Row]) -> usize {
    rows.iter()
        .filter_map(|r| r.cells.get(&column.key))
        .map(|c| cell_text(c).chars().count())
        .chain(std::iter::once(column.label.chars().count()))
        .max()
        .unwrap_or(0)
}

/// The `grid-template-columns` of a table: short columns (a status, a
/// commit, a duration) take a fixed width that grows with the text size, long
/// ones share the rest. The last track is the row's buttons.
#[must_use]
pub fn grid_template(columns: &[Column], rows: &[Row], with_actions: bool) -> String {
    const SHORT: usize = 14;
    let scaled = |px: usize| format!("calc({px}px * var(--text-scale, 1))");
    let mut tracks: Vec<String> = columns
        .iter()
        .map(|column| {
            let chars = widest(column, rows);
            let badge = rows
                .iter()
                .find_map(|r| r.cells.get(&column.key))
                .is_some_and(|c| matches!(c, Cell::Badge(_)));
            let time = rows
                .iter()
                .find_map(|r| r.cells.get(&column.key))
                .is_some_and(|c| matches!(c, Cell::Time(_)));
            if time {
                scaled(112)
            } else if badge {
                scaled(chars * 7 + 40)
            } else if chars <= SHORT || column.align_right {
                scaled(chars.min(SHORT) * 7 + 24)
            } else if column.mono {
                format!("minmax({}, {}fr)", scaled(96), chars.min(40))
            } else {
                format!("minmax({}, {}fr)", scaled(120), chars.min(40))
            }
        })
        .collect();
    if with_actions {
        // One width for every row, so the columns line up whether a row has
        // buttons or not: the widest row's labels plus their padding.
        let buttons = rows
            .iter()
            .map(|r| {
                r.actions
                    .iter()
                    .map(|a| a.label.chars().count() * 6 + 24 + 4)
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0);
        tracks.push(scaled(buttons));
    }
    tracks.join(" ")
}

/// The tone class a protocol tone draws with (the app's `tone-*` classes).
#[must_use]
pub fn tone_class(tone: &Tone) -> &'static str {
    match tone {
        Tone::Neutral => "tone-stopped",
        Tone::Success => "tone-success",
        Tone::Warning => "tone-warning",
        Tone::Danger => "tone-danger",
        Tone::Info => "tone-info",
    }
}

/// The banner tone of a message view.
#[must_use]
pub fn banner_tone(tone: &Tone) -> BannerTone {
    match tone {
        Tone::Neutral | Tone::Info => BannerTone::Info,
        Tone::Success => BannerTone::Success,
        Tone::Warning => BannerTone::Warning,
        Tone::Danger => BannerTone::Danger,
    }
}

/// The version banner of a plugin section (AD51): the hidden `status` view,
/// when it is a warning or worse. An informational answer shows nothing.
#[must_use]
pub fn status_banner(view: &View) -> Option<(BannerTone, String)> {
    match view {
        View::Message { tone, text, .. } if matches!(tone, Tone::Warning | Tone::Danger) => {
            Some((banner_tone(tone), text.clone()))
        }
        _ => None,
    }
}

/// The header of a selected row, drawn above the view the row opens: its
/// first cell is the title, its first badge the status, and the other cells
/// (except right-aligned numbers) make one muted line.
#[derive(Debug, Clone, PartialEq)]
pub struct RowSummary {
    pub title: String,
    pub badge: Option<Badge>,
    pub line: String,
}

#[must_use]
pub fn row_summary(columns: &[Column], row: &Row) -> RowSummary {
    let title = columns
        .first()
        .and_then(|c| row.cells.get(&c.key))
        .map_or_else(|| row.id.clone(), |c| cell_text(c).to_string());
    let mut badge = None;
    let mut parts: Vec<String> = Vec::new();
    for column in columns.iter().skip(1) {
        match row.cells.get(&column.key) {
            Some(Cell::Badge(b)) if badge.is_none() => badge = Some(b.clone()),
            Some(Cell::Badge(_)) | None => {}
            Some(_) if column.align_right => {}
            Some(cell) => {
                let text = cell_text(cell);
                if !text.is_empty() {
                    parts.push(text.to_string());
                }
            }
        }
    }
    // Cells that are not columns are the plugin's extra facts about the row
    // (a deployment's server): they close the line.
    for (key, cell) in &row.cells {
        if columns.iter().all(|c| c.key != *key) && !matches!(cell, Cell::Badge(_)) {
            let text = cell_text(cell);
            if !text.is_empty() {
                parts.push(text.to_string());
            }
        }
    }
    RowSummary {
        title,
        badge,
        line: parts.join(" \u{b7} "),
    }
}

// ---------------------------------------------------------------------
// Confirms
// ---------------------------------------------------------------------

/// A plugin's `Confirm` laid out as the dialog draws it. A line of the form
/// `Label: value` (a label of at most three words) becomes a row (Application,
/// Server, Branch); a line that
/// starts `Warning: ` becomes the amber box; any other line is a note under
/// the options.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConfirmView {
    pub title: String,
    pub rows: Vec<(String, String)>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
    pub button: String,
}

#[must_use]
pub fn confirm_view(confirm: &Confirm) -> ConfirmView {
    let mut out = ConfirmView {
        title: confirm.title.clone(),
        button: confirm.button.clone(),
        ..ConfirmView::default()
    };
    for line in &confirm.lines {
        match line.split_once(": ") {
            Some(("Warning", text)) => out.warnings.push(text.to_string()),
            Some((label, value))
                if !value.is_empty()
                    && label.chars().count() <= 24
                    && label.split_whitespace().count() <= 3
                    && label.chars().next().is_some_and(char::is_uppercase)
                    && !label.contains(['.', '?', '!']) =>
            {
                out.rows.push((label.to_string(), value.to_string()));
            }
            _ => out.notes.push(line.clone()),
        }
    }
    out
}

// ---------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------

/// One drawn row of a plugin log.
#[derive(Debug, Clone, PartialEq)]
pub struct LogRow {
    pub row: RowModel,
    pub tone: RowTone,
}

/// The rows of a stream: each line classified by the same rules as every
/// other log in the app, and, once the stream ended badly, any line that says
/// "failed" marked too (`Deployment failed at step 3 of 6`).
#[must_use]
pub fn log_rows(log: &LogState) -> Vec<LogRow> {
    if log.lines.is_empty() {
        return Vec::new();
    }
    let mut view = LogView::new(MAX_LOG_LINES, BTreeMap::new());
    let source = ContainerId("plugin".to_string());
    let lines: Vec<LogLine> = log
        .lines
        .iter()
        .map(|(ts, text)| LogLine {
            container: source.clone(),
            ts: *ts,
            stream: Stream::Stdout,
            text: text.clone(),
        })
        .collect();
    view.push_batch(lines);
    let failed = log.failed();
    view.rows(0..view.len())
        .into_iter()
        .map(|row| {
            let mut tone = row_tone(&row);
            if failed && tone == RowTone::Plain && row.text.to_lowercase().contains("failed") {
                tone = RowTone::Error;
            }
            LogRow { row, tone }
        })
        .collect()
}

/// The row a failed stream should open on: the first line that says what
/// went wrong. `None` while the stream runs or when it did not fail.
#[must_use]
pub fn failing_seq(rows: &[LogRow], log: &LogState) -> Option<u64> {
    if !log.failed() {
        return None;
    }
    rows.iter()
        .find(|r| r.tone == RowTone::Error)
        .or_else(|| rows.last())
        .map(|r| r.row.seq)
}

/// A line stamped at a fixed moment: scenes and tests build logs with it.
#[must_use]
pub fn stamped(at: DateTime<Utc>, texts: &[&str]) -> Vec<(DateTime<Utc>, String)> {
    texts.iter().map(|t| (at, (*t).to_string())).collect()
}

/// The icon a sidebar item of a plugin gets, by what it opens.
#[must_use]
pub fn icon_for_view(view: &str) -> &'static str {
    match view {
        "resources" => "layers",
        "servers" => "server",
        "deployments" => "activity",
        _ => "zap",
    }
}
