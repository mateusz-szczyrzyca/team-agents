//! Taking a path out of git and keeping its file: what a plan committed by mistake needs.

#[cfg(test)]
mod git_untrack {
    use std::path::Path;
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::repo_path::RepoPath;
    use agent_runtime::git::{CommitIdentity, Git, GitError, Revision, Untracked};

    const PLAN: &str = ".agent/plan.md";

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn handle(dir: &Path) -> Git {
        let dir = AbsolutePath::from_str(dir.to_str().unwrap()).unwrap();
        Git::new(
            dir,
            CommitIdentity::new("harness".to_owned(), "harness@example.com".to_owned(), None),
        )
    }

    fn plan() -> RepoPath {
        RepoPath::from_str(PLAN).unwrap()
    }

    /// A repository with one commit of `README.md` and a plan file, not added.
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("README.md"), "read me\n").unwrap();
        git(dir.path(), &["add", "README.md"]);
        git(dir.path(), &["commit", "-q", "-m", "first"]);
        std::fs::create_dir(dir.path().join(".agent")).unwrap();
        std::fs::write(dir.path().join(PLAN), "Plan — zażółć.\n\n1. Do\n[ ] 1.1 It\n").unwrap();
        dir
    }

    fn tracked(dir: &Path) -> String {
        git(dir, &["ls-files", "--cached"])
    }

    #[tokio::test]
    async fn a_committed_plan_is_removed_from_the_branch_by_a_commit_and_its_file_stays() {
        let dir = repo();
        git(dir.path(), &["add", "-f", PLAN]);
        git(dir.path(), &["commit", "-q", "-m", "by mistake"]);
        let before = git(dir.path(), &["rev-parse", "HEAD"]);

        let result = handle(dir.path())
            .untrack(&plan(), "Keep the plan out of git")
            .await
            .unwrap();

        let Untracked::Removed { commit } = result else {
            panic!("a commit was expected: {result:?}");
        };
        let head = handle(dir.path()).rev_parse(&Revision::Head).await.unwrap();
        assert_eq!(head, commit);
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD^"]), before);
        assert_eq!(
            git(dir.path(), &["log", "-1", "--format=%s %an"]),
            "Keep the plan out of git harness\n"
        );
        assert_eq!(tracked(dir.path()), "README.md\n");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(PLAN)).unwrap(),
            "Plan — zażółć.\n\n1. Do\n[ ] 1.1 It\n"
        );
    }

    #[tokio::test]
    async fn a_plan_only_staged_leaves_the_index_without_a_commit() {
        let dir = repo();
        git(dir.path(), &["add", "-f", PLAN]);
        let before = git(dir.path(), &["rev-parse", "HEAD"]);

        let result = handle(dir.path())
            .untrack(&plan(), "Keep the plan out of git")
            .await
            .unwrap();

        assert_eq!(result, Untracked::Unstaged);
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), before);
        assert_eq!(tracked(dir.path()), "README.md\n");
        assert!(dir.path().join(PLAN).is_file());
    }

    #[tokio::test]
    async fn an_untracked_plan_is_left_as_it_is() {
        let dir = repo();
        let before = git(dir.path(), &["rev-parse", "HEAD"]);

        let result = handle(dir.path())
            .untrack(&plan(), "Keep the plan out of git")
            .await
            .unwrap();

        assert_eq!(result, Untracked::NotTracked);
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), before);
    }

    #[tokio::test]
    async fn other_staged_changes_leave_the_checkout_as_it_is() {
        let dir = repo();
        git(dir.path(), &["add", "-f", PLAN]);
        git(dir.path(), &["commit", "-q", "-m", "by mistake"]);
        std::fs::write(dir.path().join("README.md"), "changed\n").unwrap();
        git(dir.path(), &["add", "README.md"]);
        let before = git(dir.path(), &["rev-parse", "HEAD"]);

        let result = handle(dir.path())
            .untrack(&plan(), "Keep the plan out of git")
            .await
            .unwrap();

        assert_eq!(result, Untracked::OtherChangesStaged);
        assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), before);
        assert_eq!(tracked(dir.path()), ".agent/plan.md\nREADME.md\n");
        assert_eq!(git(dir.path(), &["diff", "--cached", "--name-only"]), "README.md\n");
    }

    #[tokio::test]
    async fn an_empty_message_is_refused() {
        let dir = repo();
        git(dir.path(), &["add", "-f", PLAN]);
        git(dir.path(), &["commit", "-q", "-m", "by mistake"]);

        let error = handle(dir.path()).untrack(&plan(), " ").await.unwrap_err();

        assert!(matches!(error, GitError::EmptyMessage), "{error:?}");
        assert_eq!(tracked(dir.path()), ".agent/plan.md\nREADME.md\n");
    }
}
