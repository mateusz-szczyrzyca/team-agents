//! The rollback of [`Git::move_branch`] in the schedules a requester that works in its worktree can make: the unit
//! tests reach the private steps of the write, which no caller can interleave with a commit of the requester's own.

#![cfg(test)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use agent_core::config::AbsolutePath;

use super::*;
use crate::git::CommitIdentity;

/// The round and the task of the merges of these tests.
const ROUND: &str = "round-20261122-140000-c1c1";
/// See [`ROUND`].
const TASK: &str = "task-20261122-140000-c1c1";

/// Runs real git for the setup of a test and returns its output without the last line break.
fn raw(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture Author")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "Fixture Author")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git prints text")
        .trim_end_matches('\n')
        .to_owned()
}

fn handle(dir: &Path) -> Git {
    let dir = AbsolutePath::from_str(dir.to_str().expect("a UTF-8 path")).expect("an absolute path");
    Git::new(
        dir,
        CommitIdentity::new("t".to_owned(), "t@example.com".to_owned(), None),
    )
}

fn head_of(repo: &Path) -> CommitId {
    raw(repo, &["rev-parse", "HEAD"]).parse().expect("a commit")
}

/// A commit on the current branch that changes `files`.
fn commit(repo: &Path, files: &[(&str, &[u8])], message: &str) {
    for (name, bytes) in files {
        std::fs::write(repo.join(name), bytes).expect("a file");
    }
    raw(repo, &["add", "-A"]);
    raw(repo, &["commit", "-q", "-m", message]);
}

/// A repository on `main` with `guide.md` as the requester has it, and the worker's commit that adds a line to it,
/// made on a branch of its own; the merge of that commit is written to the worktree, and the branch is not moved yet.
async fn merge_written_to_the_worktree() -> (tempfile::TempDir, PathBuf, Git, CommitId, CommitId) {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let repo = tmp.path().canonicalize().expect("a canonical path").join("repo");
    std::fs::create_dir(&repo).expect("the repository directory");
    raw(&repo, &["init", "-q", "-b", "main"]);
    commit(&repo, &[("guide.md", b"one\n")], "base");
    raw(&repo, &["switch", "-q", "-c", "worker"]);
    commit(&repo, &[("guide.md", b"one\ntwo\n")], "the worker's step");
    let worker = head_of(&repo);
    raw(&repo, &["switch", "-q", "main"]);
    let git = handle(&repo);
    let head = head_of(&repo);
    let (round, task) = (
        RoundId::from_str(ROUND).expect("a round"),
        TaskId::from_str(TASK).expect("a task"),
    );
    let Merged::Clean(applied) = git
        .merge_into_scratch(&head, &worker, &round, &task)
        .await
        .expect("the merge")
    else {
        panic!("the merge applies");
    };
    git.read_tree_update(&head.to_string(), &applied.to_string())
        .await
        .expect("the files of the merge are written");
    (tmp, repo, git, head, applied)
}

#[tokio::test]
async fn a_requester_that_commits_the_staged_merge_before_the_branch_moves_keeps_the_merge() {
    let (_tmp, repo, git, head, applied) = merge_written_to_the_worktree().await;
    // The requester commits what is staged: the merge, in a commit of its own.
    raw(&repo, &["commit", "-q", "-m", "the requester commits what is staged"]);
    let moved = head_of(&repo);

    let error = git
        .move_branch(&head, &applied)
        .await
        .expect_err("the branch is not where the merge was made");

    assert!(
        matches!(&error, GitError::HeadMoved { expected, found } if *expected == head && *found == moved),
        "{error:?}"
    );
    assert_eq!(head_of(&repo), moved);
    assert_eq!(std::fs::read(repo.join("guide.md")).expect("the file"), b"one\ntwo\n");
    assert_eq!(raw(&repo, &["status", "--porcelain=v1", "-z"]), "");
}

#[tokio::test]
async fn a_branch_that_cannot_be_moved_puts_the_files_back() {
    let (_tmp, repo, git, head, applied) = merge_written_to_the_worktree().await;
    // A hook of the repository refuses the update of the reference.
    let hook = repo.join(".git").join("hooks").join("reference-transaction");
    std::fs::create_dir_all(hook.parent().expect("a directory")).expect("the hooks directory");
    std::fs::write(&hook, "#!/bin/sh\n[ \"$1\" = prepared ] && exit 1\nexit 0\n").expect("the hook");
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).expect("the mode");

    let error = git
        .move_branch(&head, &applied)
        .await
        .expect_err("the reference cannot be updated");

    assert!(
        matches!(
            &error,
            GitError::Failed {
                operation: GitOperation::UpdateRef,
                ..
            }
        ),
        "{error:?}"
    );
    assert_eq!(head_of(&repo), head);
    assert_eq!(std::fs::read(repo.join("guide.md")).expect("the file"), b"one\n");
    assert_eq!(raw(&repo, &["status", "--porcelain=v1", "-z"]), "");
}
