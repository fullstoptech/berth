//! The diff view of Review changes (P5-T3; frames "Review changes / default / 1440" and "... / new
//! project / 1440"): the file list with +/- counts, and one card per file with a unified diff and
//! line numbers.
//!
//! `.env` and `.env.*` (not `.env.example`) are masked here, before anything is drawn: the view
//! models ([`FileView`]) carry the masked text, so a component cannot print a value it was not
//! given. One Reveal per file shows them (CLAUDE.md: secrets).

use std::path::Path;

use dioxus::prelude::*;

use berth_core::{DiffLineKind, FileChange, FileDiff};

use super::icon::{Icon, IconSize};

/// The view's CSS, rendered with it (tokens only).
pub const DIFF_VIEW_CSS: &str = include_str!("../../assets/css/diff_view.css");

/// What a masked value shows. A fixed count, so the length of a secret is not told.
pub const MASK: &str = "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}";

/// A file longer than this many diff lines shows its first lines and a "Show N more lines" row.
pub const COLLAPSE_AT: usize = 39;

/// `.env` and `.env.*` hold secrets; `.env.example` is the one documented to hold none. The view
/// masks every value after `=` in the others.
#[must_use]
pub fn is_secret_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name == ".env" || (name.starts_with(".env.") && name != ".env.example")
}

/// One line of a secret file as the view shows it: everything after the first `=` is masked. A
/// line that is neither blank, a comment nor `KEY=value` can be the continuation of a quoted
/// multi-line value (a private key), so it is masked whole.
#[must_use]
pub fn mask_line(line: &str) -> String {
    match line.split_once('=') {
        Some((key, _)) => format!("{key}={MASK}"),
        None if line.trim().is_empty() || line.trim_start().starts_with('#') => line.to_string(),
        None => MASK.to_string(),
    }
}

/// The key of a `KEY=value` line (after an optional `export`), for the "Adds 2 keys" preview.
#[must_use]
pub fn key_of(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (key, _) = line.split_once('=')?;
    let key = key.trim();
    (!key.is_empty()).then_some(key)
}

/// One diff line, already masked when its file is secret and not revealed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineView {
    pub kind: DiffLineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
    /// "value changed" on a masked line whose key also has a removed line in the same hunk: with
    /// the values hidden, the two lines would read as identical (CLAUDE.md: secrets; the value
    /// is never shown to say so).
    pub note: Option<&'static str>,
    /// The line starts a new hunk: a gap in the file comes before it.
    pub gap_before: bool,
}

/// Shown after a masked line whose value changed.
pub const VALUE_CHANGED: &str = "value changed";

/// One file of a plan, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileView {
    pub path: String,
    pub change: FileChange,
    pub added: u32,
    pub removed: u32,
    /// The file's values are masked (a secret file that is not revealed).
    pub masked: bool,
    /// The file is a secret file, so the card offers Reveal.
    pub secret: bool,
    pub lines: Vec<LineView>,
}

/// The view of one file. `revealed` shows the real values of a secret file.
#[must_use]
pub fn file_view(diff: &FileDiff, revealed: bool) -> FileView {
    let secret = is_secret_file(&diff.path);
    let masked = secret && !revealed;
    let mut lines = Vec::new();
    for hunk in &diff.hunks {
        // Keys a masked file loses a line for in this hunk: an added line for one of them is a
        // changed value, which masking would otherwise hide.
        let removed_keys: Vec<&str> = if masked {
            hunk.lines
                .iter()
                .filter(|l| l.kind == DiffLineKind::Removed)
                .filter_map(|l| key_of(&l.text))
                .collect()
        } else {
            Vec::new()
        };
        for (index, line) in hunk.lines.iter().enumerate() {
            let gap_before = index == 0 && !lines.is_empty();
            let changed = line.kind == DiffLineKind::Added
                && key_of(&line.text).is_some_and(|key| removed_keys.contains(&key));
            lines.push(LineView {
                kind: line.kind,
                old_no: line.old_no,
                new_no: line.new_no,
                text: if masked {
                    mask_line(&line.text)
                } else {
                    line.text.clone()
                },
                note: changed.then_some(VALUE_CHANGED),
                gap_before,
            });
        }
    }
    FileView {
        path: diff.path.display().to_string(),
        change: diff.change,
        added: diff.added,
        removed: diff.removed,
        masked,
        secret,
        lines,
    }
}

/// The keys a secret file gains, for the preview under the file list: `KEY=••••••••` for each
/// added `KEY=value` line. The value is masked unless the file is revealed.
#[must_use]
pub fn added_key_lines(diff: &FileDiff, revealed: bool) -> Vec<String> {
    let masked = is_secret_file(&diff.path) && !revealed;
    diff.hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| l.kind == DiffLineKind::Added && key_of(&l.text).is_some())
        .map(|l| {
            if masked {
                mask_line(&l.text)
            } else {
                l.text.clone()
            }
        })
        .collect()
}

// ---------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------

fn change_tag(change: FileChange) -> Option<&'static str> {
    match change {
        FileChange::Create => Some("new file"),
        FileChange::Delete => Some("removed"),
        FileChange::Modify => None,
    }
}

/// "+9" and "\u{2212}2", the counts of a file; a zero side is left out.
fn counts(file: &FileView) -> (Option<String>, Option<String>) {
    (
        (file.added > 0).then(|| format!("+{}", file.added)),
        (file.removed > 0).then(|| format!("\u{2212}{}", file.removed)),
    )
}

/// The list of changed files, one row each with its counts. The selected row is highlighted.
#[component]
pub fn DiffFileList(
    files: Vec<FileView>,
    selected: usize,
    #[props(default)] on_select: EventHandler<usize>,
) -> Element {
    rsx! {
        div { class: "dv-files-label", "Changed files" }
        ul { class: "dv-files", role: "list", "aria-label": "Changed files",
            for (index, file) in files.iter().enumerate() {
                {
                    let (added, removed) = counts(file);
                    rsx! {
                        li { key: "{file.path}",
                            button {
                                class: if index == selected { "dv-file dv-file-selected" } else { "dv-file" },
                                r#type: "button",
                                "aria-current": (index == selected).to_string(),
                                onclick: move |_| on_select.call(index),
                                span { class: "dv-file-icon",
                                    Icon { name: "file-text", size: IconSize::Sm }
                                }
                                span { class: "dv-file-name", "{file.path}" }
                                span { class: "dv-file-counts",
                                    if let Some(added) = added {
                                        span { class: "dv-count-added", "{added}" }
                                    }
                                    if let Some(removed) = removed {
                                        span { class: "dv-count-removed", "{removed}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One file's diff: header with counts, then the lines. A long file shows its first
/// [`COLLAPSE_AT`] lines until `expanded`. A secret file offers Reveal values / Hide values.
#[component]
pub fn DiffCard(
    file: FileView,
    index: usize,
    expanded: bool,
    #[props(default)] on_expand: EventHandler<()>,
    #[props(default)] on_toggle_reveal: EventHandler<()>,
) -> Element {
    let (added, removed) = counts(&file);
    let total = file.lines.len();
    let shown = if expanded {
        total
    } else {
        total.min(COLLAPSE_AT)
    };
    let hidden = total - shown;
    let tag = change_tag(file.change);
    rsx! {
        section {
            class: "dv-card",
            id: "dv-file-{index}",
            role: "group",
            "aria-label": "Changes to {file.path}",
            header { class: "dv-card-head",
                span { class: "dv-file-icon",
                    Icon { name: "file-text", size: IconSize::Sm }
                }
                span { class: "dv-path", "{file.path}" }
                if let Some(added) = added {
                    span { class: "dv-count-added", "{added}" }
                }
                if let Some(removed) = removed {
                    span { class: "dv-count-removed", "{removed}" }
                }
                if let Some(tag) = tag {
                    span { class: "dv-tag", "{tag}" }
                }
                span { class: "dv-spacer" }
                if file.secret {
                    button {
                        class: "dv-reveal",
                        r#type: "button",
                        "aria-pressed": (!file.masked).to_string(),
                        onclick: move |_| on_toggle_reveal.call(()),
                        Icon { name: "eye", size: IconSize::Sm }
                        span {
                            if file.masked { "Reveal values" } else { "Hide values" }
                        }
                    }
                }
            }
            div { class: "dv-lines",
                if total == 0 {
                    div { class: "dv-empty", "No changes in this file." }
                }
                for (n, line) in file.lines.iter().take(shown).enumerate() {
                    Fragment { key: "{n}",
                        if line.gap_before {
                            div { class: "dv-gap", "aria-hidden": "true", "\u{22ef}" }
                        }
                        div {
                            class: match line.kind {
                                DiffLineKind::Added => "dv-line dv-line-added",
                                DiffLineKind::Removed => "dv-line dv-line-removed",
                                DiffLineKind::Context => "dv-line",
                            },
                            span { class: "dv-no",
                                if let Some(no) = line.old_no {
                                    "{no}"
                                }
                            }
                            span { class: "dv-no",
                                if let Some(no) = line.new_no {
                                    "{no}"
                                }
                            }
                            span { class: "dv-sign",
                                match line.kind {
                                    DiffLineKind::Added => "+",
                                    DiffLineKind::Removed => "\u{2212}",
                                    DiffLineKind::Context => "",
                                }
                            }
                            span { class: "dv-text", "{line.text}" }
                            if let Some(note) = line.note {
                                span { class: "dv-note", "{note}" }
                            }
                        }
                    }
                }
            }
            if hidden > 0 {
                button {
                    class: "dv-more",
                    r#type: "button",
                    onclick: move |_| on_expand.call(()),
                    "Show {hidden} more lines"
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "diff_view_tests.rs"]
mod tests;
