//! The snapshots of a checkpoint are numbered apart from the snapshots of a cut run: the briefing of a new session and the
//! history's snapshot table read those as `1`, `2`, ..., and a checkpoint taken before them must not take their numbers.
//! Read from real repositories in temporary directories.

#[cfg(test)]
mod git_checkpoint_numbers {
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
    async fn a_checkpoint_snapshot_does_not_take_the_number_of_the_snapshot_of_a_cut_run() {
        let (_tmp, repo) = repository();
        let checkpoint = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), None)
            .await
            .expect("a checkpoint snapshot");
        std::fs::write(repo.join("new.txt"), "new\n").expect("an uncommitted file");

        let cut = handle(&repo)
            .snapshot(&round(), &task())
            .await
            .expect("the snapshot of a cut run");

        assert_eq!(cut.number, 1);
        assert_eq!(checkpoint.snapshot.number, 1);
        assert_eq!(
            references(&repo),
            vec![
                format!("refs/agents/snapshots/{ROUND}/{TASK}/1"),
                format!("refs/agents/snapshots/{ROUND}/{TASK}/c1"),
            ]
        );
        assert_eq!(checkpoint.reference, format!("refs/agents/snapshots/{ROUND}/{TASK}/c1"));
    }

    #[tokio::test]
    async fn the_snapshot_of_a_cut_run_does_not_take_the_number_of_a_checkpoint_snapshot() {
        let (_tmp, repo) = repository();
        handle(&repo)
            .snapshot(&round(), &task())
            .await
            .expect("the snapshot of a cut run");
        std::fs::write(repo.join("new.txt"), "new\n").expect("an uncommitted file");

        let checkpoint = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), None)
            .await
            .expect("a checkpoint snapshot");
        let second_cut = handle(&repo)
            .snapshot(&round(), &task())
            .await
            .expect("a second snapshot");

        assert_eq!(checkpoint.snapshot.number, 1);
        assert_eq!(second_cut.number, 2);
    }

    #[tokio::test]
    async fn checkpoint_snapshots_are_counted_among_themselves_and_reused_by_their_reference() {
        let (_tmp, repo) = repository();
        let first = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), None)
            .await
            .expect("a first checkpoint snapshot");
        std::fs::write(repo.join("new.txt"), "new\n").expect("an uncommitted file");
        let second = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), Some(&first.reference))
            .await
            .expect("a second checkpoint snapshot");
        let third = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), Some(&second.reference))
            .await
            .expect("a third checkpoint snapshot");

        assert_eq!(first.reference, format!("refs/agents/snapshots/{ROUND}/{TASK}/c1"));
        assert_eq!(second.reference, format!("refs/agents/snapshots/{ROUND}/{TASK}/c2"));
        assert_eq!(
            (third.origin, third.reference),
            (SnapshotOrigin::Reused, second.reference)
        );
    }

    #[tokio::test]
    async fn the_reference_of_a_cut_run_is_not_reused_for_a_checkpoint() {
        let (_tmp, repo) = repository();
        let cut = handle(&repo)
            .snapshot(&round(), &task())
            .await
            .expect("the snapshot of a cut run");
        let cut_reference = format!("refs/agents/snapshots/{ROUND}/{TASK}/{}", cut.number);

        let checkpoint = handle(&repo)
            .snapshot_unless_unchanged(&round(), &task(), Some(&cut_reference))
            .await
            .expect("a checkpoint snapshot");

        assert_eq!(checkpoint.origin, SnapshotOrigin::Taken);
        assert_eq!(checkpoint.reference, format!("refs/agents/snapshots/{ROUND}/{TASK}/c1"));
    }
}
