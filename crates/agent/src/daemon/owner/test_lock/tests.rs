use std::fs::File;
use std::path::Path;

use super::*;

/// The worktree in the tests.
const WORKTREE: &str = "/home/example/agents/worktrees/team-agents/task-20261125-000000-aaaa";

/// A lock file that says `text` and is held, as a test run holds the lock: through a second handle of the same file,
/// which conflicts with every other handle, in this process too.
fn held(dir: &Path, text: &str) -> (std::path::PathBuf, File) {
    let path = dir.join(".make.command.lck");
    std::fs::write(&path, text).unwrap();
    let holder = File::open(&path).unwrap();
    holder.lock().unwrap();
    (path, holder)
}

#[test]
fn a_missing_lock_file_is_held_by_nobody() {
    let dir = tempfile::tempdir().unwrap();

    let holder = holder_of(&dir.path().join("missing.lck"), Path::new(WORKTREE)).unwrap();

    assert_eq!(holder, LockHolder::Nobody);
}

#[test]
fn a_lock_file_whose_lock_is_free_is_held_by_nobody_whatever_it_says() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".make.command.lck");
    std::fs::write(&path, format!("label=gate\ndir={WORKTREE}\npid=1\n")).unwrap();

    let holder = holder_of(&path, Path::new(WORKTREE)).unwrap();

    assert_eq!(holder, LockHolder::Nobody);
}

#[test]
fn a_lock_held_by_a_run_started_in_the_worktree_is_the_worktrees() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _holder) = held(dir.path(), &format!("label=gate\ndir={WORKTREE}\npid=1\n"));

    assert_eq!(holder_of(&path, Path::new(WORKTREE)).unwrap(), LockHolder::Worktree);
}

#[test]
fn a_lock_held_by_a_run_started_below_the_worktree_is_the_worktrees() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _holder) = held(dir.path(), &format!("label=test\ndir={WORKTREE}/crates/agent\npid=1\n"));

    assert_eq!(holder_of(&path, Path::new(WORKTREE)).unwrap(), LockHolder::Worktree);
}

#[test]
fn a_lock_held_by_a_run_of_another_worktree_is_not_the_worktrees() {
    let dir = tempfile::tempdir().unwrap();
    let other = "/home/example/agents/worktrees/team-agents/task-20261125-000000-bbbb";
    let (path, _holder) = held(dir.path(), &format!("label=gate\ndir={other}\npid=1\n"));

    assert_eq!(holder_of(&path, Path::new(WORKTREE)).unwrap(), LockHolder::Elsewhere);
}

#[test]
fn a_directory_that_only_starts_with_the_worktrees_name_is_not_below_it() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _holder) = held(dir.path(), &format!("label=gate\ndir={WORKTREE}-2\npid=1\n"));

    assert_eq!(holder_of(&path, Path::new(WORKTREE)).unwrap(), LockHolder::Elsewhere);
}

#[test]
fn a_lock_held_by_a_run_that_says_no_directory_is_not_the_worktrees() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _holder) = held(dir.path(), "label=gate\npid=1\n");

    assert_eq!(holder_of(&path, Path::new(WORKTREE)).unwrap(), LockHolder::Elsewhere);
}

#[test]
fn a_held_lock_whose_holder_cannot_be_read_is_an_error_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    // A directory is locked like a file but cannot be read as the text of a holder.
    let holder = File::open(dir.path()).unwrap();
    holder.lock().unwrap();

    let error = holder_of(dir.path(), Path::new(WORKTREE));

    assert!(
        matches!(&error, Err(TestLockError::Read { path, .. }) if path == dir.path()),
        "{error:?}"
    );
}

#[test]
fn a_lock_file_that_cannot_be_opened_is_an_error_naming_the_file() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".make.command.lck");
    std::fs::write(&path, "label=gate\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

    let error = holder_of(&path, Path::new(WORKTREE));

    assert!(
        matches!(&error, Err(TestLockError::Open { path: named, .. }) if *named == path),
        "{error:?}"
    );
}
