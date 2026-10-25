//! One log view: a ring buffer plus the filter, folds, search and follow state the UI needs, and
//! the row models for the visible window.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::ops::Range;

use berth_core::{ContainerId, LogLine, Stream};
use chrono::{DateTime, SecondsFormat, Utc};

use crate::buffer::{LevelCounts, LogBuffer};
use crate::search::{Matcher, Query, SearchState, SearchSummary};
use crate::{series_for, Fold, JsonSummary, Level, LogEntry, LogsError};

/// What the rows must satisfy. `Default` shows nothing (both streams off); a [`LogView`] starts
/// with both streams on and no other restriction.
#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    pub stdout: bool,
    pub stderr: bool,
    /// Empty means every level. `Debug` also admits `Trace`. A line with no level passes only when
    /// it carries an error mark and `Error` is in the set.
    pub levels: BTreeSet<Level>,
    pub containers: Option<BTreeSet<ContainerId>>,
    pub since: Option<DateTime<Utc>>,
}

impl LogFilter {
    fn everything() -> Self {
        Self {
            stdout: true,
            stderr: true,
            ..Self::default()
        }
    }

    fn passes(&self, entry: &LogEntry) -> bool {
        let stream_ok = match entry.line.stream {
            Stream::Stdout => self.stdout,
            Stream::Stderr => self.stderr,
        };
        stream_ok
            && self
                .containers
                .as_ref()
                .is_none_or(|set| set.contains(&entry.line.container))
            && self.since.is_none_or(|since| entry.line.ts >= since)
            && self.level_ok(entry)
    }

    fn level_ok(&self, entry: &LogEntry) -> bool {
        if self.levels.is_empty() {
            return true;
        }
        match entry.level {
            Some(Level::Trace) => {
                self.levels.contains(&Level::Debug) || self.levels.contains(&Level::Trace)
            }
            Some(level) => self.levels.contains(&level),
            None => entry.error_mark && self.levels.contains(&Level::Error),
        }
    }
}

/// What one `push_batch` changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewDelta {
    /// Rows added to the visible list (lines hidden by the filter or a fold do not count).
    pub appended: u32,
    /// Lines pushed out of the ring by this batch.
    pub dropped: u32,
    /// Equal to `appended` while follow is off, else 0: the "142 new lines" pill adds these up.
    pub new_while_paused: u32,
}

#[derive(Debug, Clone)]
pub struct RowModel {
    pub seq: u64,
    /// `HH:MM:SS.mmm`, UTC.
    pub ts: String,
    /// The name given to [`LogView::new`], else the first 12 characters of the container id.
    pub container: String,
    pub series: u8,
    pub stream: Stream,
    pub level: Option<Level>,
    pub error_mark: bool,
    pub text: String,
    pub json: Option<JsonSummary>,
    pub fold: Fold,
    /// Byte ranges of search matches inside `text`.
    pub matches: Vec<Range<usize>>,
}

#[derive(Debug)]
pub struct LogView {
    buffer: LogBuffer,
    names: BTreeMap<ContainerId, String>,
    series: HashMap<ContainerId, u8>,
    filter: LogFilter,
    /// Sequence numbers of the rows that pass the filter and are not folded away, ascending.
    visible: VecDeque<u64>,
    /// Heads whose trace is open.
    expanded: HashSet<u64>,
    search: Option<SearchState>,
    follow: bool,
}

impl LogView {
    /// A view over a ring of `cap` lines. `names` supplies the container column and the series
    /// hue: the n-th name (in id order) gets `series_for(n)`; containers met later continue the
    /// count.
    #[must_use]
    pub fn new(cap: usize, names: BTreeMap<ContainerId, String>) -> Self {
        let series = names
            .keys()
            .enumerate()
            .map(|(i, id)| (id.clone(), series_for(i)))
            .collect();
        Self {
            buffer: LogBuffer::new(cap),
            names,
            series,
            filter: LogFilter::everything(),
            visible: VecDeque::with_capacity(cap),
            expanded: HashSet::new(),
            search: None,
            follow: true,
        }
    }

    pub fn push_batch(&mut self, lines: Vec<LogLine>) -> ViewDelta {
        let dropped_before = self.buffer.dropped();
        let mut appended = 0_u32;
        for line in lines {
            let series = self.series_for_container(&line.container);
            let seq = self.buffer.push(line, series);
            if self.admit(seq) {
                appended += 1;
            }
        }
        self.forget_dropped();
        ViewDelta {
            appended,
            dropped: u32::try_from(self.buffer.dropped() - dropped_before).unwrap_or(u32::MAX),
            new_while_paused: if self.follow { 0 } else { appended },
        }
    }

    pub fn set_filter(&mut self, filter: LogFilter) {
        self.filter = filter;
        self.rebuild();
    }

    pub fn set_follow(&mut self, on: bool) {
        self.follow = on;
    }

    /// Starts, replaces or (with `None` or an empty query) ends a search. A bad regex returns
    /// [`LogsError::Regex`] and leaves no search active.
    ///
    /// # Errors
    /// The regex engine's message when `q.regex` is set and the pattern does not compile.
    pub fn set_search(&mut self, q: Option<Query>) -> Result<SearchSummary, LogsError> {
        self.search = None;
        let Some(query) = q else {
            return Ok(SearchSummary::default());
        };
        let Some(matcher) = Matcher::new(&query)? else {
            return Ok(SearchSummary::default());
        };
        let mut state = SearchState::new(matcher);
        state.rebuild(Self::visible_texts(&self.buffer, &self.visible));
        let summary = state.summary();
        self.search = Some(state);
        Ok(summary)
    }

    /// The row index of the next match after the current one (wrapping), and makes it current.
    pub fn next_match(&mut self) -> Option<usize> {
        self.step_match(true)
    }

    /// The row index of the previous match (wrapping), and makes it current.
    pub fn prev_match(&mut self) -> Option<usize> {
        self.step_match(false)
    }

    fn step_match(&mut self, forward: bool) -> Option<usize> {
        let seq = self.search.as_mut()?.step(forward)?;
        self.visible.binary_search(&seq).ok()
    }

    /// "3 of 41" for the active search.
    #[must_use]
    pub fn search_summary(&self) -> SearchSummary {
        self.search
            .as_ref()
            .map(SearchState::summary)
            .unwrap_or_default()
    }

    /// F8: the next error row after `after_row`, wrapping to the top; `None` when no other row is
    /// an error. Rows inside an open trace are skipped; the trace's head carries the mark.
    #[must_use]
    pub fn next_error(&self, after_row: usize) -> Option<usize> {
        let len = self.visible.len();
        let start = after_row.saturating_add(1).min(len);
        (start..len)
            .chain(0..after_row.min(len))
            .find(|&row| self.is_error_row(row))
    }

    /// Shift+F8: the previous error row before `before_row`, wrapping to the bottom.
    #[must_use]
    pub fn prev_error(&self, before_row: usize) -> Option<usize> {
        let len = self.visible.len();
        let before = before_row.min(len);
        (0..before)
            .rev()
            .chain((before.saturating_add(1)..len).rev())
            .find(|&row| self.is_error_row(row))
    }

    fn is_error_row(&self, row: usize) -> bool {
        self.visible
            .get(row)
            .and_then(|&seq| self.buffer.by_seq(seq))
            .is_some_and(|e| {
                !matches!(e.fold, Fold::Member { head } if self.buffer.by_seq(head).is_some())
                    && (e.level == Some(Level::Error) || e.error_mark)
            })
    }

    /// Opens or closes the stack trace under the row with this `seq`.
    pub fn toggle_fold(&mut self, head_seq: u64) {
        let is_head = self
            .buffer
            .by_seq(head_seq)
            .is_some_and(|e| matches!(e.fold, Fold::Head { .. }));
        if !is_head {
            return;
        }
        if !self.expanded.remove(&head_seq) {
            self.expanded.insert(head_seq);
        }
        self.rebuild();
    }

    /// Empties the buffer ("Clear"). Filter, search text and follow stay.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.visible.clear();
        self.expanded.clear();
        if let Some(search) = &mut self.search {
            search.rebuild(std::iter::empty());
        }
    }

    /// Visible rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.visible.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// Per-level line counts of the whole buffer, ignoring the filter (the level chips).
    #[must_use]
    pub fn counts(&self) -> &LevelCounts {
        self.buffer.counts()
    }

    /// Lines the ring has dropped ("Older lines dropped; Save to keep all").
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.buffer.dropped()
    }

    /// Adds lines `batch_counted` dropped before they reached the view to the dropped count.
    pub fn add_dropped(&mut self, lines: u64) {
        self.buffer.add_dropped(lines);
    }

    #[must_use]
    pub fn follow(&self) -> bool {
        self.follow
    }

    /// The series hue of a container, 1..=5.
    #[must_use]
    pub fn series(&self, id: &ContainerId) -> u8 {
        self.series.get(id).copied().unwrap_or(1)
    }

    /// Row models for `range` (clamped to the visible rows), with search highlights.
    #[must_use]
    pub fn rows(&self, range: Range<usize>) -> Vec<RowModel> {
        let end = range.end.min(self.visible.len());
        let start = range.start.min(end);
        self.visible
            .range(start..end)
            .filter_map(|&seq| self.buffer.by_seq(seq))
            .map(|entry| self.row_model(entry))
            .collect()
    }

    fn row_model(&self, entry: &LogEntry) -> RowModel {
        let text = entry.line.text.clone();
        let matches = self
            .search
            .as_ref()
            .map(|s| s.matcher.ranges(&text))
            .unwrap_or_default();
        RowModel {
            seq: entry.seq,
            ts: entry.line.ts.format("%H:%M:%S%.3f").to_string(),
            container: self.container_name(&entry.line.container),
            series: entry.series,
            stream: entry.line.stream,
            level: entry.level,
            error_mark: entry.error_mark,
            text,
            json: entry.json.clone(),
            fold: entry.fold,
            matches,
        }
    }

    /// The lines of `range` (all visible rows for `None`), one per line, for Copy, Save and Ask
    /// about these lines. A closed stack trace in the range is written out in full: a copied error
    /// without its trace is no use. `timestamps` prefixes each line with an RFC 3339 time (UTC).
    #[must_use]
    pub fn text(&self, range: Option<Range<usize>>, timestamps: bool) -> String {
        let range = range.unwrap_or(0..self.visible.len());
        let end = range.end.min(self.visible.len());
        let start = range.start.min(end);
        let mut out = String::new();
        let mut write = |entry: &LogEntry| {
            if !out.is_empty() {
                out.push('\n');
            }
            if timestamps {
                out.push_str(&entry.line.ts.to_rfc3339_opts(SecondsFormat::Millis, true));
                out.push(' ');
            }
            out.push_str(&entry.line.text);
        };
        for &seq in self.visible.range(start..end) {
            let Some(entry) = self.buffer.by_seq(seq) else {
                continue;
            };
            write(entry);
            if matches!(entry.fold, Fold::Head { .. }) && !self.expanded.contains(&seq) {
                for member in self.buffer.members_of(seq) {
                    write(member);
                }
            }
        }
        out
    }

    fn container_name(&self, id: &ContainerId) -> String {
        self.names
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.0.chars().take(12).collect())
    }

    fn series_for_container(&mut self, id: &ContainerId) -> u8 {
        if let Some(series) = self.series.get(id) {
            return *series;
        }
        let series = series_for(self.series.len());
        self.series.insert(id.clone(), series);
        series
    }

    /// Does this entry show as a row right now?
    fn shows(&self, entry: &LogEntry) -> bool {
        if let Fold::Member { head } = entry.fold {
            if let Some(head_entry) = self.buffer.by_seq(head) {
                return self.expanded.contains(&head) && self.filter.passes(head_entry);
            }
        }
        self.filter.passes(entry)
    }

    /// Adds the newest buffer entry to the visible list when it shows.
    fn admit(&mut self, seq: u64) -> bool {
        let Some(entry) = self.buffer.last().filter(|e| e.seq == seq) else {
            return false;
        };
        if !self.shows(entry) {
            return false;
        }
        if let Some(search) = &mut self.search {
            search.offer(seq, &entry.line.text);
        }
        self.visible.push_back(seq);
        true
    }

    /// Drops what the ring no longer holds from the visible list, the search and the open folds.
    fn forget_dropped(&mut self) {
        let Some(first) = self.buffer.first_seq() else {
            self.visible.clear();
            self.expanded.clear();
            return;
        };
        while self.visible.front().is_some_and(|&seq| seq < first) {
            self.visible.pop_front();
        }
        if let Some(search) = &mut self.search {
            search.evict_before(first);
        }
        if !self.expanded.is_empty() {
            self.expanded.retain(|&head| head >= first);
        }
    }

    /// Recomputes the visible list and the search matches from the whole buffer.
    fn rebuild(&mut self) {
        let visible: VecDeque<u64> = self
            .buffer
            .iter()
            .filter(|e| self.shows(e))
            .map(|e| e.seq)
            .collect();
        self.visible = visible;
        if let Some(search) = &mut self.search {
            search.rebuild(Self::visible_texts(&self.buffer, &self.visible));
        }
    }

    fn visible_texts<'a>(
        buffer: &'a LogBuffer,
        visible: &'a VecDeque<u64>,
    ) -> impl Iterator<Item = (u64, &'a str)> {
        visible
            .iter()
            .filter_map(|&seq| buffer.by_seq(seq).map(|e| (seq, e.line.text.as_str())))
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::buffer::tests::{line, out};

    fn view_with(texts: &[&str]) -> LogView {
        let mut view = LogView::new(1000, BTreeMap::new());
        view.push_batch(texts.iter().map(|t| out(t)).collect());
        view
    }

    fn texts(view: &LogView) -> Vec<String> {
        view.rows(0..view.len())
            .into_iter()
            .map(|r| r.text)
            .collect()
    }

    fn levels(of: &[Level]) -> BTreeSet<Level> {
        of.iter().copied().collect()
    }

    fn only(of: &[Level]) -> LogFilter {
        LogFilter {
            stdout: true,
            stderr: true,
            levels: levels(of),
            ..LogFilter::default()
        }
    }

    #[test]
    fn filter_by_stream() {
        let mut view = LogView::new(100, BTreeMap::new());
        view.push_batch(vec![
            line("c1", Stream::Stdout, "to stdout"),
            line("c1", Stream::Stderr, "to stderr"),
        ]);
        view.set_filter(LogFilter {
            stderr: true,
            ..LogFilter::default()
        });
        assert_eq!(texts(&view), vec!["to stderr"]);
        view.set_filter(LogFilter {
            stdout: true,
            ..LogFilter::default()
        });
        assert_eq!(texts(&view), vec!["to stdout"]);
        view.set_filter(LogFilter::default());
        assert!(view.is_empty());
    }

    #[test]
    fn filter_by_level_admits_trace_under_debug_and_marked_lines_under_error() {
        let mut view = view_with(&[
            "ERROR a",
            "WARN b",
            "INFO c",
            "DEBUG d",
            "TRACE e",
            "no level",
            "panic: no level but marked",
        ]);
        view.set_filter(only(&[Level::Error]));
        assert_eq!(texts(&view), vec!["ERROR a", "panic: no level but marked"]);
        view.set_filter(only(&[Level::Warn, Level::Info]));
        assert_eq!(texts(&view), vec!["WARN b", "INFO c"]);
        view.set_filter(only(&[Level::Debug]));
        assert_eq!(texts(&view), vec!["DEBUG d", "TRACE e"]);
        view.set_filter(only(&[]));
        assert_eq!(view.len(), 7);
    }

    #[test]
    fn filter_by_container_and_since() {
        let mut view = LogView::new(100, BTreeMap::new());
        let at = |s: i64| {
            Utc.timestamp_opt(1_761_297_000 + s, 0)
                .single()
                .expect("ts")
        };
        for (id, s, text) in [("a", 0, "a0"), ("b", 10, "b10"), ("a", 20, "a20")] {
            let mut l = line(id, Stream::Stdout, text);
            l.ts = at(s);
            view.push_batch(vec![l]);
        }
        view.set_filter(LogFilter {
            containers: Some(BTreeSet::from([ContainerId("a".to_string())])),
            ..only(&[])
        });
        assert_eq!(texts(&view), vec!["a0", "a20"]);
        view.set_filter(LogFilter {
            since: Some(at(10)),
            ..only(&[])
        });
        assert_eq!(texts(&view), vec!["b10", "a20"]);
    }

    #[test]
    fn a_trace_is_one_row_until_opened() {
        let mut view = view_with(&[
            "start",
            "Error: boom",
            "    at f (a.js:1:1)",
            "    at g (a.js:2:2)",
            "done",
        ]);
        assert_eq!(texts(&view), vec!["start", "Error: boom", "done"]);
        let head = view.rows(1..2).remove(0);
        assert_eq!(head.fold, Fold::Head { hidden: 2 });
        view.toggle_fold(head.seq);
        assert_eq!(view.len(), 5);
        assert_eq!(view.rows(2..3)[0].fold, Fold::Member { head: head.seq });
        view.toggle_fold(head.seq);
        assert_eq!(view.len(), 3);
    }

    #[test]
    fn a_member_arriving_while_open_shows_at_once() {
        let mut view = view_with(&["Error: boom", "    at f (a.js:1:1)"]);
        view.toggle_fold(0);
        let delta = view.push_batch(vec![out("    at g (a.js:2:2)")]);
        assert_eq!(delta.appended, 1);
        assert_eq!(view.len(), 3);
    }

    #[test]
    fn toggling_a_plain_line_does_nothing() {
        let mut view = view_with(&["plain", "other"]);
        view.toggle_fold(0);
        assert_eq!(view.len(), 2);
    }

    #[test]
    fn an_open_trace_follows_the_filter_of_its_head() {
        let mut view = view_with(&["INFO fine", "ERROR boom", "    at f (a.js:1:1)"]);
        view.toggle_fold(1);
        view.set_filter(only(&[Level::Info]));
        assert_eq!(texts(&view), vec!["INFO fine"]);
        view.set_filter(only(&[Level::Error]));
        assert_eq!(texts(&view), vec!["ERROR boom", "    at f (a.js:1:1)"]);
    }

    #[test]
    fn rows_carry_names_series_time_and_clamp_the_range() {
        let a = ContainerId("aaaaaaaaaaaaaaaaaaaa".to_string());
        let b = ContainerId("bbbb".to_string());
        let mut view = LogView::new(
            10,
            BTreeMap::from([
                (a.clone(), "web".to_string()),
                (b.clone(), "db".to_string()),
            ]),
        );
        view.push_batch(vec![
            line(&a.0, Stream::Stdout, "from web"),
            line(&b.0, Stream::Stderr, "from db"),
            line("cccccccccccccccccccc", Stream::Stdout, "from stranger"),
        ]);
        let rows = view.rows(0..99);
        assert_eq!(rows.len(), 3);
        assert_eq!((rows[0].container.as_str(), rows[0].series), ("web", 1));
        assert_eq!((rows[1].container.as_str(), rows[1].series), ("db", 2));
        assert_eq!(rows[1].stream, Stream::Stderr);
        assert_eq!(
            (rows[2].container.as_str(), rows[2].series),
            ("cccccccccccc", 3)
        );
        assert_eq!(rows[0].ts, "09:14:00.000");
        assert!(view.rows(5..9).is_empty());
        assert_eq!(view.rows(2..3)[0].text, "from stranger");
    }

    fn plain_query(text: &str) -> Query {
        Query {
            text: text.to_string(),
            regex: false,
            case_sensitive: false,
        }
    }

    #[test]
    fn search_highlights_counts_and_steps_with_wrap() {
        let mut view = view_with(&["alpha", "beta ECONNREFUSED", "gamma", "econnrefused again"]);
        let summary = view
            .set_search(Some(plain_query("econnrefused")))
            .expect("valid");
        assert_eq!(
            summary,
            SearchSummary {
                matches: 2,
                current: None
            }
        );
        assert_eq!(view.rows(1..2)[0].matches, vec![5..17]);
        assert!(view.rows(0..1)[0].matches.is_empty());
        assert_eq!(view.next_match(), Some(1));
        assert_eq!(
            view.search_summary(),
            SearchSummary {
                matches: 2,
                current: Some(1)
            }
        );
        assert_eq!(view.next_match(), Some(3));
        assert_eq!(view.next_match(), Some(1));
        assert_eq!(view.prev_match(), Some(3));
    }

    #[test]
    fn regex_search_and_invalid_regex() {
        let mut view = view_with(&["took 12ms", "took 7ms", "idle"]);
        let ok = view
            .set_search(Some(Query {
                text: r"took \d{2}ms".to_string(),
                regex: true,
                case_sensitive: true,
            }))
            .expect("valid");
        assert_eq!(ok.matches, 1);
        let err = view
            .set_search(Some(Query {
                text: "took (".to_string(),
                regex: true,
                case_sensitive: false,
            }))
            .expect_err("invalid");
        assert!(matches!(err, LogsError::Regex(_)));
        assert_eq!(view.search_summary(), SearchSummary::default());
        assert_eq!(view.next_match(), None);
        assert!(view.rows(0..3).iter().all(|r| r.matches.is_empty()));
    }

    #[test]
    fn search_follows_new_lines_filters_and_drops() {
        let mut view = LogView::new(3, BTreeMap::new());
        view.push_batch(vec![out("hit 1"), out("miss"), out("hit 2")]);
        assert_eq!(
            view.set_search(Some(plain_query("hit")))
                .expect("valid")
                .matches,
            2
        );
        view.push_batch(vec![out("hit 3")]);
        assert_eq!(view.search_summary().matches, 2, "hit 1 left the ring");
        view.set_filter(only(&[Level::Error]));
        assert_eq!(
            view.search_summary().matches,
            0,
            "search covers visible rows only"
        );
        view.set_filter(only(&[]));
        assert_eq!(view.search_summary().matches, 2);
    }

    #[test]
    fn next_and_previous_error_wrap_and_skip_open_traces() {
        let mut view = view_with(&[
            "INFO a",
            "ERROR b",
            "INFO c",
            "Error: d",
            "    at Caused by: error inside (x.js:1:1)",
            "INFO e",
        ]);
        view.toggle_fold(3);
        assert_eq!(view.len(), 6);
        assert_eq!(view.next_error(0), Some(1));
        assert_eq!(view.next_error(1), Some(3));
        assert_eq!(
            view.next_error(3),
            Some(1),
            "wraps, and skips the open trace member"
        );
        assert_eq!(view.prev_error(3), Some(1));
        assert_eq!(view.prev_error(1), Some(3), "wraps to the bottom");
        let calm = view_with(&["INFO a", "INFO b"]);
        assert_eq!(calm.next_error(0), None);
        assert_eq!(calm.prev_error(1), None);
        let single = view_with(&["INFO a", "ERROR b"]);
        assert_eq!(
            single.next_error(1),
            None,
            "the only error is the current row"
        );
    }

    #[test]
    fn delta_counts_visible_rows_drops_and_paused_lines() {
        let mut view = LogView::new(4, BTreeMap::new());
        let d = view.push_batch(vec![out("a"), out("b"), out("c")]);
        assert_eq!(
            d,
            ViewDelta {
                appended: 3,
                dropped: 0,
                new_while_paused: 0
            }
        );
        view.set_follow(false);
        assert!(!view.follow());
        let d = view.push_batch(vec![out("d"), out("e"), out("f")]);
        assert_eq!(
            d,
            ViewDelta {
                appended: 3,
                dropped: 2,
                new_while_paused: 3
            }
        );
        assert_eq!(view.dropped(), 2);
        assert_eq!(texts(&view), vec!["c", "d", "e", "f"]);
        view.set_filter(only(&[Level::Error]));
        let d = view.push_batch(vec![out("g")]);
        assert_eq!(
            (d.appended, d.dropped),
            (0, 1),
            "hidden lines are not appended rows"
        );
    }

    #[test]
    fn lines_dropped_upstream_join_the_dropped_count() {
        let mut view = LogView::new(2, BTreeMap::new());
        view.push_batch(vec![out("a"), out("b"), out("c")]);
        view.add_dropped(40);
        assert_eq!(view.dropped(), 41);
        view.clear();
        assert_eq!(view.dropped(), 0);
    }

    #[test]
    fn counts_ignore_the_filter() {
        let mut view = view_with(&["ERROR a", "WARN b", "WARN c"]);
        view.set_filter(LogFilter::default());
        assert_eq!((view.counts().error, view.counts().warn), (1, 2));
    }

    #[test]
    fn text_copies_ranges_with_or_without_timestamps() {
        let view = view_with(&["one", "two", "three"]);
        assert_eq!(view.text(None, false), "one\ntwo\nthree");
        assert_eq!(view.text(Some(1..3), false), "two\nthree");
        assert_eq!(view.text(Some(2..99), false), "three");
        assert_eq!(view.text(Some(5..9), false), "");
        assert_eq!(view.text(Some(0..1), true), "2025-10-24T09:14:00.000Z one");
    }

    #[test]
    fn copying_a_closed_trace_includes_its_hidden_lines_once() {
        let mut view = view_with(&["Error: boom", "    at f (a.js:1:1)", "next"]);
        assert_eq!(
            view.text(None, false),
            "Error: boom\n    at f (a.js:1:1)\nnext"
        );
        view.toggle_fold(0);
        assert_eq!(
            view.text(None, false),
            "Error: boom\n    at f (a.js:1:1)\nnext"
        );
        assert_eq!(view.text(Some(0..1), false), "Error: boom");
    }

    #[test]
    fn clear_empties_rows_and_keeps_the_search() {
        let mut view = view_with(&["hit", "hit"]);
        view.set_search(Some(plain_query("hit"))).expect("valid");
        view.clear();
        assert!(view.is_empty());
        assert_eq!(view.search_summary().matches, 0);
        view.push_batch(vec![out("hit again")]);
        assert_eq!(view.search_summary().matches, 1);
        assert_eq!(view.dropped(), 0);
    }
}
