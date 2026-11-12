//! The file that keeps paths out of git for a repository and all its worktrees.

#[cfg(test)]
mod git_exclude {
    use std::path::Path;
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_runtime::git::{CommitIdentity, Git, GitError, GitOperation};

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn handle(dir: &Path) -> Git {
        let dir = AbsolutePath::from_str(dir.to_str().unwrap()).unwrap();
        Git::new(
            dir,
            CommitIdentity::new("t".to_owned(), "t@example.com".to_owned(), None),
        )
    }

    #[tokio::test]
    async fn the_exclude_file_of_a_repository_is_in_its_git_directory() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);

        let file = handle(dir.path()).info_exclude_file().await.unwrap();

        assert_eq!(file, dir.path().join(".git/info/exclude"));
    }

    #[tokio::test]
    async fn a_linked_worktree_shares_the_exclude_file_of_its_repository() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
        let linked = dir.path().join("linked");
        git(
            &repo,
            &["worktree", "add", "-q", "-b", "other", linked.to_str().unwrap()],
        );

        let file = handle(&linked).info_exclude_file().await.unwrap();

        assert_eq!(
            file.canonicalize().unwrap(),
            repo.join(".git/info/exclude").canonicalize().unwrap()
        );
    }

    #[tokio::test]
    async fn a_directory_that_is_no_repository_has_no_exclude_file() {
        let dir = tempfile::tempdir().unwrap();

        let error = handle(dir.path()).info_exclude_file().await.unwrap_err();

        assert!(
            matches!(
                &error,
                GitError::Failed {
                    operation: GitOperation::GitPath,
                    ..
                }
            ),
            "{error:?}"
        );
    }
}
