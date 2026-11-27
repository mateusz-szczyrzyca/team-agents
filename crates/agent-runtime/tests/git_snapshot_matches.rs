//! Whether a worktree still holds what a snapshot kept: the same tree on the same head. Read from real repositories in
//! temporary directories.

#[cfg(test)]
mod git_snapshot_matches {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::{RoundId, TaskId};
    use agent_runtime::git::{CommitIdentity, Git};

    /// The round of the snapshots of these tests.
    const ROUND: &str = "round-20261120-100000-r1r1";
    /// The task of the snapshots of these tests.
    const TASK: &str = "task-20261120-100000-t1t1";

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
        let path = AbsolutePath::from_str(dir.to_str().expect("a UTF-8 path")).expect("an absolute path");
        Git::new(
            path,
            CommitIdentity::new("t".to_owned(), "t@example.com".to_owned(), None),
        )
    }

    /// A repository on `main` with one commit and one uncommitted file, in a temporary directory.
    fn repository() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let repo = tmp.path().canonicalize().expect("a canonical path").join("repo");
        std::fs::create_dir(&repo).expect("the repository directory");
        raw(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a\n").expect("a file");
        raw(&repo, &["add", "-A"]);
        raw(&repo, &["commit", "-q", "-m", "base"]);
        std::fs::write(repo.join("new.txt"), "new\n").expect("an uncommitted file");
        (tmp, repo)
    }

    /// Takes a snapshot of the worktree at `repo` and returns the reference that keeps it.
    async fn snapshot(repo: &Path) -> String {
        let round = RoundId::from_str(ROUND).expect("a round id");
        let task = TaskId::from_str(TASK).expect("a task id");
        handle(repo)
            .snapshot_unless_unchanged(&round, &task, None)
            .await
            .expect("a snapshot")
            .reference
    }

    /// The snapshot references git holds.
    fn references(repo: &Path) -> Vec<String> {
        raw(repo, &["for-each-ref", "--format=%(refname)", "refs/agents/snapshots/"])
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[tokio::test]
    async fn a_worktree_that_was_not_changed_since_its_snapshot_matches_it_and_writes_no_reference() {
        let (_tmp, repo) = repository();
        let reference = snapshot(&repo).await;
        let before = references(&repo);

        let matches = handle(&repo)
            .worktree_matches_snapshot(&reference)
            .await
            .expect("a check");

        assert!(matches);
        assert_eq!(references(&repo), before);
    }

    #[tokio::test]
    async fn a_file_changed_since_the_snapshot_is_no_match() {
        let (_tmp, repo) = repository();
        let reference = snapshot(&repo).await;
        std::fs::write(repo.join("new.txt"), "changed\n").expect("a changed file");

        let matches = handle(&repo)
            .worktree_matches_snapshot(&reference)
            .await
            .expect("a check");

        assert!(!matches);
    }

    #[tokio::test]
    async fn a_new_commit_with_the_same_tree_is_no_match_because_the_head_moved() {
        let (_tmp, repo) = repository();
        let reference = snapshot(&repo).await;
        raw(&repo, &["add", "-A"]);
        raw(&repo, &["commit", "-q", "-m", "commit what the snapshot kept"]);

        let matches = handle(&repo)
            .worktree_matches_snapshot(&reference)
            .await
            .expect("a check");

        assert!(!matches);
    }

    #[tokio::test]
    async fn a_reference_that_names_nothing_is_no_match() {
        let (_tmp, repo) = repository();

        let matches = handle(&repo)
            .worktree_matches_snapshot("refs/agents/snapshots/round-20261120-100000-r1r1/task-20261120-100000-t1t1/c9")
            .await
            .expect("a check");

        assert!(!matches);
    }
}
