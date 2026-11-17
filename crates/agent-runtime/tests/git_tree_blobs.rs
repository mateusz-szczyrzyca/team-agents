//! Tests for `Git::tree_blobs` and its parser: listing blob paths in a tree, skipping
//! non-blob entries (e.g. tree or submodule commit entries), enforcing output byte limits,
//! rejecting empty paths, and handling CLI errors.

#[cfg(test)]
mod git_tree_blobs {
    use std::collections::HashSet;
    use std::os::unix::fs::PermissionsExt;
    use std::str::FromStr;

    use agent_core::config::{AbsolutePath, EnvName};
    use agent_core::state::CommitId;
    use agent_runtime::git::{CommitIdentity, Git, GitError, GitLimits, GitOperation};
    use agent_runtime::process::ExitReport;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// A git handle whose `git` command is a shell script executing `body`.
    fn scripted_git(body: &str) -> TestResult<(tempfile::TempDir, Git)> {
        let dir = tempfile::tempdir()?;
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin)?;
        let script = bin.join("git");
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n"))?;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
        let path_str = dir
            .path()
            .to_str()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid utf-8 path"))?;
        let bin_str = bin
            .to_str()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid utf-8 bin path"))?;
        let git = Git::new(
            AbsolutePath::from_str(path_str)?,
            CommitIdentity::new("Fixture".into(), "fixture@example.com".into(), None),
        )
        .with_env(EnvName::from_str("PATH")?, bin_str);
        Ok((dir, git))
    }

    fn dummy_commit() -> TestResult<CommitId> {
        Ok(CommitId::from_str("0123456789abcdef0123456789abcdef01234567")?)
    }

    #[tokio::test]
    async fn tree_blobs_fails_with_output_too_long_when_stdout_truncated() -> TestResult {
        let (_dir, git) = scripted_git(
            r"printf '%b\0%b\0' '100644 blob 1111111111111111111111111111111111111111\tpath/to/first.txt' '100644 blob 2222222222222222222222222222222222222222\tpath/to/second.txt'",
        )?;
        let git = git.with_limits(GitLimits {
            max_output_bytes: 32,
            ..GitLimits::default()
        });

        let result = git.tree_blobs(&dummy_commit()?).await;

        assert!(
            matches!(
                result,
                Err(GitError::OutputTooLong {
                    operation: GitOperation::LsTree,
                    limit: 32,
                })
            ),
            "expected OutputTooLong with limit 32, got {result:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tree_blobs_rejects_empty_path_with_unexpected() -> TestResult {
        let (_dir, git) = scripted_git(r"printf '100644 blob 1111111111111111111111111111111111111111\t\0'")?;

        let result = git.tree_blobs(&dummy_commit()?).await;

        assert!(
            matches!(
                result,
                Err(GitError::Unexpected {
                    operation: GitOperation::LsTree,
                    ..
                })
            ),
            "expected Unexpected error on empty path, got {result:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tree_blobs_skips_non_blob_entries_such_as_trees_and_submodule_commits() -> TestResult {
        let (_dir, git) = scripted_git(
            r"printf '%b\0%b\0%b\0' '040000 tree 2222222222222222222222222222222222222222\tsubdir' '160000 commit 3333333333333333333333333333333333333333\tsubmodule' '100644 blob 1111111111111111111111111111111111111111\tsrc/main.rs'",
        )?;

        let result = git.tree_blobs(&dummy_commit()?).await?;

        assert_eq!(result, HashSet::from(["src/main.rs".to_owned()]));
        Ok(())
    }

    #[tokio::test]
    async fn tree_blobs_fake_cli_reports_output_too_long() -> TestResult {
        let (_dir, git) = scripted_git(r"printf '100644 blob 1111111111111111111111111111111111111111\ta.txt\0'")?;
        let git = git.with_limits(GitLimits {
            max_output_bytes: 16,
            ..GitLimits::default()
        });

        let result = git.tree_blobs(&dummy_commit()?).await;

        assert!(
            matches!(
                result,
                Err(GitError::OutputTooLong {
                    operation: GitOperation::LsTree,
                    limit: 16,
                })
            ),
            "expected OutputTooLong with limit 16, got {result:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tree_blobs_fake_cli_reports_unexpected_on_malformed_output() -> TestResult {
        let (_dir, git) = scripted_git(r"printf 'malformed output without proper delimiters\0'")?;

        let result = git.tree_blobs(&dummy_commit()?).await;

        assert!(
            matches!(
                result,
                Err(GitError::Unexpected {
                    operation: GitOperation::LsTree,
                    ..
                })
            ),
            "expected Unexpected on malformed output, got {result:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tree_blobs_fake_cli_reports_not_utf8() -> TestResult {
        // \377\376 is invalid UTF-8 bytes (0xFF 0xFE)
        let (_dir, git) = scripted_git(r"printf '100644 blob 1111111111111111111111111111111111111111\t\377\376\0'")?;

        let result = git.tree_blobs(&dummy_commit()?).await;

        assert!(
            matches!(
                result,
                Err(GitError::NotUtf8 {
                    operation: GitOperation::LsTree,
                    ..
                })
            ),
            "expected NotUtf8 on invalid UTF-8 path, got {result:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tree_blobs_fake_cli_reports_failed_on_nonzero_exit() -> TestResult {
        let (_dir, git) = scripted_git(r"printf 'fatal: not a tree object\n' >&2; exit 128")?;

        let result = git.tree_blobs(&dummy_commit()?).await;

        let Err(GitError::Failed {
            operation,
            exit,
            stderr,
        }) = result
        else {
            panic!("expected Failed, got {result:?}");
        };
        assert_eq!(operation, GitOperation::LsTree);
        assert_eq!(exit, ExitReport::Exited { code: 128 });
        assert!(
            stderr.contains("fatal: not a tree object"),
            "stderr should mention failure reason: {stderr}"
        );
        Ok(())
    }
}
