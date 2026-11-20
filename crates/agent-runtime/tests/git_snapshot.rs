//! A snapshot of a worktree: everything in it is kept as a commit under a reference of its round and task, and the worktree,
//! its index and its files are what they were before. Read from real repositories in temporary directories, isolated from
//! the configuration of the machine.

#[cfg(test)]
mod git_snapshot {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::{RoundId, TaskId};
    use agent_core::state::CommitId;
    use agent_runtime::git::{CommitIdentity, Git, GitError};

    /// The round and the task of the snapshots of these tests.
    const ROUND: &str = "round-20261120-100000-r1r1";
    /// See [`ROUND`].
    const TASK: &str = "task-20261120-100000-t1t1";
    /// The size of the binary file of the tests: more than the 5 MB the snapshot must hold.
    const BINARY_BYTES: usize = 5 * 1024 * 1024;

    /// Runs real git for the setup of a test and returns its output without the last line break.
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
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("git prints text")
            .trim_end_matches('\n')
            .to_owned()
    }

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().expect("a UTF-8 path")).expect("an absolute path")
    }

    fn handle(dir: &Path) -> Git {
        Git::new(
            abs(dir),
            CommitIdentity::new("t".to_owned(), "t@example.com".to_owned(), None),
        )
    }

    fn head_of(repo: &Path) -> CommitId {
        raw(repo, &["rev-parse", "HEAD"]).parse().expect("a commit")
    }

    fn write(repo: &Path, name: &str, bytes: &[u8]) {
        let file = repo.join(name);
        std::fs::create_dir_all(file.parent().expect("a parent")).expect("a directory");
        std::fs::write(file, bytes).expect("a file");
    }

    fn round() -> RoundId {
        RoundId::from_str(ROUND).expect("a round id")
    }

    fn task() -> TaskId {
        TaskId::from_str(TASK).expect("a task id")
    }

    /// An empty repository on `main` in a temporary directory.
    fn repository() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let repo = tmp.path().canonicalize().expect("a canonical path").join("repo");
        std::fs::create_dir(&repo).expect("the repository directory");
        raw(&repo, &["init", "-q", "-b", "main"]);
        (tmp, repo)
    }

    /// The state a snapshot must not change: git's view of the worktree, the bytes of the index and of the named files.
    fn state(repo: &Path, files: &[&str]) -> (String, Vec<u8>, Vec<Vec<u8>>) {
        let status = raw(
            repo,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignored"],
        );
        let index = std::fs::read(repo.join(".git").join("index")).expect("the index");
        let contents = files
            .iter()
            .map(|name| std::fs::read(repo.join(name)).unwrap_or_default())
            .collect();
        (status, index, contents)
    }

    /// The paths of the tree of `commit`.
    fn paths_of(repo: &Path, commit: &CommitId) -> Vec<String> {
        raw(repo, &["ls-tree", "-r", "--name-only", commit.to_string().as_str()])
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[tokio::test]
    async fn a_snapshot_leaves_the_worktree_its_index_and_its_files_as_they_were() {
        let (_tmp, repo) = repository();
        write(&repo, "kept.txt", b"kept\n");
        write(&repo, "changed.txt", b"before\n");
        write(&repo, "deleted.txt", b"gone\n");
        raw(&repo, &["add", "-A"]);
        raw(&repo, &["commit", "-q", "-m", "base"]);
        write(&repo, "changed.txt", b"after\n");
        std::fs::remove_file(repo.join("deleted.txt")).expect("the deletion");
        write(&repo, "staged.txt", b"staged\n");
        raw(&repo, &["add", "staged.txt"]);
        write(&repo, "untracked.txt", b"new\n");
        let files = ["kept.txt", "changed.txt", "staged.txt", "untracked.txt"];
        let before = state(&repo, &files);

        let snapshot = handle(&repo).snapshot(&round(), &task()).await.expect("a snapshot");

        assert_eq!(state(&repo, &files), before);
        let paths = paths_of(&repo, &snapshot.commit);
        for kept in ["kept.txt", "changed.txt", "staged.txt", "untracked.txt"] {
            assert!(
                paths.iter().any(|path| path == kept),
                "{kept} is in the snapshot: {paths:?}"
            );
        }
        assert!(
            !paths.iter().any(|path| path == "deleted.txt"),
            "a deleted file is gone: {paths:?}"
        );
        assert_eq!(
            raw(&repo, &["show", &format!("{}:changed.txt", snapshot.commit)]),
            "after"
        );
    }

    #[tokio::test]
    async fn untracked_and_binary_files_are_kept_and_ignored_files_are_not() {
        let (_tmp, repo) = repository();
        write(&repo, ".gitignore", b"*.log\n");
        write(&repo, "base.txt", b"base\n");
        raw(&repo, &["add", "-A"]);
        raw(&repo, &["commit", "-q", "-m", "base"]);
        write(&repo, "build.log", b"ignored\n");
        write(&repo, "notes.md", b"untracked notes\n");
        let binary: Vec<u8> = (0..BINARY_BYTES)
            .map(|index| u8::try_from(index % 251).expect("a byte"))
            .collect();
        write(&repo, "images/picture.bin", &binary);

        let snapshot = handle(&repo).snapshot(&round(), &task()).await.expect("a snapshot");

        let paths = paths_of(&repo, &snapshot.commit);
        assert!(paths.iter().any(|path| path == "notes.md"), "{paths:?}");
        assert!(paths.iter().any(|path| path == "images/picture.bin"), "{paths:?}");
        assert!(!paths.iter().any(|path| path == "build.log"), "{paths:?}");
        let size = raw(
            &repo,
            &["cat-file", "-s", &format!("{}:images/picture.bin", snapshot.commit)],
        );
        assert_eq!(size.parse::<usize>().expect("a size"), BINARY_BYTES);
        let stored = Command::new("git")
            .args(["cat-file", "blob", &format!("{}:images/picture.bin", snapshot.commit)])
            .current_dir(&repo)
            .output()
            .expect("git runs");
        assert!(stored.stdout == binary, "the binary file is kept byte for byte");
        assert!(snapshot.bytes >= u64::try_from(BINARY_BYTES).expect("a size"));
    }

    #[tokio::test]
    async fn a_hook_and_a_filter_of_the_repository_do_not_run() {
        let (tmp, repo) = repository();
        write(&repo, ".gitattributes", b"*.txt filter=up\n");
        write(&repo, "changed.txt", b"before\n");
        raw(&repo, &["add", "-A"]);
        raw(&repo, &["commit", "-q", "-m", "base"]);
        let marker = tmp.path().join("ran");
        let script = tmp.path().join("touch-marker.sh");
        std::fs::write(&script, format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display())).expect("the script");
        let mut permissions = std::fs::metadata(&script).expect("the script").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
        std::fs::set_permissions(&script, permissions).expect("the permissions");
        let hook = repo.join(".git").join("hooks").join("pre-commit");
        std::fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).expect("the hook");
        let mut hook_permissions = std::fs::metadata(&hook).expect("the hook").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut hook_permissions, 0o755);
        std::fs::set_permissions(&hook, hook_permissions).expect("the permissions");
        raw(&repo, &["config", "core.fsmonitor", script.to_str().expect("a path")]);
        raw(&repo, &["config", "filter.up.clean", script.to_str().expect("a path")]);
        raw(&repo, &["config", "filter.up.required", "true"]);
        write(&repo, "changed.txt", b"after\n");

        handle(&repo).snapshot(&round(), &task()).await.expect("a snapshot");

        assert!(!marker.exists(), "no hook, filter or monitor of the repository ran");
    }

    #[tokio::test]
    async fn two_snapshots_of_an_unchanged_worktree_are_one_tree_numbered_one_and_two() {
        let (_tmp, repo) = repository();
        write(&repo, "file.txt", b"content\n");
        raw(&repo, &["add", "-A"]);
        raw(&repo, &["commit", "-q", "-m", "base"]);
        write(&repo, "file.txt", b"edited\n");

        let first = handle(&repo).snapshot(&round(), &task()).await.expect("a snapshot");
        let second = handle(&repo)
            .snapshot(&round(), &task())
            .await
            .expect("a second snapshot");

        assert_eq!((first.number, second.number), (1, 2));
        assert_eq!(
            raw(&repo, &["rev-parse", &format!("{}^{{tree}}", first.commit)]),
            raw(&repo, &["rev-parse", &format!("{}^{{tree}}", second.commit)])
        );
        let prefix = format!("refs/agents/snapshots/{ROUND}/{TASK}");
        assert_eq!(
            raw(&repo, &["rev-parse", &format!("{prefix}/1")]),
            first.commit.to_string()
        );
        assert_eq!(
            raw(&repo, &["rev-parse", &format!("{prefix}/2")]),
            second.commit.to_string()
        );
    }

    #[tokio::test]
    async fn the_parent_of_a_snapshot_is_the_head_of_the_worktree() {
        let (_tmp, repo) = repository();
        write(&repo, "file.txt", b"content\n");
        raw(&repo, &["add", "-A"]);
        raw(&repo, &["commit", "-q", "-m", "base"]);
        let head = head_of(&repo);
        write(&repo, "file.txt", b"edited\n");

        let snapshot = handle(&repo).snapshot(&round(), &task()).await.expect("a snapshot");

        assert_eq!(snapshot.head, head);
        assert_eq!(
            raw(&repo, &["rev-parse", &format!("{}^", snapshot.commit)]),
            head.to_string()
        );
        assert_eq!(head_of(&repo), head, "the branch did not move");
    }

    #[tokio::test]
    async fn a_directory_that_is_no_repository_has_no_snapshot() {
        let tmp = tempfile::tempdir().expect("a temporary directory");

        let error = handle(tmp.path())
            .snapshot(&round(), &task())
            .await
            .expect_err("no repository to snapshot");

        assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
    }
}
