//! The ring buffer: lines are classified, stripped of ANSI escapes and folded once, on push.

use std::collections::VecDeque;

use berth_core::LogLine;

use crate::classify::classify_with;
use crate::fold::{FoldFacts, FoldTracker};
use crate::{summarize_json, Fold, JsonSummary, Level, LogEntry};

/// Lines per level currently in the buffer (`Trace` counts as `Debug`; a line with the error mark
/// and no level counts as `Error`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LevelCounts {
    pub error: u32,
    pub warn: u32,
    pub info: u32,
    pub debug: u32,
}

impl LevelCounts {
    fn slot(&mut self, level: Level) -> &mut u32 {
        match level {
            Level::Error => &mut self.error,
            Level::Warn => &mut self.warn,
            Level::Info => &mut self.info,
            Level::Debug | Level::Trace => &mut self.debug,
        }
    }
}

/// The level a line counts under: its own, or `Error` when it has none and carries the error mark
/// ("Error: connect ECONNREFUSED" has no level word but is an error line, as the Error chip and
/// F8 treat it). A trace member never adds to the count: its head carries the mark.
fn counted_level(level: Option<Level>, error_mark: bool, fold: Fold) -> Option<Level> {
    level.or_else(|| (error_mark && !matches!(fold, Fold::Member { .. })).then_some(Level::Error))
}

/// A line with everything that does not depend on its neighbours worked out: ANSI escapes and the
/// line ending stripped, JSON summary, level and error mark, and the text facts the fold decision
/// uses.
#[derive(Debug)]
pub(crate) struct PreparedLine {
    pub(crate) line: LogLine,
    json: Option<JsonSummary>,
    level: Option<Level>,
    error_mark: bool,
    fold_facts: FoldFacts,
}

impl PreparedLine {
    fn new(mut line: LogLine) -> Self {
        if line.text.contains('\x1b') {
            line.text = strip_ansi_escapes::strip_str(&line.text);
        }
        let kept = line.text.trim_end_matches(['\r', '\n']).len();
        line.text.truncate(kept);
        let json = summarize_json(&line.text);
        let (level, error_mark) = classify_with(&line.text, line.stream, json.as_ref());
        let fold_facts = FoldFacts::of(&line.text);
        Self {
            line,
            json,
            level,
            error_mark,
            fold_facts,
        }
    }
}

/// A batch of lines run through the part of [`LogView::push_batch`](crate::LogView::push_batch)
/// that needs no view state (D23): ANSI stripping, classification, JSON parsing and the text facts
/// for folding. Build it with [`PreparedBatch::new`] outside any lock, then hand it to
/// [`LogView::commit_batch`](crate::LogView::commit_batch), which does the short remainder: the
/// fold link to the previous line, sequence numbers, counts, the visible list, the search and
/// eviction. Batches must be committed in the order their lines should appear.
#[derive(Debug)]
pub struct PreparedBatch {
    pub(crate) lines: Vec<PreparedLine>,
}

impl PreparedBatch {
    /// Prepares `lines`. Touches no view, so it can run while the UI reads one.
    #[must_use]
    pub fn new(lines: Vec<LogLine>) -> Self {
        Self {
            lines: lines.into_iter().map(PreparedLine::new).collect(),
        }
    }

    /// Lines in the batch.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

#[derive(Debug)]
pub struct LogBuffer {
    entries: VecDeque<LogEntry>,
    cap: usize,
    next_seq: u64,
    dropped: u64,
    counts: LevelCounts,
    folds: FoldTracker,
}

impl LogBuffer {
    /// A buffer that keeps the newest `cap` lines (at least one).
    #[must_use]
    pub fn new(cap: usize) -> Self {
        let cap = cap.max(1);
        Self {
            // Reserved up front: growing a 100,000-entry ring by doubling copies megabytes in the
            // middle of a batch. Untouched pages cost nothing until lines arrive.
            entries: VecDeque::with_capacity(cap),
            cap,
            next_seq: 0,
            dropped: 0,
            counts: LevelCounts::default(),
            folds: FoldTracker::default(),
        }
    }

    /// Adds a line and returns its sequence number. Sequence numbers only grow, also across
    /// [`clear`](Self::clear), so a row key never repeats. The oldest line is dropped when the
    /// buffer is full.
    pub fn push(&mut self, line: LogLine, series: u8) -> u64 {
        self.push_prepared(PreparedLine::new(line), series)
    }

    /// [`push`](Self::push) for a line that is already prepared: only the parts that depend on
    /// the lines before it run here (sequence number, fold link, counts, eviction).
    pub(crate) fn push_prepared(&mut self, prepared: PreparedLine, series: u8) -> u64 {
        let PreparedLine {
            line,
            json,
            level,
            error_mark,
            fold_facts,
        } = prepared;
        let seq = self.next_seq;
        self.next_seq += 1;
        let fold = match self
            .folds
            .place_with(&line.container, line.stream, seq, &fold_facts)
        {
            Some(head) => {
                self.add_hidden_to(head);
                Fold::Member { head }
            }
            None => Fold::None,
        };
        if let Some(level) = counted_level(level, error_mark, fold) {
            *self.counts.slot(level) += 1;
        }
        self.entries.push_back(LogEntry {
            seq,
            line,
            level,
            error_mark,
            json,
            fold,
            series,
        });
        while self.entries.len() > self.cap {
            if let Some(old) = self.entries.pop_front() {
                if let Some(level) = counted_level(old.level, old.error_mark, old.fold) {
                    let slot = self.counts.slot(level);
                    *slot = slot.saturating_sub(1);
                }
                self.dropped += 1;
            }
        }
        seq
    }

    fn add_hidden_to(&mut self, head: u64) {
        if let Some(entry) = self.entry_mut(head) {
            entry.fold = match entry.fold {
                Fold::Head { hidden } => Fold::Head {
                    hidden: hidden.saturating_add(1),
                },
                _ => Fold::Head { hidden: 1 },
            };
        }
    }

    fn index_of(&self, seq: u64) -> Option<usize> {
        let first = self.entries.front()?.seq;
        let index = usize::try_from(seq.checked_sub(first)?).ok()?;
        (index < self.entries.len()).then_some(index)
    }

    fn entry_mut(&mut self, seq: u64) -> Option<&mut LogEntry> {
        let index = self.index_of(seq)?;
        self.entries.get_mut(index)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Lines pushed out of the ring since the buffer was created or last cleared.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Counts lines that never reached the buffer (see `BatchDrops`) as dropped.
    pub fn add_dropped(&mut self, lines: u64) {
        self.dropped = self.dropped.saturating_add(lines);
    }

    #[must_use]
    pub fn counts(&self) -> &LevelCounts {
        &self.counts
    }

    #[must_use]
    pub fn by_seq(&self, seq: u64) -> Option<&LogEntry> {
        self.entries.get(self.index_of(seq)?)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.counts = LevelCounts::default();
        self.dropped = 0;
        self.folds.clear();
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &LogEntry> {
        self.entries.iter()
    }

    pub(crate) fn last(&self) -> Option<&LogEntry> {
        self.entries.back()
    }

    pub(crate) fn first_seq(&self) -> Option<u64> {
        self.entries.front().map(|e| e.seq)
    }

    /// The hidden lines of a folded trace, oldest first. Members sit right after their head, so
    /// the scan stops as soon as all `hidden` lines are found.
    pub(crate) fn members_of(&self, head: u64) -> Vec<&LogEntry> {
        let Some(start) = self.index_of(head) else {
            return Vec::new();
        };
        let Some(Fold::Head { hidden }) = self.entries.get(start).map(|e| e.fold) else {
            return Vec::new();
        };
        self.entries
            .iter()
            .skip(start + 1)
            .filter(|e| e.fold == Fold::Member { head })
            .take(hidden as usize)
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use berth_core::{ContainerId, Stream};
    use chrono::DateTime;

    use super::*;

    pub(crate) fn line(container: &str, stream: Stream, text: &str) -> LogLine {
        LogLine {
            container: ContainerId(container.to_string()),
            ts: DateTime::from_timestamp(1_761_297_240, 0).expect("ts"),
            stream,
            text: text.to_string(),
        }
    }

    pub(crate) fn out(text: &str) -> LogLine {
        line("c1", Stream::Stdout, text)
    }

    #[test]
    fn ring_drops_the_oldest_and_counts_them() {
        let mut buf = LogBuffer::new(3);
        for i in 0..5 {
            buf.push(out(&format!("line {i}")), 1);
        }
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.dropped(), 2);
        assert!(buf.by_seq(1).is_none());
        assert_eq!(buf.by_seq(2).map(|e| e.line.text.as_str()), Some("line 2"));
        assert_eq!(buf.by_seq(4).map(|e| e.line.text.as_str()), Some("line 4"));
        assert!(buf.by_seq(5).is_none());
    }

    #[test]
    fn level_counts_follow_pushes_and_drops() {
        let mut buf = LogBuffer::new(3);
        buf.push(out("ERROR a"), 1);
        buf.push(out("WARN b"), 1);
        buf.push(out("INFO c"), 1);
        buf.push(out("TRACE d"), 1);
        assert_eq!(
            buf.counts(),
            &LevelCounts {
                error: 0,
                warn: 1,
                info: 1,
                debug: 1
            }
        );
    }

    #[test]
    fn error_marked_lines_without_a_level_count_as_errors_once_per_trace() {
        let mut buf = LogBuffer::new(10);
        buf.push(out("Error: connect ECONNREFUSED 172.19.0.3:5432"), 1);
        // A trace member that mentions an error word does not add to the count.
        buf.push(out("    at Object.connect (node:net:1139:12)"), 1);
        buf.push(out("Caused by: java.io.IOException: boom"), 1);
        buf.push(out("plain line"), 1);
        assert_eq!(buf.counts().error, 1, "{:?}", buf.counts());
        // Dropping the head takes it off the count again.
        let mut small = LogBuffer::new(1);
        small.push(out("Error: connect ECONNREFUSED"), 1);
        small.push(out("plain line"), 1);
        assert_eq!(small.counts().error, 0);
    }

    #[test]
    fn clear_empties_but_sequence_numbers_keep_growing() {
        let mut buf = LogBuffer::new(10);
        buf.push(out("ERROR a"), 1);
        buf.push(out("b"), 1);
        buf.clear();
        assert!(buf.is_empty());
        assert_eq!(buf.dropped(), 0);
        assert_eq!(buf.counts(), &LevelCounts::default());
        assert_eq!(buf.push(out("c"), 1), 2);
    }

    #[test]
    fn ansi_escapes_and_line_endings_are_stripped_once() {
        let mut buf = LogBuffer::new(10);
        let seq = buf.push(out("\x1b[31mERROR\x1b[0m boom\r\n"), 1);
        let entry = buf.by_seq(seq).expect("entry");
        assert_eq!(entry.line.text, "ERROR boom");
        assert_eq!(entry.level, Some(Level::Error));
    }

    #[test]
    fn json_lines_carry_a_summary_and_their_level() {
        let mut buf = LogBuffer::new(10);
        let seq = buf.push(out(r#"{"level":"warn","msg":"slow","ms":9}"#), 2);
        let entry = buf.by_seq(seq).expect("entry");
        assert_eq!(entry.level, Some(Level::Warn));
        assert_eq!(entry.series, 2);
        let json = entry.json.as_ref().expect("json");
        assert_eq!(json.message.as_deref(), Some("slow"));
    }

    #[test]
    fn stack_lines_fold_under_their_head() {
        let mut buf = LogBuffer::new(10);
        let head = buf.push(out("Error: connect ECONNREFUSED"), 1);
        let a = buf.push(out("    at f (a.js:1:1)"), 1);
        let b = buf.push(out("    at g (a.js:2:2)"), 1);
        let after = buf.push(out("retrying"), 1);
        assert_eq!(
            buf.by_seq(head).map(|e| e.fold),
            Some(Fold::Head { hidden: 2 })
        );
        assert_eq!(buf.by_seq(a).map(|e| e.fold), Some(Fold::Member { head }));
        assert_eq!(buf.by_seq(b).map(|e| e.fold), Some(Fold::Member { head }));
        assert_eq!(buf.by_seq(after).map(|e| e.fold), Some(Fold::None));
        let members: Vec<u64> = buf.members_of(head).iter().map(|e| e.seq).collect();
        assert_eq!(members, vec![a, b]);
    }

    #[test]
    fn a_member_whose_head_was_dropped_is_harmless() {
        let mut buf = LogBuffer::new(2);
        buf.push(out("Error: x"), 1);
        buf.push(out("    at f (a.js:1:1)"), 1);
        let late = buf.push(out("    at g (a.js:2:2)"), 1);
        assert_eq!(buf.len(), 2);
        assert_eq!(
            buf.by_seq(late).map(|e| e.fold),
            Some(Fold::Member { head: 0 })
        );
        assert!(buf.members_of(0).is_empty());
    }
}
