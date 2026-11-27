//! The commits a task's branch has that its base does not, read from a real repository in a temporary directory,
//! isolated from the configuration of the machine.

#[cfg(test)]
mod branch_commits {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::{BranchName, TaskId};
    use agent_runtime::git::GitError;
    use agent_runtime::snapshot::branch_commits;

    /// Runs real git for the setup of a test and returns its output without the last line break.
    fn raw(dir: &Path, args: &[&str]) {
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
    }

    fn repository() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir(&repo).unwrap();
        raw(&repo, &["init", "-q", "-b", "main"]);
        raw(&repo, &["commit", "-q", "--allow-empty", "-m", "base"]);
        (tmp, repo)
    }

    fn task() -> TaskId {
        "task-20261127-040000-abcd".parse().unwrap()
    }

    // Rule: the commits made on the work branch after it left the base come oldest first with their whole messages.
    #[tokio::test]
    async fn the_commits_of_the_work_branch_past_the_base_come_with_their_messages() {
        let (_tmp, repo) = repository();
        raw(&repo, &["switch", "-q", "-c", "work"]);
        raw(
            &repo,
            &["commit", "-q", "--allow-empty", "-m", "One", "-m", "Step: 1.1"],
        );
        raw(&repo, &["commit", "-q", "--allow-empty", "-m", "Two"]);
        let worktree = AbsolutePath::from_str(repo.to_str().unwrap()).unwrap();

        let found = branch_commits(&worktree, &task(), &BranchName::from_str("main").unwrap())
            .await
            .unwrap();

        let messages: Vec<&str> = found.iter().map(|commit| commit.message.as_str()).collect();
        assert_eq!(messages, ["One\n\nStep: 1.1", "Two"]);
    }

    // Rule: a base the repository does not have is an error that names the task and the base.
    #[tokio::test]
    async fn a_base_the_repository_does_not_have_is_refused_with_the_task_and_the_base() {
        let (_tmp, repo) = repository();
        let worktree = AbsolutePath::from_str(repo.to_str().unwrap()).unwrap();
        let base = BranchName::from_str("nowhere").unwrap();

        let error = branch_commits(&worktree, &task(), &base).await.unwrap_err();

        assert_eq!(error.task, task());
        assert_eq!(error.base, base);
        assert!(matches!(error.source, GitError::Failed { .. }), "{error:?}");
    }
}
