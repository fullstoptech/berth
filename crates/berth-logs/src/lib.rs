//! Log pipeline (PLAN §6.6, D25, ADR 0011): classify and fold lines on push, hold a ring buffer,
//! search and filter in Rust, and hand the UI one batch per 16 ms tick.

mod batch;
mod buffer;
mod classify;
mod fold;
mod json;
mod search;
mod view;

pub use batch::{batch, batch_counted, BatchDrops, DEFAULT_QUEUE_LINES, MAX_BATCH_LINES};
pub use buffer::{LevelCounts, LogBuffer, PreparedBatch};
pub use classify::classify;
pub use fold::is_stack_continuation;
pub use json::summarize_json;
pub use search::{Query, SearchSummary};
pub use view::{LogFilter, LogView, RowModel, ViewDelta};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

/// A JSON log line reduced to what a collapsed row shows: `level · message · k=v …`.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonSummary {
    pub level: Option<Level>,
    pub message: Option<String>,
    /// The first six other fields, values rendered as text.
    pub fields: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    None,
    /// First line of a stack trace; `hidden` lines follow it.
    Head {
        hidden: u32,
    },
    /// A hidden trace line; `head` is the head's `seq`.
    Member {
        head: u64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub seq: u64,
    pub line: berth_core::LogLine,
    pub level: Option<Level>,
    pub error_mark: bool,
    pub json: Option<JsonSummary>,
    pub fold: Fold,
    pub series: u8,
}

#[derive(Debug, thiserror::Error)]
pub enum LogsError {
    #[error("invalid pattern: {0}")]
    Regex(String),
}

/// Series hue for the `index`-th container of a view: 1..=5, cycling (design system has five).
#[must_use]
pub fn series_for(index: usize) -> u8 {
    // `index % 5` is below 5, so the cast cannot truncate.
    #[allow(clippy::cast_possible_truncation)]
    let n = (index % 5) as u8;
    n + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_cycles_through_five_hues() {
        let got: Vec<u8> = (0..7).map(series_for).collect();
        assert_eq!(got, vec![1, 2, 3, 4, 5, 1, 2]);
    }
}
