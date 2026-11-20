//! The snapshot a task's worktree gets when its run was cut: the record the history keeps, and the warning for a snapshot
//! above the configured size. Read from real repositories in temporary directories.

#[path = "support/scopes.rs"]
mod scopes;

#[cfg(test)]
mod snapshot_take {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::{RoundId, TaskId};
    use agent_core::state::Timestamp;
    use agent_runtime::snapshot::{SnapshotError, take_snapshot};
    use tracing_subscriber::layer::SubscriberExt;

    use crate::scopes::ScopeRecorder;

    const ROUND: &str = "round-20261120-100000-r1r1";
    const TASK: &str = "task-20261120-100000-t1t1";

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
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("text").trim_end().to_owned()
    }

    fn repository() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let repo = tmp.path().canonicalize().expect("a canonical path").join("repo");
        std::fs::create_dir(&repo).expect("the directory");
        raw(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("file.txt"), b"base\n").expect("a file");
        raw(&repo, &["add", "-A"]);
        raw(&repo, &["commit", "-q", "-m", "base"]);
        (tmp, repo)
    }

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().expect("a UTF-8 path")).expect("an absolute path")
    }

    fn round() -> RoundId {
        RoundId::from_str(ROUND).expect("a round ID")
    }

    fn task() -> TaskId {
        TaskId::from_str(TASK).expect("a task ID")
    }

    fn at() -> Timestamp {
        Timestamp::from_utc(time::UtcDateTime::from_unix_timestamp(1_700_000_000).expect("a time"))
            .expect("a timestamp")
    }

    #[tokio::test]
    async fn a_snapshot_is_recorded_with_its_round_task_number_head_and_size() {
        let (_tmp, repo) = repository();
        std::fs::write(repo.join("file.txt"), b"edited\n").expect("the edit");
        let head = raw(&repo, &["rev-parse", "HEAD"]);

        let record = take_snapshot(&abs(&repo), &round(), &task(), at(), 200 * 1024 * 1024)
            .await
            .expect("a snapshot");

        assert_eq!(record.round, round());
        assert_eq!(record.task, task());
        assert_eq!(record.number, 1);
        assert_eq!(record.head.to_string(), head);
        assert!(record.bytes > 0, "{record:?}");
        assert_eq!(record.at, at());
        let reference = format!("refs/agents/snapshots/{ROUND}/{TASK}/1");
        assert_eq!(raw(&repo, &["rev-parse", &reference]), record.commit.to_string());
    }

    #[tokio::test]
    async fn a_snapshot_above_the_limit_is_kept_and_logged_as_a_warning() {
        let (_tmp, repo) = repository();
        std::fs::write(repo.join("big.bin"), vec![7_u8; 64 * 1024]).expect("the file");
        let recorder = ScopeRecorder::default();
        let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(recorder.clone()));

        let record = tracing::instrument::WithSubscriber::with_subscriber(
            take_snapshot(&abs(&repo), &round(), &task(), at(), 1024),
            dispatch,
        )
        .await
        .expect("a snapshot");

        assert!(record.bytes >= 64 * 1024, "{record:?}");
        let events = recorder.events();
        assert!(
            events.iter().any(|event| event.name == "snapshot_large"),
            "no warning about the size: {events:?}"
        );
    }

    #[tokio::test]
    async fn a_snapshot_below_the_limit_is_not_logged_as_large() {
        let (_tmp, repo) = repository();
        std::fs::write(repo.join("small.txt"), b"small\n").expect("the file");
        let recorder = ScopeRecorder::default();
        let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(recorder.clone()));

        tracing::instrument::WithSubscriber::with_subscriber(
            take_snapshot(&abs(&repo), &round(), &task(), at(), 1024 * 1024),
            dispatch,
        )
        .await
        .expect("a snapshot");

        assert!(!recorder.events().iter().any(|event| event.name == "snapshot_large"));
    }

    #[tokio::test]
    async fn a_directory_that_is_no_repository_has_no_snapshot() {
        let tmp = tempfile::tempdir().expect("a temporary directory");

        let error = take_snapshot(&abs(tmp.path()), &round(), &task(), at(), 1024)
            .await
            .expect_err("no repository");

        assert!(matches!(error, SnapshotError::Git { .. }), "{error:?}");
    }
}
