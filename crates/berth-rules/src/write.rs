//! Guarded plan application (D31, ADR 0012).
//!
//! `apply` works in three passes so that a failed check writes nothing:
//! 1. check every file (inside the folder, hash still matches, nothing in the way);
//! 2. stage every new text in a temp file next to its target;
//! 3. rename the staged files into place, undoing the renames already done if one fails.

use std::collections::HashSet;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

use berth_core::{FileChange, FileDiff, GenerationPlan};
use tempfile::NamedTempFile;

use crate::diff::sha256_bytes;
use crate::RulesError;

#[derive(Debug)]
pub struct WriteReport {
    /// The files created, changed or removed, each as `plan.root` joined with the plan's path.
    pub written: Vec<PathBuf>,
}

/// One checked file, ready to stage.
struct Step {
    change: FileChange,
    target: PathBuf,
    /// The file's bytes before the write (`Modify`, `Delete`), kept for the undo.
    old: Option<Vec<u8>>,
    mode: Option<std::fs::Permissions>,
    new_text: Option<String>,
}

enum Undo {
    Remove(PathBuf),
    Restore(PathBuf, Vec<u8>, Option<std::fs::Permissions>),
}

/// D31: same-folder temp file + rename; refuses outside root and on hash mismatch; writes nothing if any check fails.
///
/// Errors: `OutsideRoot` for a path that leaves the folder (absolute, `..`, or through a symlink),
/// `Conflict` when a file differs from the hash in the plan (or a file to create already exists),
/// `Io` or `Read` otherwise.
pub fn apply(plan: &GenerationPlan) -> Result<WriteReport, RulesError> {
    let root = std::fs::canonicalize(&plan.root)
        .map_err(|e| RulesError::Read(plan.root.clone(), e.to_string()))?;

    let mut steps = Vec::with_capacity(plan.files.len());
    let mut seen = HashSet::new();
    for file in &plan.files {
        let target = resolve(&root, &plan.root, &file.path)?;
        if !seen.insert(target.clone()) {
            return Err(RulesError::Read(
                file.path.clone(),
                "listed twice in the plan".to_string(),
            ));
        }
        steps.push(check(file, target)?);
    }

    let mut created_dirs = Vec::new();
    let staged = match stage(&steps, &mut created_dirs) {
        Ok(staged) => staged,
        Err(e) => {
            remove_dirs(&created_dirs);
            return Err(e);
        }
    };

    let mut undo = Vec::new();
    if let Err(e) = commit(&steps, staged, &mut undo) {
        for u in undo.into_iter().rev() {
            match u {
                Undo::Remove(p) => {
                    let _ = std::fs::remove_file(p);
                }
                Undo::Restore(p, bytes, mode) => {
                    let _ = std::fs::write(&p, bytes);
                    if let Some(m) = mode {
                        let _ = std::fs::set_permissions(&p, m);
                    }
                }
            }
        }
        remove_dirs(&created_dirs);
        return Err(e);
    }

    Ok(WriteReport {
        written: plan.files.iter().map(|f| plan.root.join(&f.path)).collect(),
    })
}

/// The real path of `rel` under `root` (already canonical), following symlinks that already exist.
/// Anything that ends outside `root` is `OutsideRoot`.
fn resolve(root: &Path, plan_root: &Path, rel: &Path) -> Result<PathBuf, RulesError> {
    let outside = || RulesError::OutsideRoot(rel.to_path_buf());
    let relative = if rel.is_absolute() {
        rel.strip_prefix(root)
            .or_else(|_| rel.strip_prefix(plan_root))
            .map_err(|_| outside())?
    } else {
        rel
    };
    let mut names = Vec::new();
    for c in relative.components() {
        match c {
            Component::Normal(n) => names.push(n),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(outside());
            }
        }
    }
    if names.is_empty() {
        return Err(outside());
    }

    let mut cur = root.to_path_buf();
    let mut iter = names.into_iter();
    for name in iter.by_ref() {
        let next = cur.join(name);
        match std::fs::symlink_metadata(&next) {
            Ok(meta) if meta.file_type().is_symlink() => {
                // A dangling link cannot be resolved, so it cannot be shown to stay inside.
                let real = std::fs::canonicalize(&next).map_err(|_| outside())?;
                if !real.starts_with(root) {
                    return Err(outside());
                }
                cur = real;
            }
            Ok(_) => cur = next,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                cur = next;
                break;
            }
            Err(e) => return Err(e.into()),
        }
    }
    // What is left does not exist yet, so it holds no symlinks.
    for name in iter {
        cur.push(name);
    }
    Ok(cur)
}

fn check(file: &FileDiff, target: PathBuf) -> Result<Step, RulesError> {
    let exists = target.exists();
    if exists && target.is_dir() {
        return Err(RulesError::Read(
            file.path.clone(),
            "is a folder".to_string(),
        ));
    }
    let new_text = match file.change {
        FileChange::Delete => None,
        FileChange::Create | FileChange::Modify => {
            Some(file.new_text.clone().ok_or_else(|| {
                RulesError::Read(
                    file.path.clone(),
                    "the plan has no content for it".to_string(),
                )
            })?)
        }
    };
    match file.change {
        FileChange::Create => {
            if exists {
                return Err(RulesError::Conflict(file.path.clone()));
            }
            Ok(Step {
                change: FileChange::Create,
                target,
                old: None,
                mode: None,
                new_text,
            })
        }
        FileChange::Modify | FileChange::Delete => {
            let Some(expected) = file.old_sha256.as_deref() else {
                return Err(RulesError::Conflict(file.path.clone()));
            };
            let bytes = match std::fs::read(&target) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(RulesError::Conflict(file.path.clone()));
                }
                Err(e) => return Err(RulesError::Read(file.path.clone(), e.to_string())),
            };
            if sha256_bytes(&bytes) != expected {
                return Err(RulesError::Conflict(file.path.clone()));
            }
            let mode = std::fs::metadata(&target).ok().map(|m| m.permissions());
            Ok(Step {
                change: file.change,
                target,
                old: Some(bytes),
                mode,
                new_text,
            })
        }
    }
}

/// Writes every new text to a temp file in its target's folder. Nothing in the project changes
/// except new empty folders, which the caller removes on failure.
fn stage(
    steps: &[Step],
    created_dirs: &mut Vec<PathBuf>,
) -> Result<Vec<Option<NamedTempFile>>, RulesError> {
    let mut staged = Vec::with_capacity(steps.len());
    for step in steps {
        let Some(text) = &step.new_text else {
            staged.push(None);
            continue;
        };
        let parent = step.target.parent().unwrap_or(Path::new("."));
        make_dirs(parent, created_dirs)?;
        let mut tmp = tempfile::Builder::new()
            .prefix(".berth-tmp-")
            .tempfile_in(parent)?;
        tmp.write_all(text.as_bytes())?;
        tmp.flush()?;
        if let Some(mode) = &step.mode {
            tmp.as_file().set_permissions(mode.clone())?;
        }
        tmp.as_file().sync_all()?;
        staged.push(Some(tmp));
    }
    Ok(staged)
}

fn make_dirs(dir: &Path, created: &mut Vec<PathBuf>) -> Result<(), RulesError> {
    let mut missing = Vec::new();
    let mut cur = dir;
    while !cur.exists() {
        missing.push(cur.to_path_buf());
        match cur.parent() {
            Some(p) => cur = p,
            None => break,
        }
    }
    for d in missing.into_iter().rev() {
        std::fs::create_dir(&d)?;
        created.push(d);
    }
    Ok(())
}

fn remove_dirs(dirs: &[PathBuf]) {
    for d in dirs.iter().rev() {
        let _ = std::fs::remove_dir(d);
    }
}

fn commit(
    steps: &[Step],
    staged: Vec<Option<NamedTempFile>>,
    undo: &mut Vec<Undo>,
) -> Result<(), RulesError> {
    for (step, tmp) in steps.iter().zip(staged) {
        match (step.change, tmp) {
            (FileChange::Create, Some(tmp)) => {
                tmp.persist_noclobber(&step.target)
                    .map_err(|e| RulesError::Io(e.error))?;
                undo.push(Undo::Remove(step.target.clone()));
            }
            (FileChange::Modify, Some(tmp)) => {
                tmp.persist(&step.target)
                    .map_err(|e| RulesError::Io(e.error))?;
                if let Some(old) = &step.old {
                    undo.push(Undo::Restore(
                        step.target.clone(),
                        old.clone(),
                        step.mode.clone(),
                    ));
                }
            }
            (FileChange::Delete, _) => {
                std::fs::remove_file(&step.target)?;
                if let Some(old) = &step.old {
                    undo.push(Undo::Restore(
                        step.target.clone(),
                        old.clone(),
                        step.mode.clone(),
                    ));
                }
            }
            (_, None) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{symlink, PermissionsExt};

    use berth_core::PlanSource;

    use super::*;
    use crate::diff::{file_diff, sha256};

    struct Sandbox {
        _guard: tempfile::TempDir,
        base: PathBuf,
        root: PathBuf,
        outside: PathBuf,
    }

    /// `<tmp>/root` (the project) and `<tmp>/outside` (a neighbour that must never change).
    fn sandbox() -> Sandbox {
        let guard = tempfile::tempdir().expect("tempdir");
        let base = guard.path().canonicalize().expect("canon");
        let root = base.join("root");
        let outside = base.join("outside");
        std::fs::create_dir(&root).expect("root");
        std::fs::create_dir(&outside).expect("outside");
        Sandbox {
            _guard: guard,
            base,
            root,
            outside,
        }
    }

    fn plan(root: &Path, files: Vec<FileDiff>) -> GenerationPlan {
        GenerationPlan {
            root: root.to_path_buf(),
            files,
            source: PlanSource::Editor,
            notes: Vec::new(),
            after_write: None,
        }
    }

    fn create(rel: &str, text: &str) -> FileDiff {
        file_diff(Path::new(rel), None, Some(text))
    }

    fn modify(rel: &str, old: &str, new: &str) -> FileDiff {
        file_diff(Path::new(rel), Some(old), Some(new))
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("read_dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn creates_a_file_in_a_new_subfolder() {
        let s = sandbox();
        let report =
            apply(&plan(&s.root, vec![create("sub/dir/a.txt", "hello\n")])).expect("apply");
        assert_eq!(
            std::fs::read_to_string(s.root.join("sub/dir/a.txt")).expect("read"),
            "hello\n"
        );
        assert_eq!(report.written, vec![s.root.join("sub/dir/a.txt")]);
        assert_eq!(
            entries(&s.root.join("sub/dir")),
            vec!["a.txt"],
            "no temp file left"
        );
    }

    #[test]
    fn modifies_a_file_whose_hash_matches_and_keeps_its_mode() {
        let s = sandbox();
        let f = s.root.join("run.sh");
        std::fs::write(&f, "old\n").expect("seed");
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        apply(&plan(&s.root, vec![modify("run.sh", "old\n", "new\n")])).expect("apply");
        assert_eq!(std::fs::read_to_string(&f).expect("read"), "new\n");
        let mode = std::fs::metadata(&f).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        assert_eq!(entries(&s.root), vec!["run.sh"]);
    }

    #[test]
    fn deletes_a_file_whose_hash_matches() {
        let s = sandbox();
        std::fs::write(s.root.join("gone.txt"), "x\n").expect("seed");
        let del = file_diff(Path::new("gone.txt"), Some("x\n"), None);
        apply(&plan(&s.root, vec![del])).expect("apply");
        assert!(!s.root.join("gone.txt").exists());
    }

    #[test]
    fn an_empty_plan_writes_nothing() {
        let s = sandbox();
        let report = apply(&plan(&s.root, Vec::new())).expect("apply");
        assert!(report.written.is_empty());
    }

    #[test]
    fn refuses_a_dot_dot_path_and_writes_nothing_outside() {
        let s = sandbox();
        let err =
            apply(&plan(&s.root, vec![create("../outside/x.txt", "x")])).expect_err("refused");
        assert!(matches!(err, RulesError::OutsideRoot(_)), "{err:?}");
        assert!(entries(&s.outside).is_empty());
    }

    #[test]
    fn refuses_a_dot_dot_hidden_in_the_middle() {
        let s = sandbox();
        std::fs::create_dir(s.root.join("a")).expect("a");
        let err =
            apply(&plan(&s.root, vec![create("a/../../outside/x", "x")])).expect_err("refused");
        assert!(matches!(err, RulesError::OutsideRoot(_)), "{err:?}");
        assert!(entries(&s.outside).is_empty());
    }

    #[test]
    fn refuses_an_absolute_path_outside_and_accepts_one_inside() {
        let s = sandbox();
        let abs_out = s.outside.join("x.txt");
        let err = apply(&plan(
            &s.root,
            vec![create(abs_out.to_str().expect("utf8"), "x")],
        ))
        .expect_err("refused");
        assert!(matches!(err, RulesError::OutsideRoot(_)), "{err:?}");
        assert!(!abs_out.exists());

        let abs_in = s.root.join("in.txt");
        apply(&plan(
            &s.root,
            vec![create(abs_in.to_str().expect("utf8"), "x")],
        ))
        .expect("inside");
        assert!(abs_in.exists());
    }

    #[test]
    fn refuses_to_write_through_a_symlinked_folder_that_leaves_the_root() {
        let s = sandbox();
        symlink(&s.outside, s.root.join("link")).expect("symlink");
        let err = apply(&plan(&s.root, vec![create("link/x.txt", "x")])).expect_err("refused");
        assert!(matches!(err, RulesError::OutsideRoot(_)), "{err:?}");
        assert!(entries(&s.outside).is_empty());
    }

    #[test]
    fn refuses_to_modify_a_symlinked_file_that_leaves_the_root() {
        let s = sandbox();
        std::fs::write(s.outside.join("secret"), "old\n").expect("seed");
        symlink(s.outside.join("secret"), s.root.join("f")).expect("symlink");
        let err = apply(&plan(&s.root, vec![modify("f", "old\n", "new\n")])).expect_err("refused");
        assert!(matches!(err, RulesError::OutsideRoot(_)), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(s.outside.join("secret")).expect("read"),
            "old\n"
        );
    }

    #[test]
    fn refuses_a_dangling_symlink() {
        let s = sandbox();
        symlink(s.outside.join("nothing"), s.root.join("f")).expect("symlink");
        let err = apply(&plan(&s.root, vec![create("f", "x")])).expect_err("refused");
        assert!(matches!(err, RulesError::OutsideRoot(_)), "{err:?}");
        assert!(!s.outside.join("nothing").exists());
    }

    #[test]
    fn a_symlink_that_stays_inside_the_root_is_followed() {
        let s = sandbox();
        std::fs::create_dir(s.root.join("real")).expect("real");
        symlink(s.root.join("real"), s.root.join("alias")).expect("symlink");
        apply(&plan(&s.root, vec![create("alias/x.txt", "x")])).expect("apply");
        assert!(s.root.join("real/x.txt").exists());
    }

    #[test]
    fn refuses_when_the_file_changed_since_the_plan() {
        let s = sandbox();
        let f = s.root.join("compose.yaml");
        std::fs::write(&f, "services: {}\n").expect("seed");
        let d = modify("compose.yaml", "services: {}\n", "services:\n  a: {}\n");
        std::fs::write(&f, "services: {}\n# edited by hand\n").expect("edit");
        let err = apply(&plan(&s.root, vec![d])).expect_err("conflict");
        assert!(
            matches!(&err, RulesError::Conflict(p) if p == Path::new("compose.yaml")),
            "{err:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&f).expect("read"),
            "services: {}\n# edited by hand\n"
        );
    }

    #[test]
    fn refuses_to_create_a_file_that_has_appeared() {
        let s = sandbox();
        std::fs::write(s.root.join("Dockerfile"), "mine\n").expect("seed");
        let err =
            apply(&plan(&s.root, vec![create("Dockerfile", "generated\n")])).expect_err("conflict");
        assert!(matches!(err, RulesError::Conflict(_)), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(s.root.join("Dockerfile")).expect("read"),
            "mine\n"
        );
    }

    #[test]
    fn a_modify_of_a_vanished_file_is_a_conflict() {
        let s = sandbox();
        let err = apply(&plan(&s.root, vec![modify("gone", "a", "b")])).expect_err("conflict");
        assert!(matches!(err, RulesError::Conflict(_)), "{err:?}");
    }

    #[test]
    fn a_modify_without_a_recorded_hash_is_a_conflict() {
        let s = sandbox();
        std::fs::write(s.root.join("f"), "a").expect("seed");
        let mut d = modify("f", "a", "b");
        d.old_sha256 = None;
        let err = apply(&plan(&s.root, vec![d])).expect_err("conflict");
        assert!(matches!(err, RulesError::Conflict(_)), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(s.root.join("f")).expect("read"),
            "a"
        );
    }

    #[test]
    fn one_failed_check_writes_zero_files() {
        let s = sandbox();
        std::fs::write(s.root.join("b.txt"), "b-old\n").expect("seed b");
        std::fs::write(s.root.join("c.txt"), "c-changed\n").expect("seed c");
        let files = vec![
            create("a.txt", "a\n"),
            create("deep/er/new.txt", "n\n"),
            modify("b.txt", "b-old\n", "b-new\n"),
            modify("c.txt", "c-planned\n", "c-new\n"),
        ];
        let err = apply(&plan(&s.root, files)).expect_err("conflict");
        assert!(
            matches!(&err, RulesError::Conflict(p) if p == Path::new("c.txt")),
            "{err:?}"
        );
        assert_eq!(
            entries(&s.root),
            vec!["b.txt", "c.txt"],
            "no new file or folder"
        );
        assert_eq!(
            std::fs::read_to_string(s.root.join("b.txt")).expect("read"),
            "b-old\n"
        );
        assert_eq!(
            std::fs::read_to_string(s.root.join("c.txt")).expect("read"),
            "c-changed\n"
        );
    }

    #[test]
    fn one_outside_path_among_good_ones_writes_zero_files() {
        let s = sandbox();
        let files = vec![create("a.txt", "a\n"), create("../outside/b.txt", "b\n")];
        let err = apply(&plan(&s.root, files)).expect_err("refused");
        assert!(matches!(err, RulesError::OutsideRoot(_)), "{err:?}");
        assert!(entries(&s.root).is_empty());
        assert!(entries(&s.outside).is_empty());
    }

    #[test]
    fn a_failure_while_staging_leaves_earlier_files_untouched() {
        let s = sandbox();
        std::fs::write(s.root.join("a.txt"), "old\n").expect("seed");
        let ro = s.root.join("ro");
        std::fs::create_dir(&ro).expect("ro");
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).expect("chmod");
        let files = vec![
            modify("a.txt", "old\n", "new\n"),
            create("ro/new.txt", "x\n"),
        ];
        let result = apply(&plan(&s.root, files));
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).expect("chmod back");
        let err = result.expect_err("cannot stage into a read-only folder");
        assert!(matches!(err, RulesError::Io(_)), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(s.root.join("a.txt")).expect("read"),
            "old\n"
        );
        assert_eq!(entries(&s.root), vec!["a.txt", "ro"]);
    }

    #[test]
    fn a_failure_while_renaming_puts_earlier_files_back() {
        let s = sandbox();
        std::fs::write(s.root.join("a.txt"), "old\n").expect("seed a");
        let ro = s.root.join("ro");
        std::fs::create_dir(&ro).expect("ro");
        std::fs::write(ro.join("keep.txt"), "keep\n").expect("seed keep");
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).expect("chmod");
        let files = vec![
            modify("a.txt", "old\n", "new\n"),
            file_diff(Path::new("ro/keep.txt"), Some("keep\n"), None),
        ];
        let result = apply(&plan(&s.root, files));
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).expect("chmod back");
        assert!(matches!(result, Err(RulesError::Io(_))), "{result:?}");
        assert_eq!(
            std::fs::read_to_string(s.root.join("a.txt")).expect("read"),
            "old\n"
        );
        assert_eq!(
            std::fs::read_to_string(ro.join("keep.txt")).expect("read"),
            "keep\n"
        );
        assert_eq!(entries(&s.root), vec!["a.txt", "ro"]);
    }

    #[test]
    fn the_same_path_twice_is_refused() {
        let s = sandbox();
        let files = vec![create("a.txt", "1"), create("./a.txt", "2")];
        let err = apply(&plan(&s.root, files)).expect_err("refused");
        assert!(matches!(err, RulesError::Read(..)), "{err:?}");
        assert!(entries(&s.root).is_empty());
    }

    #[test]
    fn a_missing_root_is_an_error_not_a_panic() {
        let s = sandbox();
        let err = apply(&plan(&s.base.join("nope"), vec![create("a", "x")])).expect_err("no root");
        assert!(matches!(err, RulesError::Read(..)), "{err:?}");
    }

    #[test]
    fn hash_check_uses_the_bytes_on_disk() {
        let s = sandbox();
        std::fs::write(s.root.join("f"), "a\n").expect("seed");
        let mut d = modify("f", "a\n", "b\n");
        d.old_sha256 = Some(sha256("a\r\n"));
        let err = apply(&plan(&s.root, vec![d])).expect_err("conflict");
        assert!(matches!(err, RulesError::Conflict(_)), "{err:?}");
    }
}
