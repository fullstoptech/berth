//! Diffs with `similar`, and the SHA-256 that guards writes (D31).

use std::path::Path;

use berth_core::{DiffHunk, DiffLine, DiffLineKind, FileChange, FileDiff};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};

const CONTEXT_LINES: usize = 3;

/// Builds the diff Review changes shows. `None` for `old` is a new file, `None` for `new` a removed one.
/// `old_sha256` is the hash of `old`; `write::apply` compares it with the file on disk.
#[must_use]
pub fn file_diff(rel_path: &Path, old: Option<&str>, new: Option<&str>) -> FileDiff {
    let change = match (old, new) {
        (None, _) => FileChange::Create,
        (Some(_), None) => FileChange::Delete,
        (Some(_), Some(_)) => FileChange::Modify,
    };
    let diff = TextDiff::from_lines(old.unwrap_or(""), new.unwrap_or(""));
    let mut hunks = Vec::new();
    let mut added = 0u32;
    let mut removed = 0u32;
    for group in diff.grouped_ops(CONTEXT_LINES) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let old_range = first.old_range().start..last.old_range().end;
        let new_range = first.new_range().start..last.new_range().end;
        let mut lines = Vec::new();
        for op in &group {
            for change in diff.iter_changes(op) {
                let kind = match change.tag() {
                    ChangeTag::Insert => {
                        added += 1;
                        DiffLineKind::Added
                    }
                    ChangeTag::Delete => {
                        removed += 1;
                        DiffLineKind::Removed
                    }
                    ChangeTag::Equal => DiffLineKind::Context,
                };
                lines.push(DiffLine {
                    kind,
                    old_no: change.old_index().map(|i| to_u32(i + 1)),
                    new_no: change.new_index().map(|i| to_u32(i + 1)),
                    text: change.value().trim_end_matches(['\n', '\r']).to_string(),
                });
            }
        }
        hunks.push(DiffHunk {
            old_start: start_no(old_range.start, old_range.len()),
            old_len: to_u32(old_range.len()),
            new_start: start_no(new_range.start, new_range.len()),
            new_len: to_u32(new_range.len()),
            lines,
        });
    }
    FileDiff {
        path: rel_path.to_path_buf(),
        change,
        old_sha256: old.map(sha256),
        new_text: new.map(str::to_string),
        hunks,
        added,
        removed,
    }
}

/// Lower-case hex SHA-256 of the text's UTF-8 bytes.
#[must_use]
pub fn sha256(text: &str) -> String {
    sha256_bytes(text.as_bytes())
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Unified-diff numbering: 1-based, and an empty range points at the line before it.
fn start_no(start: usize, len: usize) -> u32 {
    if len == 0 {
        to_u32(start)
    } else {
        to_u32(start + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> &'static Path {
        Path::new("compose.yaml")
    }

    #[test]
    fn sha256_is_lowercase_hex_of_the_known_digest() {
        assert_eq!(
            sha256("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha256("").len(), 64);
    }

    #[test]
    fn new_file_is_all_added_with_numbered_lines() {
        let d = file_diff(p(), None, Some("a\nb\n"));
        assert_eq!(d.change, FileChange::Create);
        assert_eq!(d.old_sha256, None);
        assert_eq!(d.new_text.as_deref(), Some("a\nb\n"));
        assert_eq!((d.added, d.removed), (2, 0));
        assert_eq!(d.hunks.len(), 1);
        let h = &d.hunks[0];
        assert_eq!(
            (h.old_start, h.old_len, h.new_start, h.new_len),
            (0, 0, 1, 2)
        );
        assert_eq!(h.lines[1].new_no, Some(2));
        assert_eq!(h.lines[1].old_no, None);
        assert_eq!(h.lines[1].text, "b");
    }

    #[test]
    fn removed_file_keeps_the_old_hash_and_no_new_text() {
        let d = file_diff(p(), Some("a\n"), None);
        assert_eq!(d.change, FileChange::Delete);
        assert_eq!(d.old_sha256, Some(sha256("a\n")));
        assert_eq!(d.new_text, None);
        assert_eq!((d.added, d.removed), (0, 1));
    }

    #[test]
    fn one_changed_line_gets_three_lines_of_context_each_side() {
        let old: String = (1..=20).map(|i| format!("line{i}\n")).collect();
        let new = old.replace("line10\n", "changed\n");
        let d = file_diff(p(), Some(&old), Some(&new));
        assert_eq!(d.change, FileChange::Modify);
        assert_eq!((d.added, d.removed), (1, 1));
        assert_eq!(d.hunks.len(), 1);
        let h = &d.hunks[0];
        assert_eq!((h.old_start, h.old_len), (7, 7));
        assert_eq!((h.new_start, h.new_len), (7, 7));
        let context = h
            .lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Context)
            .count();
        assert_eq!(context, 6);
        assert_eq!(h.lines[0].text, "line7");
        assert_eq!(h.lines[3].kind, DiffLineKind::Removed);
        assert_eq!(h.lines[3].text, "line10");
        assert_eq!(h.lines[3].old_no, Some(10));
        assert_eq!(h.lines[4].text, "changed");
        assert_eq!(h.lines[4].new_no, Some(10));
    }

    #[test]
    fn distant_changes_make_separate_hunks() {
        let old: String = (1..=30).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l2\n", "x\n").replace("l28\n", "y\n");
        let d = file_diff(p(), Some(&old), Some(&new));
        assert_eq!(d.hunks.len(), 2);
    }

    #[test]
    fn identical_text_has_no_hunks() {
        let d = file_diff(p(), Some("a\n"), Some("a\n"));
        assert_eq!(d.change, FileChange::Modify);
        assert!(d.hunks.is_empty());
        assert_eq!((d.added, d.removed), (0, 0));
    }

    #[test]
    fn crlf_line_endings_do_not_leak_into_line_text() {
        let d = file_diff(p(), Some("a\r\n"), Some("b\r\n"));
        assert_eq!(d.hunks[0].lines[0].text, "a");
    }
}
