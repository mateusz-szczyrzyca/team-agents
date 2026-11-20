//! Delivery checks run against isolated real repositories, never the network.

use super::*;
use crate::git::{CommitIdentity, Git};
use std::path::Path;
use std::process::Command;

fn raw(dir: &Path, args: &[&str]) -> std::io::Result<()> {
    let result = Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .output()?;
    if !result.status.success() {
        return Err(std::io::Error::other(
            String::from_utf8(result.stderr).map_err(std::io::Error::other)?,
        ));
    }
    Ok(())
}

#[tokio::test]
async fn delivery_proof_accepts_only_clean_contained_item_tips() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    raw(root, &["init", "-q", "-b", "main"]).unwrap();
    std::fs::write(root.join("内容.txt"), "base").unwrap();
    raw(root, &["add", "."]).unwrap();
    raw(root, &["commit", "-qm", "Base"]).unwrap();
    raw(root, &["branch", "actual-pr"]).unwrap();
    raw(root, &["switch", "-qc", "item"]).unwrap();
    let git = Git::new(
        root.to_str().unwrap().parse().unwrap(),
        CommitIdentity::new("Fixture".into(), "fixture@example.com".into(), None),
    )
    .with_env("GIT_CONFIG_GLOBAL".parse().unwrap(), "/dev/null")
    .with_env("GIT_CONFIG_NOSYSTEM".parse().unwrap(), "1");
    let item = "item".parse().unwrap();
    let pr = "actual-pr".parse().unwrap();
    let base = Revision::Branch("main".parse().unwrap());
    git.verify_delivery(&item, &pr, &base).await.unwrap();
    // An already merged PR may have had its branch removed.
    git.verify_delivery(&item, &"deleted-pr".parse().unwrap(), &base)
        .await
        .unwrap();
    std::fs::write(root.join("untracked.txt"), "work").unwrap();
    assert!(matches!(
        git.verify_delivery(&item, &pr, &base).await,
        Err(DeliveryGitError::Dirty)
    ));
    raw(root, &["add", "."]).unwrap();
    raw(root, &["commit", "-qm", "Item work"]).unwrap();
    assert!(matches!(
        git.verify_delivery(&item, &pr, &base).await,
        Err(DeliveryGitError::OwnCommits)
    ));
    raw(root, &["branch", "-f", "actual-pr", "item"]).unwrap();
    git.verify_delivery(&item, &pr, &base).await.unwrap();
    std::fs::write(root.join("内容.txt"), "tracked work").unwrap();
    assert!(matches!(
        git.verify_delivery(&item, &pr, &base).await,
        Err(DeliveryGitError::Dirty)
    ));
    raw(root, &["restore", "."]).unwrap();
    raw(root, &["switch", "-q", "main"]).unwrap();
    assert!(matches!(
        git.verify_delivery(&item, &pr, &base).await,
        Err(DeliveryGitError::Checkout)
    ));
}

#[tokio::test]
async fn delivery_proof_preserves_errors_for_every_required_reference_and_tree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = Git::new(
        root.to_str().unwrap().parse().unwrap(),
        CommitIdentity::new("Fixture".into(), "fixture@example.com".into(), None),
    );
    let item = "item".parse().unwrap();
    let pr = "actual-pr".parse().unwrap();
    let base = Revision::Branch("main".parse().unwrap());
    assert!(matches!(
        git.verify_delivery(&item, &pr, &base).await,
        Err(DeliveryGitError::Read {
            read: DeliveryRead::Tree,
            ..
        })
    ));
    raw(root, &["init", "-q", "-b", "main"]).unwrap();
    assert!(matches!(
        git.verify_delivery(&item, &pr, &base).await,
        Err(DeliveryGitError::Read {
            read: DeliveryRead::Head,
            ..
        })
    ));
    raw(root, &["commit", "--allow-empty", "-qm", "Base"]).unwrap();
    assert!(matches!(
        git.verify_delivery(&item, &pr, &base).await,
        Err(DeliveryGitError::Read {
            read: DeliveryRead::Item,
            ..
        })
    ));
    raw(root, &["branch", "item"]).unwrap();
    let absent = Revision::Branch("absent".parse().unwrap());
    assert!(matches!(
        git.verify_delivery(&item, &pr, &absent).await,
        Err(DeliveryGitError::Read {
            read: DeliveryRead::Base,
            ..
        })
    ));
    raw(root, &["switch", "-q", "item"]).unwrap();
    raw(root, &["commit", "--allow-empty", "-qm", "Own commit"]).unwrap();
    assert!(matches!(
        git.verify_delivery(&item, &pr, &base).await,
        Err(DeliveryGitError::Read {
            read: DeliveryRead::PullRequest,
            ..
        })
    ));
}
