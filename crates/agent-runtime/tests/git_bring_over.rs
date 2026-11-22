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
    use agent_runtime::git::{BringOver, CommitIdentity, Git};

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
        raw(&repo, &["mv", "before.txt", "after.txt"]);
        std::fs::remove_file(repo.join("gone.txt")).expect("the deletion");
        write(&repo, "new.txt", b"new\n");
        let worker = commit(&repo, &[], "the worker's step");

        let changed = handle(&repo)
            .changed_in(&worker)
            .await
            .expect("the files of the commit");

        assert_eq!(changed, paths(&["after.txt", "before.txt", "gone.txt", "new.txt"]));
    }

    #[tokio::test]
    async fn a_commit_of_two_files_of_which_one_is_outside_the_request_names_it() {
        let (_tmp, repo) = repository();
        let worker = commit(
            &repo,
            &[("guide.md", b"after\n"), ("notes.md", b"after\n")],
            "the worker's step",
        );

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

        let error = handle(&repo)
            .bring_over(&worker, &round(), &task())
            .await
            .expect_err("the requester's edit is in the way");

        assert!(
            matches!(error, agent_runtime::git::GitError::Failed { .. }),
            "{error:?}"
        );
        assert_eq!(state(&repo, &files), before);
        assert_eq!(raw(&repo, &["rev-parse", "HEAD"]), before_head);
        assert_eq!(
            std::fs::read(repo.join("guide.md")).expect("the file"),
            b"one\nmy edit\n"
        );
    }
}
