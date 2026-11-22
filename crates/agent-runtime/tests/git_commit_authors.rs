//! Who wrote the commits of a round, read from real repositories in temporary directories, isolated from the
//! configuration of the machine: the question to the human about a frozen test names the author of each commit.

#[cfg(test)]
mod git_commit_authors {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::state::CommitId;
    use agent_runtime::git::{CommitIdentity, Git, GitError};

    /// Runs real git for the setup of a test as the author `author` and returns its output without the last line break.
    fn raw(dir: &Path, author: &str, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", author)
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Committer Name")
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

    /// Commits an empty change as `author` and returns the commit.
    fn commit_as(repo: &Path, author: &str, message: &str) -> CommitId {
        raw(repo, author, &["commit", "-q", "--allow-empty", "-m", message]);
        raw(repo, author, &["rev-parse", "HEAD"]).parse().unwrap()
    }

    fn repository() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir(&repo).unwrap();
        raw(&repo, "Init", &["init", "-q", "-b", "main"]);
        (tmp, repo)
    }

    // Rule: each commit has the name of its author, not of its committer, whatever the text of the name is.
    #[tokio::test]
    async fn each_commit_has_the_name_of_its_author_and_not_of_its_committer() {
        let (_tmp, repo) = repository();
        let first = commit_as(&repo, "Ada Author", "one");
        let second = commit_as(&repo, "Żółta Gęś", "two");

        let authors = handle(&repo)
            .commit_authors(&[second.clone(), first.clone()])
            .await
            .unwrap();

        assert_eq!(authors.get(&first).map(String::as_str), Some("Ada Author"));
        assert_eq!(authors.get(&second).map(String::as_str), Some("Żółta Gęś"));
        assert_eq!(authors.len(), 2);
    }

    // Rule: no commit asked about, nothing is run and nothing is returned.
    #[tokio::test]
    async fn no_commit_asked_about_gives_no_author() {
        let (_tmp, repo) = repository();
        commit_as(&repo, "Ada Author", "one");

        let authors = handle(&repo).commit_authors(&[]).await.unwrap();

        assert!(authors.is_empty());
    }

    // Rule: a commit the repository does not have is an error that says so, not a missing name.
    #[tokio::test]
    async fn a_commit_the_repository_does_not_have_is_an_error() {
        let (_tmp, repo) = repository();
        commit_as(&repo, "Ada Author", "one");
        let unknown: CommitId = "e".repeat(40).parse().unwrap();

        let error = handle(&repo).commit_authors(&[unknown]).await.unwrap_err();

        assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
    }
}
