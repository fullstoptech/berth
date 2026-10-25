//! Stack-trace detection. A trace is a head line (`Error: x`, `Traceback …`, `panic: …`) followed
//! by continuation lines; the buffer folds the continuation lines under the head.

use std::collections::HashMap;
use std::sync::LazyLock;

use berth_core::{ContainerId, Stream};
use regex::Regex;

/// `    at …` (Java, Node), `  File "…", line N` (Python), `Caused by:`, `... N more`,
/// `goroutine N [` and `created by` (Go), and a tab-indented `path.go:N` (Go).
const CONTINUATION_PATTERN: &str = r#"(?x)
    ^\s+at\s+\S.*(?:\)|:\d+(?::\d+)?)\s*$
  | ^\s+File\s+".*",\s+line\s+\d+
  | ^\s*Caused\ by:
  | ^\s*\.\.\.\s+\d+\s+(?:more|common\ frames\ omitted)
  | ^goroutine\s+\d+\s+\[
  | ^created\ by\s
  | ^\t.+\.go:\d+
"#;

// `None` when the constant fails to compile; `patterns_compile` guards that.
static CONTINUATION_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(CONTINUATION_PATTERN).ok());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Python,
    Go,
    Other,
}

/// What kind of trace line this is on its own, ignoring its neighbours.
fn own_kind(text: &str) -> Option<Kind> {
    let re = CONTINUATION_RE.as_ref()?;
    if !re.is_match(text) {
        return None;
    }
    let trimmed = text.trim_start();
    Some(if trimmed.starts_with("File \"") {
        Kind::Python
    } else if text.starts_with("goroutine")
        || text.starts_with("created by")
        || text.contains(".go:")
    {
        Kind::Go
    } else {
        Kind::Other
    })
}

/// True for `    at …`, `\tat …`, `  File "…", line N`, `Caused by:`, `... N more` and Go
/// goroutine frames. Lines that only follow such a line (a Python source line, a Go function
/// line) are decided by the buffer from the line before.
#[must_use]
pub fn is_stack_continuation(text: &str) -> bool {
    own_kind(text).is_some()
}

/// `main.handler(0xc0001, 0x5)`: no whitespace before the first `(`, ends with `)`.
fn is_go_function_line(text: &str) -> bool {
    text.find('(').is_some_and(|i| {
        i > 0 && !text[..i].contains(char::is_whitespace) && text.trim_end().ends_with(')')
    })
}

fn is_panic_head(text: &str) -> bool {
    text.starts_with("panic:") || text.starts_with("fatal error:")
}

/// What a line's text alone says about folding, computed without any state so that it can run
/// before the view lock is taken (D23). [`FoldTracker::place_with`] combines it with the previous
/// line of the same container and stream, which is the part that has to run in order.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FoldFacts {
    own: Option<Kind>,
    blank: bool,
    leading_whitespace: bool,
    leading_tab: bool,
    go_function: bool,
    panic_head: bool,
}

impl FoldFacts {
    pub(crate) fn of(text: &str) -> Self {
        Self {
            own: own_kind(text),
            blank: text.trim().is_empty(),
            leading_whitespace: text.starts_with(char::is_whitespace),
            leading_tab: text.starts_with('\t'),
            go_function: is_go_function_line(text),
            panic_head: is_panic_head(text),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Last {
    seq: u64,
    /// Head of the trace this line belongs to; its own `seq` when it is not a member.
    head: u64,
    member: bool,
    kind: Kind,
    panic_head: bool,
}

impl Last {
    fn head_for_next(&self) -> u64 {
        if self.member {
            self.head
        } else {
            self.seq
        }
    }
}

/// Remembers, per container and output stream, the last line seen. The engine's log interleaves
/// stdout and stderr in whatever order it read the two pipes (a stdout line can land between two
/// frames of a stderr trace), so a trace must follow its own stream, not the container's line order.
#[derive(Debug, Default)]
pub(crate) struct FoldTracker {
    /// `[stdout, stderr]` per container. Keyed by the container alone so that a lookup clones
    /// nothing: this runs for every line under the view lock.
    last: HashMap<ContainerId, [Option<Last>; 2]>,
}

impl FoldTracker {
    /// Records the line at `seq` and returns the head `seq` it folds under, if it continues a trace.
    #[cfg(test)]
    pub(crate) fn place(
        &mut self,
        container: &ContainerId,
        stream: Stream,
        seq: u64,
        text: &str,
    ) -> Option<u64> {
        self.place_with(container, stream, seq, &FoldFacts::of(text))
    }

    /// [`place`](Self::place) for a line whose [`FoldFacts`] are already known.
    pub(crate) fn place_with(
        &mut self,
        container: &ContainerId,
        stream: Stream,
        seq: u64,
        facts: &FoldFacts,
    ) -> Option<u64> {
        if !self.last.contains_key(container) {
            self.last.insert(container.clone(), [None, None]);
        }
        let slots = self.last.get_mut(container)?;
        let slot = &mut slots[match stream {
            Stream::Stdout => 0,
            Stream::Stderr => 1,
        }];
        let prev = *slot;
        let kind = prev.and_then(|p| continuation_kind(facts, &p));
        let (head, entry) = match (kind, prev) {
            (Some(kind), Some(p)) => {
                let head = p.head_for_next();
                (
                    Some(head),
                    Last {
                        seq,
                        head,
                        member: true,
                        kind,
                        panic_head: false,
                    },
                )
            }
            _ => (
                None,
                Last {
                    seq,
                    head: seq,
                    member: false,
                    kind: Kind::Other,
                    panic_head: facts.panic_head,
                },
            ),
        };
        *slot = Some(entry);
        head
    }

    pub(crate) fn clear(&mut self) {
        self.last.clear();
    }
}

fn continuation_kind(facts: &FoldFacts, prev: &Last) -> Option<Kind> {
    if let Some(kind) = facts.own {
        return Some(kind);
    }
    match prev.kind {
        // The source line under a `File "…"` frame, and the `~~~^^` marker lines.
        Kind::Python if prev.member && !facts.blank && facts.leading_whitespace => {
            Some(Kind::Python)
        }
        // The function line above a `\t/app/main.go:12` frame, the frame lines, blank separators.
        Kind::Go if prev.member && (facts.blank || facts.leading_tab || facts.go_function) => {
            Some(Kind::Go)
        }
        _ if !prev.member && prev.panic_head && facts.blank => Some(Kind::Go),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_compile() {
        assert!(CONTINUATION_RE.is_some());
    }

    #[test]
    fn recognises_the_continuation_shapes() {
        let yes = [
            "    at TCPConnectWrap.afterConnect [as oncomplete] (node:net:1555:16)",
            "\tat com.example.db.Pool.acquire(Pool.java:88)",
            "    at /app/src/orders.js:18:5",
            "  File \"/app/worker.py\", line 42, in <module>",
            "Caused by: java.net.SocketTimeoutException: connect timed out",
            "\t... 12 more",
            "... 5 common frames omitted",
            "goroutine 1 [running]:",
            "\t/app/main.go:27 +0x1d",
            "created by main.start in goroutine 1",
        ];
        for text in yes {
            assert!(is_stack_continuation(text), "{text}");
        }
    }

    #[test]
    fn ordinary_lines_are_not_continuations() {
        let no = [
            "Error: connect ECONNREFUSED 10.0.0.5:5432",
            "  at least three retries remain",
            "File not found",
            "ValueError: bad payload",
            "panic: runtime error: index out of range",
            "2026-10-24 INFO started",
            "",
        ];
        for text in no {
            assert!(!is_stack_continuation(text), "{text}");
        }
    }

    fn fold_all(lines: &[&str]) -> Vec<Option<u64>> {
        let mut tracker = FoldTracker::default();
        let id = ContainerId("c".to_string());
        lines
            .iter()
            .enumerate()
            .map(|(i, text)| tracker.place(&id, Stream::Stdout, i as u64, text))
            .collect()
    }

    #[test]
    fn python_source_lines_fold_with_their_frames() {
        let got = fold_all(&[
            "Traceback (most recent call last):",
            "  File \"a.py\", line 1, in <module>",
            "    main()",
            "  File \"a.py\", line 2, in main",
            "    raise ValueError()",
            "ValueError: x",
            "next",
        ]);
        assert_eq!(
            got,
            vec![None, Some(0), Some(0), Some(0), Some(0), None, None]
        );
    }

    #[test]
    fn go_panic_folds_blank_function_and_frame_lines() {
        let got = fold_all(&[
            "panic: boom",
            "",
            "goroutine 1 [running]:",
            "main.f(0x1)",
            "\t/app/main.go:3 +0x1",
            "exit status 2",
            "starting (v2)",
        ]);
        assert_eq!(
            got,
            vec![None, Some(0), Some(0), Some(0), Some(0), None, None]
        );
    }

    #[test]
    fn a_continuation_without_a_head_stays_plain() {
        assert_eq!(fold_all(&["    at foo (a.js:1:1)"]), vec![None]);
    }

    #[test]
    fn interleaved_containers_fold_independently() {
        let mut tracker = FoldTracker::default();
        let a = ContainerId("a".to_string());
        let b = ContainerId("b".to_string());
        assert_eq!(tracker.place(&a, Stream::Stdout, 0, "Error: x"), None);
        assert_eq!(tracker.place(&b, Stream::Stdout, 1, "info"), None);
        assert_eq!(
            tracker.place(&a, Stream::Stdout, 2, "    at f (a.js:1:1)"),
            Some(0)
        );
        assert_eq!(
            tracker.place(&b, Stream::Stdout, 3, "    at g (b.js:1:1)"),
            Some(1)
        );
    }

    #[test]
    fn a_trace_follows_its_own_stream_through_interleaved_stdout() {
        let mut tracker = FoldTracker::default();
        let id = ContainerId("c".to_string());
        let place =
            |tracker: &mut FoldTracker, stream, seq, text| tracker.place(&id, stream, seq, text);
        assert_eq!(
            place(
                &mut tracker,
                Stream::Stderr,
                0,
                "Traceback (most recent call last):"
            ),
            None
        );
        assert_eq!(
            place(
                &mut tracker,
                Stream::Stderr,
                1,
                "  File \"a.py\", line 1, in f"
            ),
            Some(0)
        );
        // The engine delivers a stdout line in the middle of the stderr trace.
        assert_eq!(place(&mut tracker, Stream::Stdout, 2, "INFO started"), None);
        assert_eq!(
            place(
                &mut tracker,
                Stream::Stderr,
                3,
                "  File \"a.py\", line 2, in g"
            ),
            Some(0)
        );
        assert_eq!(
            place(&mut tracker, Stream::Stderr, 4, "ValueError: x"),
            None
        );
    }
}
