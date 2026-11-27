//! The whole messages of the commits of a branch, read from a real repository in a temporary directory, isolated from
//! the configuration of the machine: a checkpoint ties the commits to the steps their trailers name.

#[cfg(test)]
mod git_range_messages {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::state::CommitId;
    use agent_runtime::git::{CommitIdentity, CommitMessage, Git, GitError};

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
        String::from_utf8(output.stdout)
            .unwrap()
            .trim_end_matches('\n')
            .to_owned()
    }

    fn handle(dir: &Path) -> Git {
        Git::new(
            AbsolutePath::from_str(dir.to_str().unwrap()).unwrap(),
            CommitIdentity::new("t".to_owned(), "t@example.com".to_owned(), None),
        )
    }

    fn commit(repo: &Path, message: &str) -> CommitId {
        raw(repo, &["commit", "-q", "--allow-empty", "-m", message]);
        raw(repo, &["rev-parse", "HEAD"]).parse().unwrap()
    }

    fn repository() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir(&repo).unwrap();
        raw(&repo, &["init", "-q", "-b", "main"]);
        (tmp, repo)
    }

    // Rule: the messages of the commits after the base, oldest first, whole: the subject, the body and the trailer.
    #[tokio::test]
    async fn the_commits_after_the_base_come_oldest_first_with_their_whole_messages() {
        let (_tmp, repo) = repository();
        let base = commit(&repo, "base");
        let first = commit(&repo, "First — żółć\n\nWhy it is so.\n\nStep: 1.1");
        let second = commit(&repo, "Second\n\nStep: 1.2");

        let found = handle(&repo).range_messages(&base, &second).await.unwrap();

        assert_eq!(
            found,
            [
                CommitMessage {
                    id: first,
                    message: "First — żółć\n\nWhy it is so.\n\nStep: 1.1".to_owned(),
                },
                CommitMessage {
                    id: second,
                    message: "Second\n\nStep: 1.2".to_owned(),
                },
            ]
        );
    }

    // Rule: a range with no commit has no message.
    #[tokio::test]
    async fn an_empty_range_has_no_message() {
        let (_tmp, repo) = repository();
        let base = commit(&repo, "base");

        assert_eq!(handle(&repo).range_messages(&base, &base).await.unwrap(), []);
    }

    // Rule: a commit that is not in the repository is refused, not read as an empty range.
    #[tokio::test]
    async fn a_commit_that_is_not_in_the_repository_is_refused() {
        let (_tmp, repo) = repository();
        let base = commit(&repo, "base");
        let missing: CommitId = "a".repeat(40).parse().unwrap();

        let error = handle(&repo).range_messages(&base, &missing).await.unwrap_err();

        assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
    }
}
