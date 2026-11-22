//! The check of a commit about to be made, over real repositories in temporary directories, isolated from the
//! configuration of the machine: a commit that takes lines out of a test its `RED:` commit froze is refused unless it is
//! a `TEST-FIX:` of test files only, and the check changes nothing.

#[cfg(test)]
mod commit_guard {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::{AbsolutePath, Checks};
    use agent_core::decide::checks::{Change, Lines};
    use agent_core::decide::commit_guard::{Refusal, Taken, Verdict};
    use agent_core::ids::BranchName;
    use agent_core::repo_path::{PathPattern, PathPatterns, RepoPath};
    use agent_core::state::CommitId;
    use agent_runtime::commit_guard::{CommitKind, GuardError, GuardStep, Judged, Operation, judge_staged};
    use agent_runtime::git::{CommitIdentity, Git, StagedAgainst};

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

    fn write(repo: &Path, name: &str, text: &str) {
        let path = repo.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn commit(repo: &Path, subject: &str) -> CommitId {
        raw(repo, &["add", "-A"]);
        raw(repo, &["commit", "-q", "-m", subject]);
        raw(repo, &["rev-parse", "HEAD"]).parse().unwrap()
    }

    fn rules() -> Checks {
        Checks {
            test_paths: PathPatterns::try_from(vec!["tests/".parse::<PathPattern>().unwrap()]).unwrap(),
            ..Checks::default()
        }
    }

    fn base() -> BranchName {
        "main".parse().unwrap()
    }

    /// A branch `feature` that left `main` (also the remote-tracking `origin/main`) with a `RED:` commit that added
    /// `tests/a.rs` and a `GREEN:` commit after it.
    struct Round {
        /// Keeps the directory.
        _tmp: tempfile::TempDir,
        repo: PathBuf,
        red: CommitId,
    }

    fn round() -> Round {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir(&repo).unwrap();
        raw(&repo, &["init", "-q", "-b", "main"]);
        write(&repo, "README.md", "readme\n");
        commit(&repo, "Start");
        raw(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        raw(&repo, &["switch", "-q", "-c", "feature"]);
        write(&repo, "tests/a.rs", "one\ntwo\nthree\n");
        write(&repo, "src/a.rs", "stub\n");
        let red = commit(&repo, "RED: parse a line");
        write(&repo, "src/a.rs", "code\n");
        commit(&repo, "GREEN: parse a line");
        Round { _tmp: tmp, repo, red }
    }

    async fn judged(round: &Round, subject: &str, kind: CommitKind) -> Result<Judged, GuardError> {
        judge_staged(&handle(&round.repo), &rules(), &base(), subject, kind).await
    }

    fn refused_for_a(red: &CommitId, taken: Taken) -> Judged {
        Judged::Verdict(Verdict::Refused(vec![Refusal::FrozenTestEdited {
            path: "tests/a.rs".parse().unwrap(),
            red: red.clone(),
            taken,
        }]))
    }

    fn allowed() -> Judged {
        Judged::Verdict(Verdict::Allowed)
    }

    // Rule: a commit that takes a line out of a test its RED commit froze is refused, and names the RED commit.
    #[tokio::test]
    async fn a_commit_that_edits_a_frozen_test_is_refused_with_its_red_commit() {
        let round = round();
        write(&round.repo, "tests/a.rs", "one\nTWO\nthree\n");
        raw(&round.repo, &["add", "-A"]);

        let got = judged(&round, "Format the test", CommitKind::New).await.unwrap();

        assert_eq!(got, refused_for_a(&round.red, Taken::Lines(1)));
    }

    // Rule: the same change as a TEST-FIX commit of test files only passes.
    #[tokio::test]
    async fn the_same_change_as_a_test_fix_commit_passes() {
        let round = round();
        write(&round.repo, "tests/a.rs", "one\nTWO\nthree\n");
        raw(&round.repo, &["add", "-A"]);

        let got = judged(&round, "TEST-FIX: the case named the wrong line", CommitKind::New)
            .await
            .unwrap();

        assert_eq!(got, allowed());
    }

    // Rule: a commit that only adds tests passes.
    #[tokio::test]
    async fn a_commit_that_only_adds_tests_passes() {
        let round = round();
        write(&round.repo, "tests/a.rs", "one\ntwo\nthree\nfour\n");
        write(&round.repo, "tests/b.rs", "new\n");
        raw(&round.repo, &["add", "-A"]);

        let got = judged(&round, "Cover one more case", CommitKind::New).await.unwrap();

        assert_eq!(got, allowed());
    }

    // Rule: an amend replaces HEAD, so it is judged against the parent of HEAD: removing the line that the commit being
    // amended added is no edit of a frozen test, while the same staged state in a new commit is.
    #[tokio::test]
    async fn an_amend_is_judged_against_the_parent_of_head() {
        let round = round();
        write(&round.repo, "tests/a.rs", "one\ntwo\nthree\nfour\n");
        let extra = commit(&round.repo, "Cover one more case");
        write(&round.repo, "tests/a.rs", "one\ntwo\nthree\n");
        raw(&round.repo, &["add", "-A"]);

        let amended = judged(&round, "Cover one more case", CommitKind::Amend).await.unwrap();
        let added = judged(&round, "Take the case away", CommitKind::New).await.unwrap();

        assert_eq!(amended, allowed());
        assert_eq!(added, refused_for_a(&round.red, Taken::Lines(1)));
        assert_eq!(raw(&round.repo, &["rev-parse", "HEAD"]), extra.to_string());
    }

    // Rule: amending the RED commit itself is fine: nothing is frozen yet at its parent.
    #[tokio::test]
    async fn the_red_commit_itself_may_be_amended() {
        let round = round();
        raw(&round.repo, &["reset", "-q", "--hard", &round.red.to_string()]);
        write(&round.repo, "tests/a.rs", "uno\ndos\ntres\n");
        raw(&round.repo, &["add", "-A"]);

        let got = judged(&round, "RED: parse a line", CommitKind::Amend).await.unwrap();

        assert_eq!(got, allowed());
    }

    // Rule: a commit that a merge, a cherry-pick, a revert or a rebase makes is not judged here.
    #[tokio::test]
    async fn the_commit_of_a_merge_in_progress_is_not_judged() {
        let round = round();
        raw(&round.repo, &["switch", "-q", "-c", "other", "main"]);
        write(&round.repo, "other.md", "other\n");
        commit(&round.repo, "Other work");
        raw(&round.repo, &["switch", "-q", "feature"]);
        raw(&round.repo, &["merge", "-q", "--no-commit", "--no-ff", "other"]);
        write(&round.repo, "tests/a.rs", "one\nTWO\nthree\n");
        raw(&round.repo, &["add", "-A"]);

        let got = judged(&round, "Merge branch 'other'", CommitKind::New).await.unwrap();

        assert_eq!(got, Judged::NotAuthored(Operation::Merge));
    }

    // Rule: the check changes nothing: not a ref, not the index, not a file, also where the branch is on the remote.
    #[tokio::test]
    async fn the_check_changes_nothing_even_for_a_branch_on_the_remote() {
        let round = round();
        raw(&round.repo, &["update-ref", "refs/remotes/origin/feature", "HEAD"]);
        write(&round.repo, "tests/a.rs", "one\nTWO\nthree\n");
        raw(&round.repo, &["add", "-A"]);
        let snapshot = |repo: &Path| {
            [
                raw(repo, &["for-each-ref"]),
                raw(repo, &["ls-files", "-s"]),
                raw(repo, &["status", "--porcelain=v1"]),
                std::fs::read_to_string(repo.join("tests/a.rs")).unwrap(),
            ]
        };
        let before = snapshot(&round.repo);

        let got = judged(&round, "Format the test", CommitKind::New).await.unwrap();

        assert_eq!(got, refused_for_a(&round.red, Taken::Lines(1)));
        assert_eq!(snapshot(&round.repo), before);
    }

    // Rule: a branch that cannot be placed against its base cannot be judged, and the error says which step failed.
    #[tokio::test]
    async fn a_missing_base_is_an_error_of_the_merge_base_step() {
        let round = round();
        raw(&round.repo, &["update-ref", "-d", "refs/remotes/origin/main"]);
        write(&round.repo, "tests/a.rs", "one\nTWO\nthree\n");
        raw(&round.repo, &["add", "-A"]);

        let got = judged(&round, "Format the test", CommitKind::New).await;

        assert!(matches!(
            got,
            Err(GuardError::Git {
                step: GuardStep::MergeBase,
                ..
            })
        ));
    }

    // Rule: the staged changes are what the commit would change against its parent: a rename is a deletion and an
    // addition, a binary file has no lines.
    #[tokio::test]
    async fn the_staged_changes_list_files_with_their_lines_against_a_commit() {
        let round = round();
        let head: CommitId = raw(&round.repo, &["rev-parse", "HEAD"]).parse().unwrap();
        write(&round.repo, "tests/a.rs", "one\nTWO\nthree\nfour\n");
        raw(&round.repo, &["mv", "src/a.rs", "src/b.rs"]);
        std::fs::write(round.repo.join("logo.bin"), [0_u8, 159, 146, 150]).unwrap();
        raw(&round.repo, &["add", "-A"]);

        let from_head = handle(&round.repo)
            .staged_changes(&StagedAgainst::Commit(head))
            .await
            .unwrap();
        let from_nothing = handle(&round.repo)
            .staged_changes(&StagedAgainst::EmptyTree)
            .await
            .unwrap();

        let path = |text: &str| -> RepoPath { text.parse().unwrap() };
        let find = |files: &[agent_core::decide::checks::FileChange], name: &str| {
            files.iter().find(|file| file.path == path(name)).cloned().unwrap()
        };
        assert_eq!(find(&from_head, "tests/a.rs").change, Change::Modified);
        assert_eq!(
            find(&from_head, "tests/a.rs").lines,
            Lines::Text { added: 2, removed: 1 }
        );
        assert_eq!(find(&from_head, "src/a.rs").change, Change::Deleted);
        assert_eq!(find(&from_head, "src/b.rs").change, Change::Added);
        assert_eq!(find(&from_head, "logo.bin").lines, Lines::Binary);
        assert_eq!(from_head.len(), 4);
        assert_eq!(find(&from_nothing, "README.md").change, Change::Added);
        assert_eq!(
            find(&from_nothing, "tests/a.rs").lines,
            Lines::Text { added: 4, removed: 0 }
        );
    }
}
