//! Activity reconstruction uses committer dates rather than author dates.
use super::*;
use crate::git::CommitIdentity;
use agent_core::config::AbsolutePath;
use std::process::Command;

#[tokio::test]
async fn root_commit_time_uses_committer_date_and_reports_missing_revisions() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["commit", "--allow-empty", "-qm", "root"],
    ] {
        let output = Command::new("git")
            .current_dir(dir.path())
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-11-11T19:15:00Z")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let git = Git::new(
        dir.path().to_str().unwrap().parse::<AbsolutePath>().unwrap(),
        CommitIdentity::new("Fixture".into(), "fixture@example.com".into(), None),
    );
    assert_eq!(
        git.commit_time(&Revision::Head).await.unwrap(),
        "2026-11-11T19:15:00Z".parse().unwrap()
    );
    assert!(matches!(
        git.commit_time(&Revision::Branch("missing".parse().unwrap())).await,
        Err(CommitTimeError::Git(GitError::Failed { .. }))
    ));
}
