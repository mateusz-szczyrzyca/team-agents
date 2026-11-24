//! The id of the tree a worktree holds: it tells whether the uncommitted work of a worktree changed between two looks.
//! Read from real repositories in temporary directories, isolated from the configuration of the machine.

#[cfg(test)]
mod git_worktree_tree {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_runtime::git::{CommitIdentity, Git};

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
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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

    /// A repository with one commit on `main` in a temporary directory.
    fn repository() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir(&repo).unwrap();
        raw(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), b"one\n").unwrap();
        raw(&repo, &["add", "-A"]);
        raw(&repo, &["commit", "-q", "-m", "start"]);
        (tmp, repo)
    }

    // Rule: the id is the id of a git tree, and a worktree that holds nothing new has the tree of its head.
    #[tokio::test]
    async fn a_clean_worktree_has_the_tree_of_its_head() {
        let (_tmp, repo) = repository();

        let tree = handle(&repo).worktree_tree().await.unwrap();

        assert_eq!(tree, raw(&repo, &["rev-parse", "HEAD^{tree}"]));
    }

    // Rule: reading it twice from the same files gives the same id, and the repository is not changed by reading.
    #[tokio::test]
    async fn the_same_uncommitted_files_give_the_same_tree() {
        let (_tmp, repo) = repository();
        std::fs::write(repo.join("draft.txt"), b"draft\n").unwrap();
        let git = handle(&repo);

        let first = git.worktree_tree().await.unwrap();
        let second = git.worktree_tree().await.unwrap();

        assert_eq!(first, second);
        assert_ne!(
            first,
            raw(&repo, &["rev-parse", "HEAD^{tree}"]),
            "the new file is in the tree"
        );
        assert_eq!(
            raw(&repo, &["status", "--porcelain"]),
            "?? draft.txt",
            "nothing was staged"
        );
    }

    // Rule: new bytes at an unchanged path give another tree: a changed draft is not the draft that was told of.
    #[tokio::test]
    async fn changed_bytes_at_the_same_path_give_another_tree() {
        let (_tmp, repo) = repository();
        std::fs::write(repo.join("draft.txt"), b"draft\n").unwrap();
        let git = handle(&repo);
        let before = git.worktree_tree().await.unwrap();

        std::fs::write(repo.join("draft.txt"), b"draft, changed\n").unwrap();
        let after = git.worktree_tree().await.unwrap();

        assert_ne!(before, after);
    }
}
