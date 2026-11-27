//! The commits and the changed files of a worktree since it was made, read from a real repository in a temporary
//! directory, isolated from the configuration of the machine.

#[cfg(test)]
mod work_since {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::TaskId;
    use agent_core::state::CommitId;
    use agent_runtime::git::GitError;
    use agent_runtime::snapshot::{WorkSpan, work_since};

    /// Runs real git for the setup of a test and returns its output without the last line break.
    fn raw(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Author")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Committer")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap().trim_end().to_owned()
    }

    fn repository() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir(&repo).unwrap();
        raw(&repo, &["init", "-q", "-b", "main"]);
        raw(&repo, &["commit", "-q", "--allow-empty", "-m", "base"]);
        (tmp, repo)
    }

    fn head(repo: &Path) -> CommitId {
        raw(repo, &["rev-parse", "HEAD"]).parse().unwrap()
    }

    fn commit_file(repo: &Path, name: &str, message: &str) {
        std::fs::write(repo.join(name), name).unwrap();
        raw(repo, &["add", name]);
        raw(repo, &["commit", "-q", "-m", message]);
    }

    fn task() -> TaskId {
        "task-20261127-040000-abcd".parse().unwrap()
    }

    // Rule: the commits after the start come oldest first with their messages, and the files they changed are listed.
    #[tokio::test]
    async fn the_commits_and_the_changed_files_since_the_start_are_read() {
        let (_tmp, repo) = repository();
        let start = head(&repo);
        commit_file(&repo, "a.txt", "One");
        commit_file(&repo, "b.txt", "Two");
        let tip = head(&repo);
        let worktree = AbsolutePath::from_str(repo.to_str().unwrap()).unwrap();

        let found = work_since(
            &worktree,
            &task(),
            &WorkSpan {
                start: &start,
                head: &tip,
                tree: &tip,
            },
        )
        .await
        .unwrap();

        let messages: Vec<&str> = found.commits.iter().map(|commit| commit.message.as_str()).collect();
        assert_eq!(messages, ["One", "Two"]);
        assert_eq!(found.changed, ["a.txt", "b.txt"]);
    }

    // Rule: a file that only the tree of the snapshot holds (it was never committed) is a changed file; its commit is not.
    #[tokio::test]
    async fn a_file_only_the_tree_holds_is_changed_but_makes_no_commit() {
        let (_tmp, repo) = repository();
        let start = head(&repo);
        commit_file(&repo, "a.txt", "One");
        let tip = head(&repo);
        commit_file(&repo, "notes.md", "Snapshot");
        let tree = head(&repo);
        let worktree = AbsolutePath::from_str(repo.to_str().unwrap()).unwrap();

        let found = work_since(
            &worktree,
            &task(),
            &WorkSpan {
                start: &start,
                head: &tip,
                tree: &tree,
            },
        )
        .await
        .unwrap();

        assert_eq!(found.commits.len(), 1);
        assert_eq!(found.changed, ["a.txt", "notes.md"]);
    }

    // Rule: a worktree that did nothing since its start has no commit and no changed file.
    #[tokio::test]
    async fn a_worktree_that_did_nothing_has_no_commit_and_no_changed_file() {
        let (_tmp, repo) = repository();
        let start = head(&repo);
        let worktree = AbsolutePath::from_str(repo.to_str().unwrap()).unwrap();

        let found = work_since(
            &worktree,
            &task(),
            &WorkSpan {
                start: &start,
                head: &start,
                tree: &start,
            },
        )
        .await
        .unwrap();

        assert_eq!((found.commits.len(), found.changed.len()), (0, 0));
    }

    // Rule: a start the repository does not have is an error that names the task.
    #[tokio::test]
    async fn a_start_the_repository_does_not_have_is_refused_with_the_task() {
        let (_tmp, repo) = repository();
        let tip = head(&repo);
        let missing: CommitId = "b".repeat(40).parse().unwrap();
        let worktree = AbsolutePath::from_str(repo.to_str().unwrap()).unwrap();

        let error = work_since(
            &worktree,
            &task(),
            &WorkSpan {
                start: &missing,
                head: &tip,
                tree: &tip,
            },
        )
        .await
        .unwrap_err();

        assert_eq!(error.task, task());
        assert!(matches!(error.source, GitError::Failed { .. }), "{error:?}");
    }
}
