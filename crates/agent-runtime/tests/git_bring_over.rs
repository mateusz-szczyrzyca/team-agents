//! A worker's commit against the requester's worktree: the files it changes, the ones outside the request's, and
//! bringing it to the requester's worktree after a snapshot without touching uncommitted work. Real repositories in
//! temporary directories, isolated from the configuration of the machine.

#[cfg(test)]
mod git_bring_over {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::ids::{RoundId, TaskId};
    use agent_core::repo_path::RepoPath;
    use agent_core::state::CommitId;
    use agent_runtime::git::{BringOver, CommitIdentity, Git, GitError, Merged};

    /// The round and the task of the snapshots of these tests.
    const ROUND: &str = "round-20261122-140000-b1b1";
    /// See [`ROUND`].
    const TASK: &str = "task-20261122-140000-b1b1";

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

    fn path(name: &str) -> RepoPath {
        RepoPath::from_str(name).expect("a repository path")
    }

    fn paths(names: &[&str]) -> Vec<RepoPath> {
        names.iter().map(|name| path(name)).collect()
    }

    fn round() -> RoundId {
        RoundId::from_str(ROUND).expect("a round id")
    }

    fn task() -> TaskId {
        TaskId::from_str(TASK).expect("a task id")
    }

    fn write(repo: &Path, name: &str, bytes: &[u8]) {
        let file = repo.join(name);
        std::fs::create_dir_all(file.parent().expect("a parent")).expect("a directory");
        std::fs::write(file, bytes).expect("a file");
    }

    /// An empty repository on `main` in a temporary directory.
    fn repository() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let repo = tmp.path().canonicalize().expect("a canonical path").join("repo");
        std::fs::create_dir(&repo).expect("the repository directory");
        raw(&repo, &["init", "-q", "-b", "main"]);
        (tmp, repo)
    }

    /// A commit on the repository's current branch that changes `files`, as the requester's own work leaves it.
    fn commit(repo: &Path, files: &[(&str, &[u8])], message: &str) -> CommitId {
        for (name, bytes) in files {
            write(repo, name, bytes);
        }
        raw(repo, &["add", "-A"]);
        raw(repo, &["commit", "-q", "-m", message]);
        raw(repo, &["rev-parse", "HEAD"]).parse().expect("a commit")
    }

    /// The state a bringing over must not change: git's view of the worktree, the bytes of the index and of the named
    /// files.
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

    #[tokio::test]
    async fn the_files_a_commit_changes_are_named_with_a_rename_as_both_paths() {
        let (_tmp, repo) = repository();
        commit(
            &repo,
            &[
                ("kept.txt", b"kept\n"),
                ("before.txt", b"one\ntwo\n"),
                ("gone.txt", b"gone\n"),
            ],
            "base",
        );
        // The worker's commit is on a branch of its own: the files it changes are those since the requester's head.
        raw(&repo, &["switch", "-q", "-c", "worker"]);
        raw(&repo, &["mv", "before.txt", "after.txt"]);
        std::fs::remove_file(repo.join("gone.txt")).expect("the deletion");
        write(&repo, "new.txt", b"new\n");
        let worker = commit(&repo, &[], "the worker's step");
        raw(&repo, &["switch", "-q", "main"]);

        let changed = handle(&repo)
            .changed_in(&worker)
            .await
            .expect("the files of the commit");

        assert_eq!(changed, paths(&["after.txt", "before.txt", "gone.txt", "new.txt"]));
    }

    #[tokio::test]
    async fn a_commit_of_two_files_of_which_one_is_outside_the_request_names_it() {
        let (_tmp, repo) = repository();
        commit(&repo, &[("guide.md", b"before\n")], "base");
        raw(&repo, &["switch", "-q", "-c", "worker"]);
        let worker = commit(
            &repo,
            &[("guide.md", b"after\n"), ("notes.md", b"after\n")],
            "the worker's step",
        );
        raw(&repo, &["switch", "-q", "main"]);

        let outside = handle(&repo)
            .outside(&worker, &paths(&["guide.md"]))
            .await
            .expect("the files outside the request");

        assert_eq!(outside, paths(&["notes.md"]));
    }

    #[tokio::test]
    async fn a_commit_that_applies_reaches_the_head_and_leaves_uncommitted_and_untracked_files_as_they_were() {
        let (_tmp, repo) = repository();
        commit(&repo, &[("guide.md", b"one\n"), ("other.txt", b"other\n")], "base");
        let files = ["guide.md", "other.txt", "uncommitted.txt", "untracked.txt"];
        // The worker's commit, made on a branch of the same repository from the base the requester is at, before the
        // requester writes its own uncommitted and untracked files: a `git add -A` in the worker's run must not take
        // them into the worker's commit.
        let base = raw(&repo, &["rev-parse", "HEAD"]);
        raw(&repo, &["switch", "-q", "-c", "worker", &base]);
        commit(&repo, &[("guide.md", b"one\ntwo\n")], "the worker's step");
        let worker: CommitId = raw(&repo, &["rev-parse", "HEAD"]).parse().expect("a commit");
        raw(&repo, &["switch", "-q", "main"]);
        write(&repo, "uncommitted.txt", b"mine\n");
        write(&repo, "untracked.txt", b"new\n");
        // The requester's state, as a bringing over must leave it: its files, its index and its untracked files.
        let before = state(&repo, &files);
        let before_head = raw(&repo, &["rev-parse", "HEAD"]);

        let outcome = handle(&repo)
            .bring_over(&worker, &round(), &task())
            .await
            .expect("the commit applies");

        let BringOver::Applied { snapshot, commit } = outcome else {
            panic!("the commit applies: {outcome:?}");
        };
        assert_eq!(raw(&repo, &["show", &format!("{commit}:guide.md")]), "one\ntwo");
        // The snapshot is the requester's worktree as it was, at the head the requester had before the merge.
        assert_eq!(snapshot.head.to_string(), before_head);
        assert_eq!(raw(&repo, &["rev-parse", &format!("{commit}^")]), before_head);
        // The file's bytes, against the blob's bytes: `raw` cuts the last line break of what it prints.
        let blob = Command::new("git")
            .args(["cat-file", "blob", &format!("{commit}:guide.md")])
            .current_dir(&repo)
            .output()
            .expect("git runs");
        assert!(blob.status.success());
        assert_eq!(std::fs::read(repo.join("guide.md")).expect("the file"), blob.stdout);
        // The requester's own files byte for byte, as they were written.
        assert_eq!(std::fs::read(repo.join("uncommitted.txt")).expect("mine"), b"mine\n");
        assert_eq!(std::fs::read(repo.join("untracked.txt")).expect("new"), b"new\n");
        let (status, _index, contents) = state(&repo, &files);
        assert_eq!(&contents[2], b"mine\n");
        assert_eq!(&contents[3], b"new\n");
        assert_eq!(status, before.0);
    }

    #[tokio::test]
    async fn a_conflicting_commit_applies_nothing_and_leaves_the_worktree_as_before() {
        let (_tmp, repo) = repository();
        commit(&repo, &[("guide.md", b"one\n")], "base");
        write(&repo, "uncommitted.txt", b"mine\n");
        let files = ["guide.md", "uncommitted.txt"];
        // The worker's commit, from the base the requester is at, which the requester's own step then moves.
        let base = raw(&repo, &["rev-parse", "HEAD"]);
        raw(&repo, &["switch", "-q", "-c", "worker", &base]);
        commit(&repo, &[("guide.md", b"worker\n")], "the worker's step");
        let worker: CommitId = raw(&repo, &["rev-parse", "HEAD"]).parse().expect("a commit");
        raw(&repo, &["switch", "-q", "main"]);
        commit(&repo, &[("guide.md", b"requester\n")], "the requester's own step");
        let before_head = raw(&repo, &["rev-parse", "HEAD"]);
        // The requester's state, as a conflict must leave it: its files, its index and its uncommitted file.
        let before = state(&repo, &files);

        let outcome = handle(&repo)
            .bring_over(&worker, &round(), &task())
            .await
            .expect("a conflict is an answer");

        let BringOver::Conflicted {
            snapshot,
            worker: returned,
            files: conflicts,
        } = outcome
        else {
            panic!("the commit conflicts: {outcome:?}");
        };
        assert_eq!(returned, worker);
        assert_eq!(snapshot.head.to_string(), before_head);
        assert_eq!(conflicts, paths(&["guide.md"]));
        assert_eq!(state(&repo, &files), before);
        assert_eq!(raw(&repo, &["rev-parse", "HEAD"]), before_head);
        assert_eq!(raw(&repo, &["show", "HEAD:guide.md"]), "requester");
    }

    #[tokio::test]
    async fn a_commit_that_changes_a_file_the_requester_changed_uncommitted_is_refused_and_writes_nothing() {
        let (_tmp, repo) = repository();
        commit(&repo, &[("guide.md", b"one\n"), ("other.txt", b"other\n")], "base");
        let base = raw(&repo, &["rev-parse", "HEAD"]);
        raw(&repo, &["switch", "-q", "-c", "worker", &base]);
        commit(&repo, &[("guide.md", b"one\ntwo\n")], "the worker's step");
        let worker: CommitId = raw(&repo, &["rev-parse", "HEAD"]).parse().expect("a commit");
        raw(&repo, &["switch", "-q", "main"]);
        // The requester is in the middle of its own edit of the file the worker's commit changes, and has an untracked
        // file: the merge applies to the branch, but writing it would lose the edit.
        write(&repo, "guide.md", b"one\nmy edit\n");
        write(&repo, "untracked.txt", b"new\n");
        let files = ["guide.md", "other.txt", "untracked.txt"];
        let before_head = raw(&repo, &["rev-parse", "HEAD"]);
        let before = state(&repo, &files);

        let outcome = handle(&repo)
            .bring_over(&worker, &round(), &task())
            .await
            .expect("a requester's edit in the way is an answer");

        let BringOver::InTheWay {
            worker: returned,
            files: in_the_way,
            ..
        } = outcome
        else {
            panic!("the requester's edit is in the way: {outcome:?}");
        };
        assert_eq!(returned, worker);
        assert_eq!(in_the_way, paths(&["guide.md"]));
        assert_eq!(state(&repo, &files), before);
        assert_eq!(raw(&repo, &["rev-parse", "HEAD"]), before_head);
        assert_eq!(
            std::fs::read(repo.join("guide.md")).expect("the file"),
            b"one\nmy edit\n"
        );
    }

    /// A repository with a base commit and the worker's branch made from it by `step`; the requester is on `main`.
    fn with_worker(base: &[(&str, &[u8])], step: impl FnOnce(&Path)) -> (tempfile::TempDir, PathBuf, CommitId) {
        let (tmp, repo) = repository();
        commit(&repo, base, "base");
        raw(&repo, &["switch", "-q", "-c", "worker"]);
        step(&repo);
        let worker: CommitId = raw(&repo, &["rev-parse", "HEAD"]).parse().expect("a commit");
        raw(&repo, &["switch", "-q", "main"]);
        (tmp, repo, worker)
    }

    #[tokio::test]
    async fn a_merge_made_on_an_older_head_is_not_written_after_the_requester_committed() {
        let (_tmp, repo, worker) = with_worker(&[("guide.md", b"one\n")], |repo| {
            commit(repo, &[("guide.md", b"one\ntwo\n")], "the worker's step");
        });
        let git = handle(&repo);
        let made_on: CommitId = raw(&repo, &["rev-parse", "HEAD"]).parse().expect("a commit");
        let Merged::Clean(applied) = git
            .merge_into_scratch(&made_on, &worker, &round(), &task())
            .await
            .expect("the merge")
        else {
            panic!("the merge applies");
        };
        // The requester commits while the merge waits to be written.
        let moved = commit(&repo, &[("mine.txt", b"mine\n")], "the requester's own step");
        let before = state(&repo, &["guide.md", "mine.txt"]);

        let error = git
            .write_merged(&made_on, &applied)
            .await
            .expect_err("the branch is not where the merge was made");

        assert!(
            matches!(&error, GitError::HeadMoved { expected, found } if *expected == made_on && *found == moved),
            "{error:?}"
        );
        assert_eq!(raw(&repo, &["rev-parse", "HEAD"]), moved.to_string());
        assert_eq!(state(&repo, &["guide.md", "mine.txt"]), before);
        assert_eq!(std::fs::read(repo.join("mine.txt")).expect("mine"), b"mine\n");
    }

    #[tokio::test]
    async fn the_files_of_a_worker_history_of_two_commits_are_all_named() {
        let (_tmp, repo, worker) = with_worker(&[("guide.md", b"one\n")], |repo| {
            commit(repo, &[("secret.md", b"secret\n")], "the first commit");
            commit(repo, &[("guide.md", b"one\ntwo\n")], "the second commit");
        });

        let outside = handle(&repo)
            .outside(&worker, &paths(&["guide.md"]))
            .await
            .expect("the files outside the request");

        assert_eq!(outside, paths(&["secret.md"]));
    }

    #[tokio::test]
    async fn the_files_of_a_worker_merge_commit_are_named() {
        let (_tmp, repo, worker) = with_worker(&[("guide.md", b"one\n")], |repo| {
            commit(repo, &[("guide.md", b"one\ntwo\n")], "the worker's step");
            raw(repo, &["switch", "-q", "-c", "side", "main"]);
            commit(repo, &[("side.md", b"side\n")], "a side step");
            raw(repo, &["switch", "-q", "worker"]);
            raw(repo, &["merge", "-q", "--no-ff", "-m", "merge the side", "side"]);
        });

        let outside = handle(&repo)
            .outside(&worker, &paths(&["guide.md"]))
            .await
            .expect("the files outside the request");

        assert_eq!(outside, paths(&["side.md"]));
    }

    #[tokio::test]
    async fn a_path_that_is_not_utf8_is_refused_and_nothing_is_written() {
        use std::os::unix::ffi::OsStrExt;
        let (_tmp, repo) = repository();
        commit(&repo, &[("guide.md", b"one\n")], "base");
        let odd = repo.join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt"));
        std::fs::write(&odd, b"latin\n").expect("the file");
        raw(&repo, &["add", "-A"]);
        raw(
            &repo,
            &["commit", "-q", "-m", "the requester's file with a latin-1 name"],
        );
        let base = raw(&repo, &["rev-parse", "HEAD"]);
        raw(&repo, &["switch", "-q", "-c", "worker", &format!("{base}~1")]);
        let worker = commit(&repo, &[("guide.md", b"one\ntwo\n")], "the worker's step");
        raw(&repo, &["switch", "-q", "main"]);
        let before_head = raw(&repo, &["rev-parse", "HEAD"]);

        let error = handle(&repo)
            .bring_over(&worker, &round(), &task())
            .await
            .expect_err("the name is not UTF-8");

        assert!(matches!(error, GitError::NotUtf8 { .. }), "{error:?}");
        assert_eq!(raw(&repo, &["rev-parse", "HEAD"]), before_head);
        assert_eq!(std::fs::read(&odd).expect("the file"), b"latin\n");
        assert_eq!(std::fs::read(repo.join("guide.md")).expect("the file"), b"one\n");
    }

    #[tokio::test]
    async fn a_file_where_the_other_side_has_a_directory_is_a_conflict_in_both_directions() {
        // The worker adds a file in a directory where the requester has a file of that name, and the other way round.
        let (_tmp, repo, worker) = with_worker(&[("guide.md", b"one\n")], |repo| {
            commit(repo, &[("node/child.txt", b"child\n")], "the worker's step");
        });
        commit(&repo, &[("node", b"file\n")], "the requester's own step");
        let before_head = raw(&repo, &["rev-parse", "HEAD"]);
        let outcome = handle(&repo)
            .bring_over(&worker, &round(), &task())
            .await
            .expect("a conflict is an answer");
        let BringOver::Conflicted { files, .. } = outcome else {
            panic!("a file and a directory of one name conflict: {outcome:?}");
        };
        assert_eq!(files, paths(&["node", "node/child.txt"]));
        assert_eq!(raw(&repo, &["rev-parse", "HEAD"]), before_head);

        let (_tmp, repo, worker) = with_worker(&[("guide.md", b"one\n")], |repo| {
            commit(repo, &[("node", b"file\n")], "the worker's step");
        });
        commit(&repo, &[("node/child.txt", b"child\n")], "the requester's own step");
        let outcome = handle(&repo)
            .bring_over(&worker, &round(), &task())
            .await
            .expect("a conflict is an answer");
        let BringOver::Conflicted { files, .. } = outcome else {
            panic!("a file and a directory of one name conflict: {outcome:?}");
        };
        assert_eq!(files, paths(&["node", "node/child.txt"]));
    }

    #[tokio::test]
    async fn a_tracked_edit_and_a_staged_change_that_the_merge_does_not_touch_stay_as_they_were() {
        let (_tmp, repo, worker) = with_worker(
            &[
                ("guide.md", b"one\n"),
                ("other.txt", b"other\n"),
                ("staged.txt", b"staged\n"),
            ],
            |repo| {
                commit(repo, &[("guide.md", b"one\ntwo\n")], "the worker's step");
            },
        );
        write(&repo, "other.txt", b"my edit\n");
        write(&repo, "staged.txt", b"my staged edit\n");
        raw(&repo, &["add", "staged.txt"]);
        let status = raw(&repo, &["status", "--porcelain=v1", "-z"]);
        assert_eq!(status, " M other.txt\0M  staged.txt\0");

        let outcome = handle(&repo)
            .bring_over(&worker, &round(), &task())
            .await
            .expect("the commit applies");

        assert!(matches!(outcome, BringOver::Applied { .. }), "{outcome:?}");
        assert_eq!(raw(&repo, &["status", "--porcelain=v1", "-z"]), status);
        assert_eq!(std::fs::read(repo.join("other.txt")).expect("mine"), b"my edit\n");
        assert_eq!(raw(&repo, &["show", ":staged.txt"]), "my staged edit");
        assert_eq!(std::fs::read(repo.join("guide.md")).expect("the file"), b"one\ntwo\n");
    }

    #[tokio::test]
    async fn a_worker_history_with_no_commit_in_common_is_brought_over() {
        let (_tmp, repo) = repository();
        commit(&repo, &[("guide.md", b"one\n")], "base");
        raw(&repo, &["switch", "-q", "--orphan", "worker"]);
        let worker = commit(&repo, &[("orphan.md", b"orphan\n")], "the worker's own history");
        raw(&repo, &["switch", "-q", "main"]);

        let outcome = handle(&repo)
            .bring_over(&worker, &round(), &task())
            .await
            .expect("the commit applies");

        assert!(matches!(outcome, BringOver::Applied { .. }), "{outcome:?}");
        assert_eq!(std::fs::read(repo.join("orphan.md")).expect("the file"), b"orphan\n");
        assert_eq!(std::fs::read(repo.join("guide.md")).expect("the file"), b"one\n");
        assert_eq!(raw(&repo, &["status", "--porcelain=v1", "-z"]), "");
    }

    #[tokio::test]
    async fn a_file_the_worker_deleted_and_a_mode_it_changed_are_brought_over() {
        let (_tmp, repo, worker) = with_worker(&[("other.txt", b"other\n"), ("run.sh", b"echo\n")], |repo| {
            raw(repo, &["rm", "-q", "other.txt"]);
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(repo.join("run.sh"), std::fs::Permissions::from_mode(0o755))
                    .expect("the mode");
            }
            raw(repo, &["add", "-A"]);
            raw(repo, &["commit", "-q", "-m", "the worker's step"]);
        });

        let outcome = handle(&repo)
            .bring_over(&worker, &round(), &task())
            .await
            .expect("the commit applies");

        assert!(matches!(outcome, BringOver::Applied { .. }), "{outcome:?}");
        assert!(!repo.join("other.txt").exists());
        assert!(raw(&repo, &["ls-tree", "HEAD", "run.sh"]).starts_with("100755 "));
        assert_eq!(raw(&repo, &["status", "--porcelain=v1", "-z"]), "");
    }

    #[tokio::test]
    async fn the_commit_brought_over_keeps_the_workers_author_and_message() {
        let (_tmp, repo, worker) = with_worker(&[("guide.md", b"one\n")], |repo| {
            write(repo, "guide.md", b"one\ntwo\n");
            raw(repo, &["add", "-A"]);
            raw(
                repo,
                &[
                    "commit",
                    "-q",
                    "--author=Worker Name <worker@example.com>",
                    "-m",
                    "Write the second line",
                    "-m",
                    "The body of the worker's message.",
                ],
            );
        });

        let outcome = handle(&repo)
            .bring_over(&worker, &round(), &task())
            .await
            .expect("the commit applies");

        let BringOver::Applied { commit, .. } = outcome else {
            panic!("the commit applies: {outcome:?}");
        };
        let log = raw(&repo, &["log", "-1", "--format=%an <%ae>%n%B", &commit.to_string()]);
        assert!(
            log.starts_with("Worker Name <worker@example.com>\nWrite the second line\n"),
            "{log}"
        );
        assert!(log.contains("The body of the worker's message."), "{log}");
        assert!(log.contains(&worker.to_string()), "{log}");
    }

    #[tokio::test]
    async fn an_untracked_file_in_the_way_of_a_file_the_worker_adds_is_a_typed_outcome() {
        let (_tmp, repo, worker) = with_worker(&[("guide.md", b"one\n")], |repo| {
            commit(repo, &[("notes.md", b"the worker's notes\n")], "the worker's step");
        });
        write(&repo, "notes.md", b"my own notes\n");
        let files = ["guide.md", "notes.md"];
        let before = state(&repo, &files);

        let outcome = handle(&repo)
            .bring_over(&worker, &round(), &task())
            .await
            .expect("an untracked file in the way is an answer");

        let BringOver::InTheWay { files: in_the_way, .. } = outcome else {
            panic!("an untracked file is in the way: {outcome:?}");
        };
        assert_eq!(in_the_way, paths(&["notes.md"]));
        assert_eq!(state(&repo, &files), before);
    }
}
