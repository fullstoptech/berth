//! Plain and regex search over the visible rows. Matches are kept as sequence numbers so they
//! survive appends and drops.

use std::collections::VecDeque;
use std::ops::Range;

use regex::{Regex, RegexBuilder};

use crate::LogsError;

#[derive(Debug, Clone)]
pub struct Query {
    pub text: String,
    pub regex: bool,
    pub case_sensitive: bool,
}

/// "3 of 41": `current` counts from 1.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchSummary {
    pub matches: u32,
    pub current: Option<u32>,
}

/// Compiled query. Plain text is escaped, so `a.b` finds only `a.b`.
#[derive(Debug)]
pub(crate) struct Matcher {
    re: Regex,
}

impl Matcher {
    /// `Ok(None)` for an empty query; `Err` carries the regex engine's message.
    pub(crate) fn new(query: &Query) -> Result<Option<Self>, LogsError> {
        if query.text.is_empty() {
            return Ok(None);
        }
        let pattern = if query.regex {
            query.text.clone()
        } else {
            regex::escape(&query.text)
        };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!query.case_sensitive)
            .size_limit(1 << 20)
            .build()
            .map_err(|e| LogsError::Regex(e.to_string()))?;
        Ok(Some(Self { re }))
    }

    pub(crate) fn is_match(&self, text: &str) -> bool {
        self.re.is_match(text)
    }

    /// Byte ranges of the matches in `text`, empty matches left out.
    pub(crate) fn ranges(&self, text: &str) -> Vec<Range<usize>> {
        self.re
            .find_iter(text)
            .filter(|m| !m.is_empty())
            .map(|m| m.range())
            .collect()
    }
}

#[derive(Debug)]
pub(crate) struct SearchState {
    pub(crate) matcher: Matcher,
    /// Sequence numbers of matching visible rows, ascending.
    matches: VecDeque<u64>,
    current: Option<u64>,
}

impl SearchState {
    pub(crate) fn new(matcher: Matcher) -> Self {
        Self {
            matcher,
            matches: VecDeque::new(),
            current: None,
        }
    }

    /// Called for each newly visible row, in sequence order.
    pub(crate) fn offer(&mut self, seq: u64, text: &str) {
        if self.matcher.is_match(text) {
            self.matches.push_back(seq);
        }
    }

    /// Starts over from the visible rows; keeps the current match if its row is still a match.
    pub(crate) fn rebuild<'a>(&mut self, rows: impl Iterator<Item = (u64, &'a str)>) {
        self.matches.clear();
        for (seq, text) in rows {
            self.offer(seq, text);
        }
        if self
            .current
            .is_some_and(|c| self.matches.binary_search(&c).is_err())
        {
            self.current = None;
        }
    }

    /// Forgets matches in lines that left the ring.
    pub(crate) fn evict_before(&mut self, first_seq: u64) {
        while self.matches.front().is_some_and(|&seq| seq < first_seq) {
            self.matches.pop_front();
        }
        if self.current.is_some_and(|c| c < first_seq) {
            self.current = None;
        }
    }

    pub(crate) fn summary(&self) -> SearchSummary {
        let current = self
            .current
            .and_then(|c| self.matches.binary_search(&c).ok())
            .and_then(|index| u32::try_from(index + 1).ok());
        SearchSummary {
            matches: u32::try_from(self.matches.len()).unwrap_or(u32::MAX),
            current,
        }
    }

    /// Moves to the next (or previous) match, wrapping at the ends, and returns its sequence number.
    pub(crate) fn step(&mut self, forward: bool) -> Option<u64> {
        if self.matches.is_empty() {
            self.current = None;
            return None;
        }
        let next = match (self.current, forward) {
            (Some(c), true) => {
                let at = self.matches.partition_point(|&seq| seq <= c);
                self.matches.get(at).or_else(|| self.matches.front())
            }
            (Some(c), false) => {
                let at = self.matches.partition_point(|&seq| seq < c);
                at.checked_sub(1)
                    .and_then(|i| self.matches.get(i))
                    .or_else(|| self.matches.back())
            }
            (None, true) => self.matches.front(),
            (None, false) => self.matches.back(),
        };
        self.current = next.copied();
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(text: &str, regex: bool, case_sensitive: bool) -> Query {
        Query {
            text: text.to_string(),
            regex,
            case_sensitive,
        }
    }

    fn matcher(text: &str, regex: bool, case_sensitive: bool) -> Matcher {
        Matcher::new(&query(text, regex, case_sensitive))
            .expect("valid")
            .expect("non-empty")
    }

    #[test]
    fn plain_text_is_literal_and_case_insensitive_by_default() {
        let m = matcher("a.b", false, false);
        assert!(m.is_match("xx A.B xx"));
        assert!(!m.is_match("axb"));
    }

    #[test]
    fn case_sensitive_plain_text() {
        let m = matcher("Error", false, true);
        assert!(m.is_match("an Error"));
        assert!(!m.is_match("an error"));
    }

    #[test]
    fn regex_matches_and_reports_byte_ranges() {
        let m = matcher(r"ECONN\w+", true, true);
        assert_eq!(m.ranges("x ECONNREFUSED y ECONNRESET"), vec![2..14, 17..27]);
    }

    #[test]
    fn invalid_regex_is_an_error_naming_the_pattern_problem() {
        let err = Matcher::new(&query("a(", true, false)).expect_err("invalid");
        assert!(matches!(err, LogsError::Regex(ref m) if m.contains("unclosed group")));
        assert!(err.to_string().starts_with("invalid pattern: "));
    }

    #[test]
    fn empty_query_is_no_search_and_empty_matches_have_no_highlight() {
        assert!(Matcher::new(&query("", false, false))
            .expect("ok")
            .is_none());
        assert!(matcher("x*", true, false).ranges("abc").is_empty());
    }

    #[test]
    fn stepping_wraps_and_counts_from_one() {
        let mut s = SearchState::new(matcher("hit", false, false));
        for (seq, text) in [(3, "hit"), (4, "miss"), (7, "HIT"), (9, "hit")] {
            s.offer(seq, text);
        }
        assert_eq!(
            s.summary(),
            SearchSummary {
                matches: 3,
                current: None
            }
        );
        assert_eq!(s.step(true), Some(3));
        assert_eq!(s.step(true), Some(7));
        assert_eq!(s.summary().current, Some(2));
        assert_eq!(s.step(true), Some(9));
        assert_eq!(s.step(true), Some(3));
        assert_eq!(s.step(false), Some(9));
        assert_eq!(s.step(false), Some(7));
    }

    #[test]
    fn eviction_forgets_old_matches_and_the_current_one() {
        let mut s = SearchState::new(matcher("hit", false, false));
        for seq in [1, 2, 3] {
            s.offer(seq, "hit");
        }
        s.step(true);
        s.evict_before(2);
        assert_eq!(
            s.summary(),
            SearchSummary {
                matches: 2,
                current: None
            }
        );
    }
}
