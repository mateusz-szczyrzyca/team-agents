//! Finding run paths include every kind of local change.
#[cfg(test)]
mod tests {
    use agent_runtime::git::{CommitIdentity, Git};
    use std::path::Path;

    fn git(dir: &Path, args: &[&str]) {
        let result = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(result.status.success(), "{}", String::from_utf8(result.stderr).unwrap());
    }

    #[tokio::test]
    async fn changed_files_includes_committed_dirty_untracked_deleted_and_spaced_paths() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-b", "main"]);
        for name in ["committed", "dirty", "deleted", "unchanged"] {
            std::fs::write(dir.path().join(name), "base").unwrap();
        }
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "Base"]);
        git(dir.path(), &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        let handle = Git::new(
            dir.path().to_str().unwrap().parse().unwrap(),
            CommitIdentity::new("Fixture".into(), "fixture@example.com".into(), None),
        );
        assert_eq!(
            handle.changed_files(&"main".parse().unwrap()).await.unwrap(),
            Vec::new()
        );
        std::fs::write(dir.path().join("committed"), "commit").unwrap();
        git(dir.path(), &["add", "committed"]);
        git(dir.path(), &["commit", "-m", "Change"]);
        std::fs::write(dir.path().join("dirty"), "dirty").unwrap();
        std::fs::remove_file(dir.path().join("deleted")).unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("docs/user guide é.md"), "new").unwrap();
        let paths = handle.changed_files(&"main".parse().unwrap()).await.unwrap();
        assert_eq!(
            paths.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
            ["committed", "deleted", "dirty", "docs/user guide é.md"]
        );
    }
}
