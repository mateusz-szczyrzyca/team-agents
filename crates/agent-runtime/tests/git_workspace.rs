//! What the daemon asks git about the checkout a `run` comes from, and the branch it starts for a member's next item.

#[cfg(test)]
mod git_workspace {
    use std::path::Path;
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::BranchName;
    use agent_runtime::git::{CommitIdentity, Git, GitError, GitOperation, Revision};

    /// Runs real git for the setup of a test and returns its trimmed output.
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

    /// A repository with one commit on `main`; the temporary directory is returned to keep it alive.
    fn repository() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir(&repo).unwrap();
        raw(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        raw(&repo, &["add", "a.txt"]);
        raw(&repo, &["commit", "-q", "-m", "First"]);
        (tmp, repo)
    }

    #[tokio::test]
    async fn the_toplevel_is_the_root_of_the_checkout_from_any_directory_of_it() {
        let (_tmp, repo) = repository();
        std::fs::create_dir_all(repo.join("deep/er")).unwrap();

        let root = handle(&repo.join("deep/er")).toplevel().await.unwrap();

        assert_eq!(root.as_path(), repo);
    }

    #[tokio::test]
    async fn a_directory_in_no_repository_has_no_toplevel() {
        let tmp = tempfile::tempdir().unwrap();

        let error = handle(tmp.path()).toplevel().await.unwrap_err();

        assert!(
            matches!(
                &error,
                GitError::Failed {
                    operation: GitOperation::TopLevel,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn the_origin_url_is_what_the_origin_remote_says() {
        let (_tmp, repo) = repository();
        raw(
            &repo,
            &["remote", "add", "origin", "https://example.com/team/example-repo.git"],
        );

        let url = handle(&repo).origin_url().await.unwrap();

        assert_eq!(url, "https://example.com/team/example-repo.git");
    }

    #[tokio::test]
    async fn a_repository_without_an_origin_has_no_origin_url() {
        let (_tmp, repo) = repository();

        let error = handle(&repo).origin_url().await.unwrap_err();

        assert!(
            matches!(
                &error,
                GitError::Failed {
                    operation: GitOperation::OriginUrl,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_new_branch_starts_where_asked_and_is_checked_out_in_the_worktree() {
        let (tmp, repo) = repository();
        let first = raw(&repo, &["rev-parse", "HEAD"]);
        std::fs::write(repo.join("b.txt"), "two\n").unwrap();
        raw(&repo, &["add", "b.txt"]);
        raw(&repo, &["commit", "-q", "-m", "Second"]);
        let linked = tmp.path().canonicalize().unwrap().join("linked");
        raw(
            &repo,
            &["worktree", "add", "-q", "-b", "first-work", linked.to_str().unwrap()],
        );

        handle(&linked)
            .switch_new_branch(&branch("second-work"), &Revision::Branch(branch("main")))
            .await
            .unwrap();

        assert_eq!(raw(&linked, &["rev-parse", "--abbrev-ref", "HEAD"]), "second-work");
        assert_eq!(raw(&linked, &["rev-parse", "HEAD"]), raw(&repo, &["rev-parse", "main"]));
        assert_ne!(raw(&linked, &["rev-parse", "HEAD"]), first);
    }

    #[tokio::test]
    async fn a_branch_that_exists_is_not_made_again() {
        let (_tmp, repo) = repository();

        let error = handle(&repo)
            .switch_new_branch(&branch("main"), &Revision::Head)
            .await
            .unwrap_err();

        assert!(
            matches!(
                &error,
                GitError::Failed {
                    operation: GitOperation::Switch,
                    ..
                }
            ),
            "{error:?}"
        );
    }
}
