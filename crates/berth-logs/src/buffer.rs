//! The ring buffer: lines are classified, stripped of ANSI escapes and folded once, on push.

use std::collections::VecDeque;

use berth_core::LogLine;

use crate::classify::classify_with;
use crate::fold::FoldTracker;
use crate::{summarize_json, Fold, Level, LogEntry};

/// Lines per level currently in the buffer (`Trace` counts as `Debug`).
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
            entries: VecDeque::with_capacity(cap.min(4096)),
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
    pub fn push(&mut self, mut line: LogLine, series: u8) -> u64 {
        if line.text.contains('\x1b') {
            line.text = strip_ansi_escapes::strip_str(&line.text);
        }
        let kept = line.text.trim_end_matches(['\r', '\n']).len();
        line.text.truncate(kept);

        let seq = self.next_seq;
        self.next_seq += 1;
        let json = summarize_json(&line.text);
        let (level, error_mark) = classify_with(&line.text, line.stream, json.as_ref());
        let fold = match self.folds.place(&line.container, seq, &line.text) {
            Some(head) => {
                self.add_hidden_to(head);
                Fold::Member { head }
            }
            None => Fold::None,
        };
        if let Some(level) = level {
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
                if let Some(level) = old.level {
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
