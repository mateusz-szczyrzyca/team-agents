//! Tests for pushing a commit directly to a remote branch and updating unheld local branches.

#[cfg(test)]
mod git_publish_commit {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::BranchName;
    use agent_core::state::CommitId;
    use agent_runtime::git::{CommitIdentity, Git, GitError, GitOperation, Revision, WorktreeBranch};

    fn raw(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "protocol.file.allow=always"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture Author")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim_end().to_owned()
    }

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
    }

    fn handle(dir: &Path) -> Git {
        Git::new(
            abs(dir),
            CommitIdentity::new("t".to_owned(), "t@example.com".to_owned(), None),
        )
    }

    fn branch(name: &str) -> BranchName {
        name.parse().unwrap()
    }

    fn commit(text: &str) -> CommitId {
        text.parse().unwrap()
    }

    fn commit_file(repo: &Path, name: &str, text: &str) -> CommitId {
        std::fs::write(repo.join(name), text).unwrap();
        raw(repo, &["add", name]);
        raw(repo, &["commit", "-q", "-m", name]);
        commit(&raw(repo, &["rev-parse", "HEAD"]))
    }

    fn repository_with_origin() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let repo = root.join("repo");
        let origin = root.join("octo").join("example.git");
        std::fs::create_dir(&repo).unwrap();
        std::fs::create_dir_all(&origin).unwrap();
        raw(&origin, &["init", "-q", "--bare", "-b", "main"]);
        raw(&repo, &["init", "-q", "-b", "main"]);
        commit_file(&repo, "a.txt", "one\n");
        raw(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
        raw(&repo, &["push", "-q", "origin", "main"]);
        (tmp, repo, origin)
    }

    #[tokio::test]
    async fn push_commit_sends_arbitrary_commit_to_remote_branch() {
        let (_tmp, repo, origin) = repository_with_origin();
        let second = commit_file(&repo, "b.txt", "two\n");
        let target_branch = branch("agents/gemini-1/task-1-1.1a");

        handle(&repo).push_commit(&target_branch, &second).await.unwrap();

        let remote_head = raw(&origin, &["rev-parse", "refs/heads/agents/gemini-1/task-1-1.1a"]);
        assert_eq!(remote_head, second.to_string());
    }

    #[tokio::test]
    async fn held_branches_finds_branches_checked_out_in_worktrees() {
        let (tmp, repo, _origin) = repository_with_origin();
        let linked = tmp.path().canonicalize().unwrap().join("linked");
        let git = handle(&repo);
        git.worktree_add(
            &abs(&linked),
            &WorktreeBranch::New {
                branch: branch("worktree-branch"),
                start: Revision::Head,
            },
        )
        .await
        .unwrap();

        let held = git.held_branches().await.unwrap();
        assert!(held.contains(&branch("main")));
        assert!(held.contains(&branch("worktree-branch")));
        assert!(!held.contains(&branch("non-existent-branch")));
    }

    #[tokio::test]
    async fn branch_point_if_unheld_skips_held_branch_and_updates_unheld() {
        let (_tmp, repo, _origin) = repository_with_origin();
        let first = commit(&raw(&repo, &["rev-parse", "HEAD"]));
        let second = commit_file(&repo, "b.txt", "two\n");
        let unheld = branch("unheld-branch");
        raw(&repo, &["branch", unheld.as_str(), &first.to_string()]);

        let git = handle(&repo);
        // main is checked out in repo, so it is held:
        let moved_held = git.branch_point_if_unheld(&branch("main"), &second).await.unwrap();
        assert!(!moved_held);
        // main still points to second, which was committed on main:
        assert_eq!(raw(&repo, &["rev-parse", "main"]), second.to_string());

        // unheld is not checked out anywhere:
        let moved_unheld = git.branch_point_if_unheld(&unheld, &second).await.unwrap();
        assert!(moved_unheld);
        assert_eq!(raw(&repo, &["rev-parse", unheld.as_str()]), second.to_string());
    }

    #[tokio::test]
    async fn pushing_diverged_commit_is_refused_and_remote_branch_stays_unchanged() {
        let (_tmp, repo, origin) = repository_with_origin();
        let first = commit(&raw(&repo, &["rev-parse", "HEAD"]));
        let target_branch = branch("agents/gemini-1/task-1-1.1a");
        let git = handle(&repo);

        // Push initial commit to remote branch:
        git.push_commit(&target_branch, &first).await.unwrap();

        // Advance remote branch with an independent commit:
        let remote_third = commit_file(&repo, "c.txt", "remote three\n");
        raw(
            &repo,
            &[
                "push",
                "-q",
                "origin",
                &format!("{remote_third}:refs/heads/{target_branch}"),
            ],
        );
        assert_eq!(
            raw(&origin, &["rev-parse", &format!("refs/heads/{target_branch}")]),
            remote_third.to_string()
        );

        // Reset local repo back to first and create a diverged commit:
        raw(&repo, &["reset", "--hard", &first.to_string()]);
        let local_diverged = commit_file(&repo, "d.txt", "diverged local\n");

        // Attempting to push diverged commit directly to target_branch must be refused:
        let result = git.push_commit(&target_branch, &local_diverged).await;
        assert!(
            matches!(result, Err(agent_runtime::git::GitError::PushRefused { .. })),
            "expected PushRefused, got {result:?}"
        );

        // Remote branch on origin must stay at remote_third, completely unchanged:
        assert_eq!(
            raw(&origin, &["rev-parse", &format!("refs/heads/{target_branch}")]),
            remote_third.to_string()
        );
    }

    #[tokio::test]
    async fn held_branches_fails_with_not_a_branch_on_malformed_branch_name() {
        let (tmp, repo, _origin) = repository_with_origin();
        let linked = tmp.path().canonicalize().unwrap().join("linked");
        let head = raw(&repo, &["rev-parse", "HEAD"]);
        raw(&repo, &["update-ref", "refs/heads/-invalid", &head]);
        raw(&repo, &["worktree", "add", linked.to_str().unwrap(), &head]);
        raw(&linked, &["symbolic-ref", "HEAD", "refs/heads/-invalid"]);

        let git = handle(&repo);
        let error = git.held_branches().await.unwrap_err();
        assert!(
            matches!(
                error,
                GitError::NotABranch {
                    operation: GitOperation::WorktreeList,
                    ref source,
                } if source.value == "-invalid"
            ),
            "expected GitError::NotABranch, got {error:?}"
        );
    }

    #[tokio::test]
    async fn branch_point_if_unheld_propagates_not_a_branch_error() {
        let (tmp, repo, _origin) = repository_with_origin();
        let linked = tmp.path().canonicalize().unwrap().join("linked");
        let head = raw(&repo, &["rev-parse", "HEAD"]);
        let head_commit = commit(&head);
        raw(&repo, &["update-ref", "refs/heads/-invalid", &head]);
        raw(&repo, &["worktree", "add", linked.to_str().unwrap(), &head]);
        raw(&linked, &["symbolic-ref", "HEAD", "refs/heads/-invalid"]);

        let git = handle(&repo);
        let target = branch("main");
        let error = git.branch_point_if_unheld(&target, &head_commit).await.unwrap_err();
        assert!(
            matches!(
                error,
                GitError::NotABranch {
                    operation: GitOperation::WorktreeList,
                    ref source,
                } if source.value == "-invalid"
            ),
            "expected GitError::NotABranch, got {error:?}"
        );
    }
}
