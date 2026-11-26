//! A snapshot for a checkpoint stands in for the one before it when the worktree holds the same tree on the same head, and
//! the references of a round's snapshots can be removed. Read from real repositories in temporary directories.

#[cfg(test)]
mod git_snapshot_reuse {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::{RoundId, TaskId};
    use agent_runtime::git::{CommitIdentity, Git, SnapshotOrigin};

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

    fn round() -> RoundId {
        RoundId::from_str(ROUND).expect("a round id")
    }

    fn task() -> TaskId {
        TaskId::from_str(TASK).expect("a task id")
    }

    /// A repository on `main` with one commit, in a temporary directory.
    fn repository() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let repo = tmp.path().canonicalize().expect("a canonical path").join("repo");
        std::fs::create_dir(&repo).expect("the repository directory");
        raw(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "a\n").expect("a file");
        raw(&repo, &["add", "-A"]);
        raw(&repo, &["commit", "-q", "-m", "base"]);
        (tmp, repo)
    }

    /// The snapshot references git holds.
    fn references(repo: &Path) -> Vec<String> {
        raw(repo, &["for-each-ref", "--format=%(refname)", "refs/agents/snapshots/"])
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[tokio::test]
    async fn a_second_snapshot_of_the_same_tree_and_head_reuses_the_first_reference() {
        let (_tmp, repo) = repository();
        std::fs::write(repo.join("new.txt"), "new\n").expect("an uncommitted file");
        let first = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), None)
            .await
            .expect("a first snapshot");

        let second = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), Some(&first.reference))
            .await
            .expect("a second snapshot");

        assert_eq!(first.origin, SnapshotOrigin::Taken);
        assert_eq!(second.origin, SnapshotOrigin::Reused);
        assert_eq!(
            second,
            agent_runtime::git::CheckpointSnapshot {
                origin: SnapshotOrigin::Reused,
                ..first
            }
        );
        assert_eq!(references(&repo).len(), 1);
    }

    #[tokio::test]
    async fn a_changed_uncommitted_file_makes_a_new_snapshot() {
        let (_tmp, repo) = repository();
        std::fs::write(repo.join("new.txt"), "new\n").expect("an uncommitted file");
        let first = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), None)
            .await
            .expect("a first snapshot");
        std::fs::write(repo.join("new.txt"), "changed\n").expect("a changed file");

        let second = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), Some(&first.reference))
            .await
            .expect("a second snapshot");

        assert_eq!(second.origin, SnapshotOrigin::Taken);
        assert_ne!(second.reference, first.reference);
        assert_eq!(references(&repo).len(), 2);
    }

    #[tokio::test]
    async fn a_new_commit_with_the_same_tree_makes_a_new_snapshot() {
        let (_tmp, repo) = repository();
        std::fs::write(repo.join("new.txt"), "new\n").expect("an uncommitted file");
        let first = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), None)
            .await
            .expect("a first snapshot");
        raw(&repo, &["add", "-A"]);
        raw(&repo, &["commit", "-q", "-m", "the file, committed"]);

        let second = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), Some(&first.reference))
            .await
            .expect("a second snapshot");

        assert_eq!(second.origin, SnapshotOrigin::Taken);
        assert_eq!(second.snapshot.head.to_string(), raw(&repo, &["rev-parse", "HEAD"]));
    }

    #[tokio::test]
    async fn a_previous_reference_that_is_gone_makes_a_new_snapshot() {
        let (_tmp, repo) = repository();
        let missing = format!("refs/agents/snapshots/{ROUND}/{TASK}/7");

        let taken = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), Some(&missing))
            .await
            .expect("a snapshot");

        assert_eq!(taken.origin, SnapshotOrigin::Taken);
    }

    #[tokio::test]
    async fn deleting_snapshot_references_removes_them_and_a_missing_one_is_no_error() {
        let (_tmp, repo) = repository();
        let first = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), None)
            .await
            .expect("a snapshot");
        std::fs::write(repo.join("new.txt"), "new\n").expect("an uncommitted file");
        let second = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), Some(&first.reference))
            .await
            .expect("a snapshot");
        let missing = format!("refs/agents/snapshots/{ROUND}/{TASK}/99");

        handle(&repo)
            .delete_snapshot_references(&[first.reference, second.reference, missing])
            .await
            .expect("the references are removed");

        assert_eq!(references(&repo), Vec::<String>::new());
    }
}
