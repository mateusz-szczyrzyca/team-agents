//! What the daemon asks git when it publishes a branch and when it starts the next item after a merge: whether a
//! branch is part of another, what the remote has of a branch, and moving a reviewer's worktree to a commit.

#[cfg(test)]
mod git_publish {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::BranchName;
    use agent_core::state::CommitId;
    use agent_runtime::git::{CommitIdentity, Git, GitError, GitOperation, Revision};

    /// Runs real git for the setup of a test and returns its trimmed output.
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

    /// Commits a file `name` with `text` in `repo` and returns the new commit.
    fn commit_file(repo: &Path, name: &str, text: &str) -> CommitId {
        std::fs::write(repo.join(name), text).unwrap();
        raw(repo, &["add", name]);
        raw(repo, &["commit", "-q", "-m", name]);
        commit(&raw(repo, &["rev-parse", "HEAD"]))
    }

    /// A repository `repo` with one commit on `main` and a bare `origin` that has it; the temporary directory is
    /// returned to keep both alive.
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

    #[test]
    fn a_remote_branch_is_spelled_as_its_tracking_reference() {
        assert_eq!(
            Revision::RemoteBranch(branch("main")).to_string(),
            "refs/remotes/origin/main"
        );
    }

    #[tokio::test]
    async fn a_commit_is_part_of_the_branch_that_was_made_from_it() {
        let (_tmp, repo, _origin) = repository_with_origin();
        let first = commit(&raw(&repo, &["rev-parse", "HEAD"]));
        raw(&repo, &["switch", "-q", "-c", "work"]);
        commit_file(&repo, "b.txt", "two\n");

        let part = handle(&repo)
            .is_ancestor(&Revision::Commit(first), &Revision::Branch(branch("work")))
            .await
            .unwrap();

        assert!(part);
    }

    #[tokio::test]
    async fn a_commit_is_part_of_itself() {
        let (_tmp, repo, _origin) = repository_with_origin();
        let head = commit(&raw(&repo, &["rev-parse", "HEAD"]));

        let part = handle(&repo)
            .is_ancestor(&Revision::Commit(head), &Revision::Head)
            .await
            .unwrap();

        assert!(part);
    }

    #[tokio::test]
    async fn a_branch_with_commits_of_its_own_is_not_part_of_the_one_it_came_from() {
        let (_tmp, repo, _origin) = repository_with_origin();
        raw(&repo, &["switch", "-q", "-c", "work"]);
        commit_file(&repo, "b.txt", "two\n");

        let part = handle(&repo)
            .is_ancestor(&Revision::Branch(branch("work")), &Revision::Branch(branch("main")))
            .await
            .unwrap();

        assert!(!part);
    }

    #[tokio::test]
    async fn two_diverged_branches_are_not_part_of_each_other() {
        let (_tmp, repo, _origin) = repository_with_origin();
        raw(&repo, &["switch", "-q", "-c", "left"]);
        commit_file(&repo, "l.txt", "left\n");
        raw(&repo, &["switch", "-q", "main"]);
        raw(&repo, &["switch", "-q", "-c", "right"]);
        commit_file(&repo, "r.txt", "right\n");
        let git = handle(&repo);

        let left_in_right = git
            .is_ancestor(&Revision::Branch(branch("left")), &Revision::Branch(branch("right")))
            .await
            .unwrap();
        let right_in_left = git
            .is_ancestor(&Revision::Branch(branch("right")), &Revision::Branch(branch("left")))
            .await
            .unwrap();

        assert!(!left_in_right);
        assert!(!right_in_left);
    }

    #[tokio::test]
    async fn a_revision_that_names_nothing_is_an_error_not_an_answer() {
        let (_tmp, repo, _origin) = repository_with_origin();

        let error = handle(&repo)
            .is_ancestor(&Revision::Branch(branch("nothing")), &Revision::Head)
            .await
            .unwrap_err();

        assert!(
            matches!(
                &error,
                GitError::Failed {
                    operation: GitOperation::MergeBase,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_fetch_brings_the_remote_tip_of_a_branch_and_leaves_the_local_branch() {
        let (tmp, repo, origin) = repository_with_origin();
        let other = tmp.path().canonicalize().unwrap().join("other");
        raw(
            tmp.path(),
            &["clone", "-q", origin.to_str().unwrap(), other.to_str().unwrap()],
        );
        let advanced = commit_file(&other, "b.txt", "two\n");
        raw(&other, &["push", "-q", "origin", "main"]);
        let local_before = raw(&repo, &["rev-parse", "main"]);

        handle(&repo).fetch(&branch("main")).await.unwrap();

        let remote_tip = handle(&repo)
            .rev_parse(&Revision::RemoteBranch(branch("main")))
            .await
            .unwrap();
        assert_eq!(remote_tip, advanced);
        assert_eq!(raw(&repo, &["rev-parse", "main"]), local_before);
    }

    #[tokio::test]
    async fn a_fetch_of_a_branch_the_remote_does_not_have_fails_naming_the_operation() {
        let (_tmp, repo, _origin) = repository_with_origin();

        let error = handle(&repo).fetch(&branch("nothing")).await.unwrap_err();

        assert!(
            matches!(
                &error,
                GitError::Failed {
                    operation: GitOperation::Fetch,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_fetch_in_a_repository_without_an_origin_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir(&repo).unwrap();
        raw(&repo, &["init", "-q", "-b", "main"]);
        commit_file(&repo, "a.txt", "one\n");

        let error = handle(&repo).fetch(&branch("main")).await.unwrap_err();

        assert!(
            matches!(
                &error,
                GitError::Failed {
                    operation: GitOperation::Fetch,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_worktree_is_moved_to_a_commit_and_leaves_its_branch_where_it_was() {
        let (tmp, repo, _origin) = repository_with_origin();
        let first = commit(&raw(&repo, &["rev-parse", "HEAD"]));
        let second = commit_file(&repo, "b.txt", "two\n");
        let linked = tmp.path().canonicalize().unwrap().join("linked");
        raw(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "review",
                linked.to_str().unwrap(),
                &first.to_string(),
            ],
        );

        handle(&linked).detach(&second).await.unwrap();

        assert_eq!(raw(&linked, &["rev-parse", "HEAD"]), second.to_string());
        assert_eq!(raw(&repo, &["rev-parse", "review"]), first.to_string());
    }

    #[tokio::test]
    async fn moving_a_worktree_discards_the_changes_a_reviewer_left_in_it() {
        let (_tmp, repo, _origin) = repository_with_origin();
        let head = commit(&raw(&repo, &["rev-parse", "HEAD"]));
        let second = commit_file(&repo, "a.txt", "two\n");
        std::fs::write(repo.join("a.txt"), "scribbles\n").unwrap();

        handle(&repo).detach(&second).await.unwrap();

        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "two\n");
        assert_ne!(head, second);
    }

    #[tokio::test]
    async fn moving_a_worktree_to_a_commit_the_repository_does_not_have_fails() {
        let (_tmp, repo, _origin) = repository_with_origin();

        let error = handle(&repo)
            .detach(&commit("0123456789abcdef0123456789abcdef01234567"))
            .await
            .unwrap_err();

        assert!(
            matches!(
                &error,
                GitError::Failed {
                    operation: GitOperation::SwitchDetach,
                    ..
                }
            ),
            "{error:?}"
        );
    }
}
