//! Writing the unmerged branches of a removed team into one git bundle: the bundle holds their commits, a fetch from it
//! brings them back, and a branch git does not have is an error rather than an empty bundle.

#[cfg(test)]
mod git_bundle {
    use std::path::Path;
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::BranchName;
    use agent_runtime::git::{CommitIdentity, Git, GitError};

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

    fn branch(name: &str) -> BranchName {
        BranchName::from_str(name).unwrap()
    }

    /// A repository on `main` with one commit, and a branch `work/unmerged` with one commit of its own.
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("README.md"), "read me\n").unwrap();
        git(dir.path(), &["add", "README.md"]);
        git(dir.path(), &["commit", "-q", "-m", "first"]);
        git(dir.path(), &["checkout", "-q", "-b", "work/unmerged"]);
        std::fs::write(dir.path().join("work.txt"), "not merged yet\n").unwrap();
        git(dir.path(), &["add", "work.txt"]);
        git(dir.path(), &["commit", "-q", "-m", "unmerged work"]);
        git(dir.path(), &["checkout", "-q", "main"]);
        dir
    }

    #[tokio::test]
    async fn the_bundle_holds_the_branch_and_a_fetch_from_it_brings_the_commit_back() {
        let dir = repo();
        let bundle_dir = tempfile::tempdir().unwrap();
        let bundle = AbsolutePath::from_str(bundle_dir.path().join("team.bundle").to_str().unwrap()).unwrap();
        let tip = git(dir.path(), &["rev-parse", "work/unmerged"]).trim().to_owned();

        handle(dir.path())
            .bundle_create(&bundle, &[branch("work/unmerged")])
            .await
            .unwrap();

        assert!(bundle_dir.path().join("team.bundle").is_file());
        git(dir.path(), &["bundle", "verify", bundle.as_path().to_str().unwrap()]);
        let heads = git(
            dir.path(),
            &["bundle", "list-heads", bundle.as_path().to_str().unwrap()],
        );
        assert!(heads.contains(&format!("{tip} refs/heads/work/unmerged")), "{heads}");
        let other = tempfile::tempdir().unwrap();
        git(other.path(), &["init", "-q", "-b", "main"]);
        git(
            other.path(),
            &[
                "fetch",
                "-q",
                bundle.as_path().to_str().unwrap(),
                "refs/heads/work/unmerged:refs/heads/back",
            ],
        );
        assert_eq!(git(other.path(), &["rev-parse", "back"]).trim(), tip);
    }

    #[tokio::test]
    async fn a_branch_that_does_not_exist_is_an_error_and_no_bundle_is_made() {
        let dir = repo();
        let bundle_dir = tempfile::tempdir().unwrap();
        let bundle = AbsolutePath::from_str(bundle_dir.path().join("team.bundle").to_str().unwrap()).unwrap();

        let result = handle(dir.path())
            .bundle_create(&bundle, &[branch("work/missing")])
            .await;

        assert!(matches!(result, Err(GitError::Failed { .. })), "{result:?}");
        assert!(!bundle_dir.path().join("team.bundle").exists());
    }
}
