//! Tarring a build context (AD43): a folder walked in sorted order
//! with its `.dockerignore` honoured, or one inline Dockerfile. The
//! matcher implements the common subset of Docker's rules — comments,
//! `!` negation, `*` and `?` inside one path segment, `**` across
//! segments, patterns anchored at the context root, and a path under
//! an excluded directory is excluded. It is not a full port of
//! Docker's patternmatcher (no character classes, no escaping).
//!
//! The Dockerfile itself is always included, as the Docker CLI does,
//! even when `.dockerignore` names it. Symlinks are stored as links.

use std::path::{Path, PathBuf};

use crate::EngineError;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pattern {
    negate: bool,
    segments: Vec<String>,
    dir_only: bool,
}

#[derive(Debug, Default)]
pub struct Ignore {
    patterns: Vec<Pattern>,
}

impl Ignore {
    /// Read `<folder>/.dockerignore`; a missing file ignores nothing.
    pub fn load(folder: &Path) -> Self {
        let text = std::fs::read_to_string(folder.join(".dockerignore")).unwrap_or_default();
        Self::parse(&text)
    }

    fn parse(text: &str) -> Self {
        let patterns = text
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                let (negate, line) = match line.strip_prefix('!') {
                    Some(rest) => (true, rest.trim()),
                    None => (false, line),
                };
                let line = line.strip_prefix('/').unwrap_or(line);
                let (dir_only, line) = match line.strip_suffix('/') {
                    Some(rest) => (true, rest),
                    None => (false, line),
                };
                if line.is_empty() {
                    return None;
                }
                Some(Pattern {
                    negate,
                    segments: line.split('/').map(str::to_string).collect(),
                    dir_only,
                })
            })
            .collect();
        Self { patterns }
    }

    fn has_negations(&self) -> bool {
        self.patterns.iter().any(|p| p.negate)
    }

    /// Whether the slash-relative path is excluded. The last matching
    /// pattern decides, as in Docker.
    #[must_use]
    pub fn is_ignored(&self, rel: &str, is_dir: bool) -> bool {
        let parts: Vec<&str> = rel.split('/').collect();
        let mut ignored = false;
        for pattern in &self.patterns {
            if pattern.dir_only && !is_dir {
                // A directory pattern still excludes what is under it:
                // the parent check below runs the pattern as a dir.
                if !matches_any_parent(pattern, &parts) {
                    continue;
                }
            } else if !segments_match(&pattern.segments, &parts)
                && !matches_any_parent(pattern, &parts)
            {
                continue;
            }
            ignored = !pattern.negate;
        }
        ignored
    }
}

/// A pattern matches a path when it matches one of its parent
/// directories (`node_modules` excludes `node_modules/x/index.js`).
fn matches_any_parent(pattern: &Pattern, parts: &[&str]) -> bool {
    (1..parts.len()).any(|n| segments_match(&pattern.segments, &parts[..n]))
}

fn segments_match(pattern: &[String], parts: &[&str]) -> bool {
    match (pattern.first(), parts.first()) {
        (None, None) => true,
        (Some(head), _) if head == "**" => {
            // `**` consumes zero or more segments.
            (0..=parts.len()).any(|skip| segments_match(&pattern[1..], &parts[skip..]))
        }
        (Some(head), Some(part)) => {
            segment_match(head, part) && segments_match(&pattern[1..], &parts[1..])
        }
        _ => false,
    }
}

/// One path segment: `*` any run, `?` one character, rest literal.
fn segment_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    glob_match(&pattern, &text)
}

fn glob_match(pattern: &[char], text: &[char]) -> bool {
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some('*'), _) => (0..=text.len()).any(|skip| glob_match(&pattern[1..], &text[skip..])),
        (Some('?'), Some(_)) => glob_match(&pattern[1..], &text[1..]),
        (Some(p), Some(t)) => p == t && glob_match(&pattern[1..], &text[1..]),
        _ => false,
    }
}

fn io_err(err: std::io::Error) -> EngineError {
    EngineError::Docker(format!("build context: {err}"))
}

/// The entries of `folder` the tar will carry: slash-relative paths,
/// sorted, directories included, the Dockerfile always present.
fn collect(folder: &Path, dockerfile: &str) -> Result<Vec<PathBuf>, EngineError> {
    let ignore = Ignore::load(folder);
    let descend_all = ignore.has_negations();
    let mut out = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(io_err)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries {
            let rel = path
                .strip_prefix(folder)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let is_dir = path.is_dir() && !path.is_symlink();
            if rel == dockerfile {
                out.push(path);
                continue;
            }
            if ignore.is_ignored(&rel, is_dir) {
                if is_dir && descend_all {
                    stack.push(path);
                }
                continue;
            }
            if is_dir {
                stack.push(path.clone());
            }
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Tar `folder` for `POST /build`, honouring `.dockerignore`.
///
/// # Errors
/// The folder or one of its files cannot be read.
pub fn tar_folder(folder: &Path, dockerfile: &str) -> Result<bytes::Bytes, EngineError> {
    let mut builder = tar::Builder::new(Vec::new());
    builder.follow_symlinks(false);
    for path in collect(folder, dockerfile)? {
        let rel = path
            .strip_prefix(folder)
            .map_err(|e| EngineError::Docker(format!("build context: {e}")))?
            .to_path_buf();
        builder.append_path_with_name(&path, &rel).map_err(io_err)?;
    }
    Ok(bytes::Bytes::from(builder.into_inner().map_err(io_err)?))
}

/// Tar one inline Dockerfile: it is the whole context (AD43).
///
/// # Errors
/// The in-memory tar cannot be written (does not happen in practice).
pub fn tar_inline(dockerfile_text: &str) -> Result<bytes::Bytes, EngineError> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(dockerfile_text.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "Dockerfile", dockerfile_text.as_bytes())
        .map_err(io_err)?;
    Ok(bytes::Bytes::from(builder.into_inner().map_err(io_err)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_context_ignore_reads_patterns_negation_and_dirs() {
        let ignore =
            Ignore::parse("# comment\n*.log\nnode_modules\nbuild/\n!important.log\n**/tmp\n");
        assert!(ignore.is_ignored("app.log", false));
        assert!(!ignore.is_ignored("important.log", false));
        assert!(ignore.is_ignored("node_modules", true));
        assert!(ignore.is_ignored("node_modules/x/index.js", false));
        assert!(ignore.is_ignored("build", true));
        assert!(ignore.is_ignored("build/out.o", false));
        assert!(!ignore.is_ignored("buildx", true));
        assert!(ignore.is_ignored("a/b/tmp", true));
        assert!(ignore.is_ignored("tmp", false));
        assert!(!ignore.is_ignored("src/main.rs", false));
        assert!(
            !ignore.is_ignored("deep/app.log", false),
            "anchored at root"
        );
    }

    #[test]
    fn build_context_tar_keeps_dockerfile_and_skips_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(root.join("Dockerfile"), "FROM scratch\n").expect("write");
        std::fs::write(root.join(".dockerignore"), "Dockerfile\nsecret.txt\n").expect("write");
        std::fs::write(root.join("secret.txt"), "nope").expect("write");
        std::fs::write(root.join("index.html"), "<p>hi</p>").expect("write");
        std::fs::create_dir(root.join("sub")).expect("mkdir");
        std::fs::write(root.join("sub/a.txt"), "a").expect("write");

        let tarred = tar_folder(root, "Dockerfile").expect("tar");
        let mut archive = tar::Archive::new(std::io::Cursor::new(tarred));
        let names: Vec<String> = archive
            .entries()
            .expect("entries")
            .filter_map(Result::ok)
            .map(|e| e.path().expect("path").to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().any(|n| n == "Dockerfile"), "{names:?}");
        assert!(names.iter().any(|n| n == "index.html"), "{names:?}");
        assert!(names.iter().any(|n| n == "sub/a.txt"), "{names:?}");
        assert!(!names.iter().any(|n| n == "secret.txt"), "{names:?}");
    }

    #[test]
    fn build_context_inline_tar_holds_only_the_dockerfile() {
        let tarred = tar_inline("FROM alpine:3\n").expect("tar");
        let mut archive = tar::Archive::new(std::io::Cursor::new(tarred));
        let mut entries = archive.entries().expect("entries");
        let mut entry = entries.next().expect("one").expect("entry");
        assert_eq!(entry.path().expect("path").to_string_lossy(), "Dockerfile");
        let mut text = String::new();
        std::io::Read::read_to_string(&mut entry, &mut text).expect("read");
        assert_eq!(text, "FROM alpine:3\n");
        assert!(entries.next().is_none());
    }
}
