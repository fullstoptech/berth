//! Diffs with `similar`. P5-T2 implements.

/// `similar`, 3 lines of context.
// P5-T2 implements
#[must_use]
pub fn file_diff(
    rel_path: &std::path::Path,
    _old: Option<&str>,
    _new: Option<&str>,
) -> berth_core::FileDiff {
    berth_core::FileDiff {
        path: rel_path.to_path_buf(),
        change: berth_core::FileChange::Create,
        old_sha256: None,
        new_text: None,
        hunks: Vec::new(),
        added: 0,
        removed: 0,
    }
}

// P5-T2 implements
#[must_use]
pub fn sha256(_text: &str) -> String {
    String::new()
}
