//! What a run changed, as git says it: the commits between two heads with their author time, and the files a worktree
//! holds that no commit does. Read from real repositories in temporary directories, isolated from the configuration of the
//! machine; reading them leaves the repository as it was.

#[cfg(test)]
mod git_changes {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::decide::checks::{Change, FileChange, Lines};
    use agent_core::history::CommitRecord;
    use agent_core::repo_path::RepoPath;
    use agent_core::state::{CommitId, Timestamp};
    use agent_runtime::git::{CommitIdentity, Git, GitError, GitLimits};

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

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
    }

    fn handle(dir: &Path) -> Git {
        Git::new(
            abs(dir),
            CommitIdentity::new("t".to_owned(), "t@example.com".to_owned(), None),
        )
    }

    fn head_of(repo: &Path) -> CommitId {
        raw(repo, &["rev-parse", "HEAD"]).parse().unwrap()
    }

    fn write(repo: &Path, name: &str, bytes: &[u8]) {
        let file = repo.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }

    /// Commits everything that changed in `repo` with `message`, authored at `seconds` after the epoch.
    fn commit_at(repo: &Path, message: &str, seconds: i64) -> CommitId {
        raw(repo, &["add", "-A"]);
        let date = format!("{seconds} +0000");
        raw(repo, &["commit", "-q", "--allow-empty", "-m", message, "--date", &date]);
        head_of(repo)
    }

    /// An empty repository on `main` in a temporary directory.
    fn repository() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir(&repo).unwrap();
        raw(&repo, &["init", "-q", "-b", "main"]);
        (tmp, repo)
    }

    /// A linked worktree of `repo`, the shape the harness works in, on a branch of its own.
    fn linked_worktree(tmp: &tempfile::TempDir, repo: &Path) -> PathBuf {
        let tree = tmp.path().canonicalize().unwrap().join("tree");
        raw(repo, &["worktree", "add", "-q", "-b", "work", tree.to_str().unwrap()]);
        tree
    }

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_utc(time::UtcDateTime::from_unix_timestamp(seconds).unwrap()).unwrap()
    }

    fn change(path: &str, change: Change, lines: Lines) -> FileChange {
        FileChange {
            path: RepoPath::from_str(path).unwrap(),
            change,
            lines,
        }
    }

    fn text(added: u32, removed: u32) -> Lines {
        Lines::Text { added, removed }
    }

    fn by_path(mut changes: Vec<FileChange>) -> Vec<FileChange> {
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        changes
    }

    /// Every path under `dir`, with the size of each file: what a repository holds, to see that nothing was added.
    fn listing(dir: &Path) -> Vec<(PathBuf, u64)> {
        let mut found = Vec::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(next) = pending.pop() {
            for entry in std::fs::read_dir(&next).unwrap() {
                let entry = entry.unwrap();
                let metadata = entry.metadata().unwrap();
                if metadata.is_dir() {
                    pending.push(entry.path());
                } else {
                    found.push((entry.path(), metadata.len()));
                }
            }
        }
        found.sort();
        found
    }

    // ---- range_log ----------------------------------------------------------------------------------------------------

    // Rule: the commits of a range come oldest first, each with the first line of its message and when it was authored.
    #[tokio::test]
    async fn the_commits_of_a_range_come_oldest_first_with_their_subject_and_the_time_they_were_authored() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let start = commit_at(&repo, "start", 1_700_000_000);
        write(&repo, "b.txt", b"two\n");
        let second = commit_at(
            &repo,
            "Second: déjà vu 😀\n\nA body that is not the subject.",
            1_700_000_100,
        );
        write(&repo, "c.txt", b"three\n");
        let third = commit_at(&repo, "Third", 1_700_000_200);

        let log = handle(&repo).range_log(&start, &third).await.unwrap();

        assert_eq!(
            log,
            [
                CommitRecord {
                    id: second,
                    subject: "Second: déjà vu 😀".to_owned(),
                    authored_at: at(1_700_000_100)
                },
                CommitRecord {
                    id: third,
                    subject: "Third".to_owned(),
                    authored_at: at(1_700_000_200)
                },
            ]
        );
    }

    // Rule: a merge is listed, after the commits it brought in.
    #[tokio::test]
    async fn a_range_with_a_merge_lists_the_merge_after_the_commits_it_brought_in() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let start = commit_at(&repo, "start", 1_700_000_000);
        raw(&repo, &["switch", "-q", "-c", "side"]);
        write(&repo, "side.txt", b"side\n");
        let side = commit_at(&repo, "side work", 1_700_000_100);
        raw(&repo, &["switch", "-q", "main"]);
        write(&repo, "main.txt", b"main\n");
        let main = commit_at(&repo, "main work", 1_700_000_200);
        raw(&repo, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
        let merge = head_of(&repo);

        let log = handle(&repo).range_log(&start, &merge).await.unwrap();

        let ids: Vec<&CommitId> = log.iter().map(|commit| &commit.id).collect();
        assert_eq!(ids.len(), 3, "{log:?}");
        assert_eq!(ids.last().copied(), Some(&merge));
        assert!(ids.contains(&&side) && ids.contains(&&main), "{log:?}");
    }

    // Rule: a range with no commits is empty, whether its ends are one commit or one is the ancestor of the other.
    #[tokio::test]
    async fn a_range_with_nothing_in_it_has_no_commits() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let first = commit_at(&repo, "first", 1_700_000_000);
        write(&repo, "b.txt", b"two\n");
        let second = commit_at(&repo, "second", 1_700_000_100);
        let git = handle(&repo);

        assert_eq!(git.range_log(&first, &first).await.unwrap(), []);
        assert_eq!(git.range_log(&second, &first).await.unwrap(), []);
    }

    // Rule: a range with more commits than the limit is an error, one with exactly the limit is not.
    #[tokio::test]
    async fn a_range_over_the_limit_of_commits_is_refused_and_one_at_the_limit_is_read() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"0\n");
        let start = commit_at(&repo, "start", 1_700_000_000);
        for n in 1..=3 {
            write(&repo, "a.txt", format!("{n}\n").as_bytes());
            commit_at(&repo, &format!("c{n}"), 1_700_000_000 + n);
        }
        let head = head_of(&repo);
        let limits = |max_commits| GitLimits {
            max_commits,
            ..GitLimits::default()
        };

        let at_the_limit = handle(&repo)
            .with_limits(limits(3))
            .range_log(&start, &head)
            .await
            .unwrap();
        let over = handle(&repo)
            .with_limits(limits(2))
            .range_log(&start, &head)
            .await
            .unwrap_err();

        assert_eq!(at_the_limit.len(), 3);
        assert!(matches!(over, GitError::TooManyCommits { limit: 2 }), "{over:?}");
    }

    // ---- uncommitted_changes ------------------------------------------------------------------------------------------

    // Rule: a clean tree has no uncommitted changes, and files git is told to ignore are not changes.
    #[tokio::test]
    async fn a_clean_tree_has_nothing_uncommitted_and_an_ignored_file_is_not_a_change() {
        let (_tmp, repo) = repository();
        write(&repo, ".gitignore", b"target/\n");
        write(&repo, "a.txt", b"one\n");
        commit_at(&repo, "start", 1_700_000_000);
        let git = handle(&repo);
        assert_eq!(git.uncommitted_changes().await.unwrap(), []);

        write(&repo, "target/out.bin", b"ignored\n");
        write(&repo, ".git/info/exclude", b"scratch.txt\n");
        write(&repo, "scratch.txt", b"excluded\n");

        assert_eq!(git.uncommitted_changes().await.unwrap(), []);
    }

    // Rule: every way a file can differ from the commit is listed with its lines: modified, deleted, new (tracked or not),
    // renamed, binary.
    #[tokio::test]
    async fn every_kind_of_uncommitted_change_is_listed_with_its_lines() {
        let (_tmp, repo) = repository();
        write(&repo, "modified.txt", b"one\ntwo\nthree\n");
        write(&repo, "deleted.txt", b"gone\n");
        write(
            &repo,
            "moved.txt",
            b"a long enough line to be recognised as the same file\nand another one\n",
        );
        write(&repo, "staged.txt", b"before\n");
        write(&repo, "dir with space/é.txt", b"x\n");
        commit_at(&repo, "start", 1_700_000_000);

        write(&repo, "modified.txt", b"one\nTWO\nthree\nfour\n");
        std::fs::remove_file(repo.join("deleted.txt")).unwrap();
        std::fs::rename(repo.join("moved.txt"), repo.join("renamed.txt")).unwrap();
        write(&repo, "staged.txt", b"after\nafter\n");
        raw(&repo, &["add", "staged.txt"]);
        write(&repo, "new.txt", b"1\n2\n3\n");
        write(&repo, "image.bin", &[0, 1, 2, 3, 0, 255]);
        write(&repo, "dir with space/é.txt", b"x\ny\n");

        let changes = handle(&repo).uncommitted_changes().await.unwrap();

        assert_eq!(
            by_path(changes),
            [
                change("deleted.txt", Change::Deleted, text(0, 1)),
                change("dir with space/é.txt", Change::Modified, text(1, 0)),
                change("image.bin", Change::Added, Lines::Binary),
                change("modified.txt", Change::Modified, text(2, 1)),
                change("new.txt", Change::Added, text(3, 0)),
                change(
                    "renamed.txt",
                    Change::Renamed {
                        from: RepoPath::from_str("moved.txt").unwrap()
                    },
                    text(0, 0)
                ),
                change("staged.txt", Change::Modified, text(2, 1)),
            ]
        );
    }

    // Rule: reading what is uncommitted changes nothing: not the index, not the objects, not a file under the repository's
    // directory; in a linked worktree, where the harness works, as in a repository.
    #[tokio::test]
    async fn reading_the_uncommitted_changes_leaves_the_repository_as_it_was() {
        let (tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        commit_at(&repo, "start", 1_700_000_000);
        let tree = linked_worktree(&tmp, &repo);
        write(&tree, "a.txt", b"one\ntwo\n");
        write(&tree, "untracked.txt", &b"x".repeat(10_000));
        raw(&tree, &["add", "a.txt"]);
        let before = listing(&repo.join(".git"));
        let status = raw(&tree, &["status", "--porcelain=v2"]);

        let changes = handle(&tree).uncommitted_changes().await.unwrap();

        assert_eq!(changes.len(), 2, "{changes:?}");
        assert_eq!(listing(&repo.join(".git")), before);
        assert_eq!(raw(&tree, &["status", "--porcelain=v2"]), status);
    }

    // Rule: a merge that waits in the tree to be committed is read as what it will change.
    #[tokio::test]
    async fn a_merge_that_waits_to_be_committed_shows_what_it_brought_in() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        commit_at(&repo, "start", 1_700_000_000);
        raw(&repo, &["switch", "-q", "-c", "side"]);
        write(&repo, "side.txt", b"s1\ns2\n");
        commit_at(&repo, "side", 1_700_000_100);
        raw(&repo, &["switch", "-q", "main"]);
        write(&repo, "main.txt", b"m\n");
        commit_at(&repo, "main", 1_700_000_200);
        raw(&repo, &["merge", "-q", "--no-ff", "--no-commit", "side"]);

        let changes = handle(&repo).uncommitted_changes().await.unwrap();

        assert_eq!(changes, [change("side.txt", Change::Added, text(2, 0))]);
    }

    // Rule: a directory that is no repository is an error, not an empty answer.
    #[tokio::test]
    async fn a_directory_that_is_no_repository_cannot_be_read() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().canonicalize().unwrap();

        let error = handle(&dir).uncommitted_changes().await.unwrap_err();

        assert!(matches!(error, GitError::Failed { .. }), "{error:?}");
    }

    // ---- work_changes -------------------------------------------------------------------------------------------------

    // Rule: what a run changed is the commits and files between its two heads, and what it left in the tree.
    #[tokio::test]
    async fn what_a_run_changed_is_what_lies_between_its_heads_and_what_it_left_in_the_tree() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let before = commit_at(&repo, "start", 1_700_000_000);
        write(&repo, "a.txt", b"one\ntwo\n");
        write(&repo, "b.txt", b"new\n");
        let after = commit_at(&repo, "the run's commit", 1_700_000_100);
        write(&repo, "left.txt", b"left over\n");

        let changes = handle(&repo).work_changes(Some(&before)).await.unwrap();

        assert_eq!(changes.head_after, after);
        assert_eq!(
            changes.commits,
            [CommitRecord {
                id: after,
                subject: "the run's commit".to_owned(),
                authored_at: at(1_700_000_100)
            }]
        );
        assert_eq!(
            by_path(changes.files),
            [
                change("a.txt", Change::Modified, text(1, 0)),
                change("b.txt", Change::Added, text(1, 0))
            ]
        );
        assert_eq!(changes.uncommitted, [change("left.txt", Change::Added, text(1, 0))]);
    }

    // Rule: a run that did not move its head has no commits and no files between its heads.
    #[tokio::test]
    async fn a_run_that_did_not_commit_has_no_commits_and_no_files() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let head = commit_at(&repo, "start", 1_700_000_000);

        let changes = handle(&repo).work_changes(Some(&head)).await.unwrap();

        assert_eq!(
            (changes.head_after, changes.commits, changes.files, changes.uncommitted),
            (head, vec![], vec![], vec![])
        );
    }

    // Rule: without the head it started from, nothing can be said of what lies between the heads, and the tree is still read.
    #[tokio::test]
    async fn without_the_head_it_started_from_only_the_uncommitted_files_are_known() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let head = commit_at(&repo, "start", 1_700_000_000);
        write(&repo, "left.txt", b"x\n");

        let changes = handle(&repo).work_changes(None).await.unwrap();

        assert_eq!(changes.head_after, head);
        assert_eq!((changes.commits, changes.files), (vec![], vec![]));
        assert_eq!(changes.uncommitted, [change("left.txt", Change::Added, text(1, 0))]);
    }

    // Rule: a head that went back has no commits after the old one, and the files that differ are what the reset undid.
    #[tokio::test]
    async fn a_head_that_went_back_lists_no_commits_and_the_files_the_reset_undid() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let first = commit_at(&repo, "first", 1_700_000_000);
        write(&repo, "b.txt", b"two\n");
        let second = commit_at(&repo, "second", 1_700_000_100);
        raw(&repo, &["reset", "-q", "--hard", &first.to_string()]);

        let changes = handle(&repo).work_changes(Some(&second)).await.unwrap();

        assert_eq!(changes.head_after, first);
        assert_eq!(changes.commits, []);
        assert_eq!(changes.files, [change("b.txt", Change::Deleted, text(0, 1))]);
    }
    #[tokio::test]
    async fn history_reads_raw_worktree_bytes_without_running_configured_filters() {
        for driver in ["clean", "process"] {
            let (tmp, repo) = repository();
            write(&repo, ".gitattributes", b"*.txt filter=fixture\n");
            commit_at(&repo, "attributes", 1_700_000_000);
            let marker = tmp.path().join("filter-called");
            let command = format!("printf called > '{}'; cat", marker.display());
            raw(&repo, &["config", &format!("filter.fixture.{driver}"), &command]);
            raw(&repo, &["config", "filter.fixture.required", "true"]);
            write(&repo, "new.txt", b"raw\nbytes\n");

            let result = handle(&repo).uncommitted_changes().await;

            assert!(!marker.exists(), "history invoked the configured {driver} filter");
            assert_eq!(result.unwrap(), [change("new.txt", Change::Added, text(2, 0))]);
        }
    }

    #[tokio::test]
    async fn history_reads_do_not_invoke_a_configured_filesystem_monitor() {
        use std::os::unix::fs::PermissionsExt as _;

        let (tmp, repo) = repository();
        commit_at(&repo, "start", 1_700_000_000);
        let marker = tmp.path().join("monitor-called");
        let monitor = tmp.path().join("monitor.sh");
        std::fs::write(&monitor, format!("#!/bin/sh\nprintf called > '{}'\n", marker.display())).unwrap();
        std::fs::set_permissions(&monitor, std::fs::Permissions::from_mode(0o700)).unwrap();
        raw(&repo, &["config", "core.fsmonitor", monitor.to_str().unwrap()]);
        write(&repo, "new.txt", b"raw\n");

        let changes = handle(&repo).uncommitted_changes().await.unwrap();

        assert!(!marker.exists(), "history invoked the configured filesystem monitor");
        assert_eq!(changes, [change("new.txt", Change::Added, text(1, 0))]);
    }
}
