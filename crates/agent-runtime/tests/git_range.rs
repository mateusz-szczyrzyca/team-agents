//! What git says about a range of commits — the merge base, the files, the commits and the added lines —, read from real
//! repositories in temporary directories, isolated from the configuration of the machine.

#[cfg(test)]
mod git_range {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_core::decide::checks::UnchangedLine;
    use agent_core::decide::checks::{AddedLine, Change, CommitFacts, FileChange, Lines};
    use agent_core::repo_path::RepoPath;
    use agent_core::state::CommitId;
    use agent_runtime::git::{CommitIdentity, Git, GitError, GitLimits, GitOperation, Revision};

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

    fn path(text: &str) -> RepoPath {
        text.parse().unwrap()
    }

    fn head_of(repo: &Path) -> CommitId {
        raw(repo, &["rev-parse", "HEAD"]).parse().unwrap()
    }

    /// Writes `bytes` to `name` (with its directories) in `repo`.
    fn write(repo: &Path, name: &str, bytes: &[u8]) {
        let file = repo.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }

    /// Commits everything that changed in `repo` with `message` and returns the commit.
    fn commit_all(repo: &Path, message: &str) -> CommitId {
        raw(repo, &["add", "-A"]);
        raw(repo, &["commit", "-q", "--allow-empty", "-m", message]);
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

    /// The changes sorted by path: git's own order of a rename is not part of what is promised.
    fn by_path(mut changes: Vec<FileChange>) -> Vec<FileChange> {
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        changes
    }

    fn change(path_text: &str, change: Change, lines: Lines) -> FileChange {
        FileChange {
            path: path(path_text),
            change,
            lines,
        }
    }

    // ---- merge_base ---------------------------------------------------------------------------------------------------

    // Rule: the merge base is the commit where a branch left the one it came from, however far that one went on.
    #[tokio::test]
    async fn the_merge_base_is_where_the_branch_left_its_base() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let start = commit_all(&repo, "start");
        raw(&repo, &["switch", "-q", "-c", "work"]);
        write(&repo, "b.txt", b"two\n");
        commit_all(&repo, "work");
        raw(&repo, &["switch", "-q", "main"]);
        write(&repo, "c.txt", b"three\n");
        commit_all(&repo, "main goes on");

        let base = handle(&repo)
            .merge_base(
                &Revision::Branch("main".parse().unwrap()),
                &Revision::Branch("work".parse().unwrap()),
            )
            .await
            .unwrap();

        assert_eq!(base, start);
    }

    #[tokio::test]
    async fn the_merge_base_of_a_commit_and_its_ancestor_is_the_ancestor() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let first = commit_all(&repo, "first");
        write(&repo, "b.txt", b"two\n");
        let second = commit_all(&repo, "second");

        let base = handle(&repo)
            .merge_base(&Revision::Commit(first.clone()), &Revision::Commit(second))
            .await
            .unwrap();

        assert_eq!(base, first);
    }

    #[tokio::test]
    async fn histories_with_nothing_in_common_have_no_merge_base() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        commit_all(&repo, "first");
        raw(&repo, &["switch", "-q", "--orphan", "other"]);
        write(&repo, "b.txt", b"two\n");
        commit_all(&repo, "unrelated");

        let error = handle(&repo)
            .merge_base(
                &Revision::Branch("main".parse().unwrap()),
                &Revision::Branch("other".parse().unwrap()),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, GitError::NoMergeBase { .. }), "{error:?}");
    }

    #[tokio::test]
    async fn the_merge_base_of_a_revision_that_is_not_there_is_git_failing() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        commit_all(&repo, "first");

        let error = handle(&repo)
            .merge_base(&Revision::Head, &Revision::RemoteBranch("main".parse().unwrap()))
            .await
            .unwrap_err();

        assert!(
            matches!(
                error,
                GitError::Failed {
                    operation: GitOperation::MergeBase,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    // ---- range_changes ------------------------------------------------------------------------------------------------

    // Rule: the files of a range are what differs between its two ends, with the kind of change and the lines it added
    // and took away.
    #[tokio::test]
    async fn the_files_of_a_range_have_their_kind_of_change_and_their_lines() {
        let (_tmp, repo) = repository();
        write(&repo, "tests/a.txt", b"1\n2\n3\n");
        write(&repo, "src/old.rs", b"x\n");
        write(&repo, "docs/keep.md", b"kept\nkept\nkept\nkept\n");
        write(&repo, "img.bin", &[0, 1, 2, 3]);
        write(&repo, "same.txt", b"same\n");
        let base = commit_all(&repo, "base");
        write(&repo, "tests/a.txt", b"1\ntwo\n3\n4\n");
        std::fs::remove_file(repo.join("src/old.rs")).unwrap();
        write(&repo, "src/new.rs", b"a\nb\n");
        std::fs::remove_file(repo.join("docs/keep.md")).unwrap();
        write(&repo, "docs/kept.md", b"kept\nkept\nkept\nkept\n");
        write(&repo, "img.bin", &[0, 9, 9, 9, 9]);
        write(&repo, "dir with space/ü.rs", b"fn a() {}\nfn b() {}\n");
        let head = commit_all(&repo, "head");

        let changes = handle(&repo).range_changes(&base, &head).await.unwrap();

        assert_eq!(
            by_path(changes),
            [
                change(
                    "dir with space/ü.rs",
                    Change::Added,
                    Lines::Text { added: 2, removed: 0 }
                ),
                change(
                    "docs/kept.md",
                    Change::Renamed {
                        from: path("docs/keep.md")
                    },
                    Lines::Text { added: 0, removed: 0 }
                ),
                change("img.bin", Change::Modified, Lines::Binary),
                change("src/new.rs", Change::Added, Lines::Text { added: 2, removed: 0 }),
                change("src/old.rs", Change::Deleted, Lines::Text { added: 0, removed: 1 }),
                change("tests/a.txt", Change::Modified, Lines::Text { added: 2, removed: 1 }),
            ]
        );
    }

    #[tokio::test]
    async fn a_range_with_no_difference_has_no_files() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let base = commit_all(&repo, "base");
        // a commit that changes nothing, and one that changes a file back
        let same = commit_all(&repo, "empty");
        write(&repo, "a.txt", b"two\n");
        commit_all(&repo, "change");
        write(&repo, "a.txt", b"one\n");
        let back = commit_all(&repo, "back");

        assert_eq!(handle(&repo).range_changes(&base, &same).await.unwrap(), []);
        assert_eq!(handle(&repo).range_changes(&base, &back).await.unwrap(), []);
    }

    #[tokio::test]
    async fn a_rename_that_changes_lines_has_them_counted() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"1\n2\n3\n4\n5\n6\n");
        let base = commit_all(&repo, "base");
        std::fs::remove_file(repo.join("a.txt")).unwrap();
        write(&repo, "b.txt", b"1\n2\n3\n4\n5\nsix\n7\n");
        let head = commit_all(&repo, "head");

        let changes = handle(&repo).range_changes(&base, &head).await.unwrap();

        assert_eq!(
            changes,
            [change(
                "b.txt",
                Change::Renamed { from: path("a.txt") },
                Lines::Text { added: 2, removed: 1 }
            )]
        );
    }

    #[tokio::test]
    async fn a_commit_that_is_not_in_the_repository_is_git_failing() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let base = commit_all(&repo, "base");
        let missing: CommitId = "1".repeat(40).parse().unwrap();

        let error = handle(&repo).range_changes(&base, &missing).await.unwrap_err();

        assert!(
            matches!(error, GitError::Failed { .. } | GitError::UnknownRevision { .. }),
            "{error:?}"
        );
    }

    // ---- range_commits ------------------------------------------------------------------------------------------------

    // Rule: the commits of a range are those after the base up to the head, each after its parents, with its parents, its
    // message and the files it changed against its first parent.
    #[tokio::test]
    async fn the_commits_of_a_range_come_after_their_parents_with_their_messages_and_files() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"zero\n");
        let base = commit_all(&repo, "before the range");
        write(&repo, "tests/x.txt", b"a\nb\n");
        write(&repo, "f.rs", b"c\n");
        let red = commit_all(&repo, "RED: first\n\nWhy.\n\nTest-First: not possible — because\n");
        raw(&repo, &["switch", "-q", "-c", "side"]);
        write(&repo, "z.txt", b"z\n");
        let side = commit_all(&repo, "side");
        raw(&repo, &["switch", "-q", "main"]);
        write(&repo, "tests/x.txt", b"a\n");
        std::fs::remove_file(repo.join("f.rs")).unwrap();
        write(&repo, "with space.rs", b"s\n");
        let green = commit_all(&repo, "GREEN: second");
        raw(&repo, &["merge", "-q", "--no-ff", "-m", "Merge side", "side"]);
        let merge = head_of(&repo);
        write(&repo, "last.txt", b"1\n2\n3\n");
        let last = commit_all(&repo, "Last\n\nline one\nline two");

        let commits = handle(&repo).range_commits(&base, &last).await.unwrap();

        let ids: Vec<&CommitId> = commits.iter().map(|commit| &commit.id).collect();
        assert_eq!(ids.len(), 5, "{ids:?}");
        for commit in &commits {
            for parent in &commit.parents {
                let before = ids.iter().position(|id| *id == parent);
                let at = ids.iter().position(|id| **id == commit.id);
                assert!(before < at || before.is_none(), "{parent} comes after {}", commit.id);
            }
        }
        let find = |id: &CommitId| -> &CommitFacts { commits.iter().find(|commit| commit.id == *id).unwrap() };
        let red_facts = find(&red);
        assert_eq!(red_facts.parents, std::slice::from_ref(&base));
        assert_eq!(red_facts.subject, "RED: first");
        assert_eq!(red_facts.body, "Why.\n\nTest-First: not possible — because");
        assert_eq!(
            red_facts.files,
            [
                change("f.rs", Change::Added, Lines::Text { added: 1, removed: 0 }),
                change("tests/x.txt", Change::Added, Lines::Text { added: 2, removed: 0 }),
            ]
        );
        let green_facts = find(&green);
        assert_eq!(green_facts.parents, std::slice::from_ref(&red));
        assert_eq!(green_facts.subject, "GREEN: second");
        assert_eq!(green_facts.body, "");
        assert_eq!(
            green_facts.files,
            [
                change("f.rs", Change::Deleted, Lines::Text { added: 0, removed: 1 }),
                change("tests/x.txt", Change::Modified, Lines::Text { added: 0, removed: 1 }),
                change("with space.rs", Change::Added, Lines::Text { added: 1, removed: 0 }),
            ]
        );
        let merge_facts = find(&merge);
        assert_eq!(merge_facts.parents, [green, side.clone()]);
        assert_eq!(merge_facts.subject, "Merge side");
        assert_eq!(merge_facts.files, []);
        let last_facts = find(&last);
        assert_eq!(last_facts.subject, "Last");
        assert_eq!(last_facts.body, "line one\nline two");
        assert_eq!(
            find(&side).files,
            [change("z.txt", Change::Added, Lines::Text { added: 1, removed: 0 })]
        );
    }

    #[tokio::test]
    async fn the_commits_of_an_empty_range_are_none() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let head = commit_all(&repo, "only");

        assert_eq!(handle(&repo).range_commits(&head, &head).await.unwrap(), []);
    }

    // Rule: a merge lists no files of its own, and the files it changed against each of its parents, in their order: a
    // resolution that drops a line shows against the parent that had it.
    #[tokio::test]
    async fn a_merge_has_the_files_it_changed_against_each_of_its_parents() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"zero\n");
        let base = commit_all(&repo, "before the range");
        raw(&repo, &["switch", "-q", "-c", "side"]);
        write(&repo, "z.txt", b"z\n");
        let side = commit_all(&repo, "side");
        raw(&repo, &["switch", "-q", "main"]);
        write(&repo, "tests/x.txt", b"a\nb\n");
        let red = commit_all(&repo, "RED: x");
        raw(&repo, &["merge", "-q", "--no-ff", "--no-commit", "side"]);
        write(&repo, "tests/x.txt", b"a\n");
        let merge = commit_all(&repo, "Merge side");

        let commits = handle(&repo).range_commits(&base, &merge).await.unwrap();

        let merge_facts = commits.iter().find(|commit| commit.id == merge).unwrap();
        assert_eq!(merge_facts.parents, [red, side]);
        assert_eq!(merge_facts.files, []);
        assert_eq!(
            merge_facts.merge_files,
            [
                vec![
                    change("tests/x.txt", Change::Modified, Lines::Text { added: 0, removed: 1 }),
                    change("z.txt", Change::Added, Lines::Text { added: 1, removed: 0 }),
                ],
                vec![change(
                    "tests/x.txt",
                    Change::Added,
                    Lines::Text { added: 1, removed: 0 }
                )],
            ]
        );
        for commit in commits.iter().filter(|commit| commit.id != merge) {
            assert_eq!(commit.merge_files, Vec::<Vec<FileChange>>::new(), "{}", commit.subject);
        }
    }

    #[tokio::test]
    async fn a_commit_that_renames_a_file_is_a_deletion_and_an_addition() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"1\n2\n3\n");
        let base = commit_all(&repo, "base");
        std::fs::remove_file(repo.join("a.txt")).unwrap();
        write(&repo, "b.txt", b"1\n2\n3\n");
        let head = commit_all(&repo, "move");

        let commits = handle(&repo).range_commits(&base, &head).await.unwrap();

        assert_eq!(
            commits[0].files,
            [
                change("a.txt", Change::Deleted, Lines::Text { added: 0, removed: 3 }),
                change("b.txt", Change::Added, Lines::Text { added: 3, removed: 0 }),
            ]
        );
    }

    #[tokio::test]
    async fn a_message_with_control_characters_and_non_ascii_text_is_read_as_it_is() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let base = commit_all(&repo, "base");
        write(&repo, "b.txt", b"two\n");
        let head = commit_all(&repo, "Zażółć \u{1}gęślą\u{1e}jaźń\n\nBody\u{1f}with\ttab and ąę\n");

        let commits = handle(&repo).range_commits(&base, &head).await.unwrap();

        assert_eq!(commits[0].subject, "Zażółć \u{1}gęślą\u{1e}jaźń");
        assert_eq!(commits[0].body, "Body\u{1f}with\ttab and ąę");
    }

    #[tokio::test]
    async fn a_binary_file_of_a_commit_has_no_line_counts() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let base = commit_all(&repo, "base");
        write(&repo, "img.bin", &[0, 1, 2]);
        let head = commit_all(&repo, "image");

        let commits = handle(&repo).range_commits(&base, &head).await.unwrap();

        assert_eq!(commits[0].files, [change("img.bin", Change::Added, Lines::Binary)]);
    }

    // Rule: a range with more commits than the limit is not read.
    #[tokio::test]
    async fn a_range_with_more_commits_than_the_limit_is_an_error_and_one_of_the_limit_is_not() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let base = commit_all(&repo, "base");
        commit_all(&repo, "one");
        commit_all(&repo, "two");
        let head = commit_all(&repo, "three");
        let limits = |max_commits| GitLimits {
            max_commits,
            ..GitLimits::default()
        };

        let fits = handle(&repo)
            .with_limits(limits(3))
            .range_commits(&base, &head)
            .await
            .unwrap();
        let error = handle(&repo)
            .with_limits(limits(2))
            .range_commits(&base, &head)
            .await
            .unwrap_err();

        assert_eq!(fits.len(), 3);
        assert!(matches!(error, GitError::TooManyCommits { limit: 2 }), "{error:?}");
    }

    // ---- range_added_lines --------------------------------------------------------------------------------------------

    fn added_line(path_text: &str, number: u32, text: &str) -> AddedLine {
        AddedLine {
            path: path(path_text),
            number,
            text: text.to_owned(),
        }
    }

    // Rule: the added lines of a file are those of the hunks of its diff, numbered as in the file at the head.
    #[tokio::test]
    async fn the_added_lines_of_a_file_have_their_numbers_in_the_file_at_the_head() {
        let (_tmp, repo) = repository();
        write(&repo, "src/a.rs", b"one\ntwo\nthree\nfour\nfive\nsix\nseven\n");
        let base = commit_all(&repo, "base");
        write(
            &repo,
            "src/a.rs",
            b"one\nONE\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n++x\n",
        );
        let head = commit_all(&repo, "head");

        let lines = handle(&repo)
            .range_added_lines(&base, &head, &[path("src/a.rs")])
            .await
            .unwrap();

        assert_eq!(
            lines,
            [
                added_line("src/a.rs", 2, "ONE"),
                added_line("src/a.rs", 9, "eight"),
                added_line("src/a.rs", 10, "++x"),
            ]
        );
    }

    #[tokio::test]
    async fn a_replaced_line_is_an_added_line_and_a_removed_one_is_not() {
        let (_tmp, repo) = repository();
        write(&repo, "a.rs", b"one\ntwo\nthree\n");
        write(&repo, "b.rs", b"one\ntwo\nthree\n");
        let base = commit_all(&repo, "base");
        write(&repo, "a.rs", b"one\n2\nthree\n");
        write(&repo, "b.rs", b"one\nthree\n");
        let head = commit_all(&repo, "head");

        let lines = handle(&repo)
            .range_added_lines(&base, &head, &[path("a.rs"), path("b.rs")])
            .await
            .unwrap();

        assert_eq!(lines, [added_line("a.rs", 2, "2")]);
    }

    #[tokio::test]
    async fn the_lines_of_a_new_file_without_a_final_line_break_are_all_added() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let base = commit_all(&repo, "base");
        write(&repo, "new.rs", b"first\nlast");
        let head = commit_all(&repo, "head");

        let lines = handle(&repo)
            .range_added_lines(&base, &head, &[path("new.rs")])
            .await
            .unwrap();

        assert_eq!(
            lines,
            [added_line("new.rs", 1, "first"), added_line("new.rs", 2, "last")]
        );
    }

    #[tokio::test]
    async fn bytes_that_are_not_utf8_are_replaced_in_the_text_of_a_line() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let base = commit_all(&repo, "base");
        write(&repo, "latin.rs", b"ab\xffcd\nplain\n");
        let head = commit_all(&repo, "head");

        let lines = handle(&repo)
            .range_added_lines(&base, &head, &[path("latin.rs")])
            .await
            .unwrap();

        assert_eq!(
            lines,
            [
                added_line("latin.rs", 1, "ab\u{fffd}cd"),
                added_line("latin.rs", 2, "plain")
            ]
        );
    }

    #[tokio::test]
    async fn a_path_is_taken_as_it_is_and_not_as_a_pattern() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let base = commit_all(&repo, "base");
        write(&repo, "weird[1].rs", b"in weird\n");
        write(&repo, "weird1.rs", b"in plain\n");
        write(&repo, "dir/*.rs", b"star\n");
        write(&repo, "dir/other.rs", b"other\n");
        let head = commit_all(&repo, "head");

        let lines = handle(&repo)
            .range_added_lines(&base, &head, &[path("weird[1].rs"), path("dir/*.rs")])
            .await
            .unwrap();

        assert_eq!(
            lines,
            [
                added_line("weird[1].rs", 1, "in weird"),
                added_line("dir/*.rs", 1, "star")
            ]
        );
    }

    #[tokio::test]
    async fn a_binary_a_deleted_a_missing_and_an_unchanged_file_have_no_added_lines() {
        let (_tmp, repo) = repository();
        write(&repo, "gone.rs", b"x\n");
        write(&repo, "kept.rs", b"y\n");
        let base = commit_all(&repo, "base");
        std::fs::remove_file(repo.join("gone.rs")).unwrap();
        write(&repo, "image.rs", &[0, 1, 2, 3]);
        let head = commit_all(&repo, "head");

        let lines = handle(&repo)
            .range_added_lines(
                &base,
                &head,
                &[path("gone.rs"), path("kept.rs"), path("image.rs"), path("nowhere.rs")],
            )
            .await
            .unwrap();

        assert_eq!(lines, []);
    }

    #[tokio::test]
    async fn no_paths_have_no_added_lines_and_ask_git_nothing() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let head = commit_all(&repo, "base");

        assert_eq!(handle(&repo).range_added_lines(&head, &head, &[]).await.unwrap(), []);
    }

    // ---- round_facts --------------------------------------------------------------------------------------------------

    // Rule: the facts of a round are its files, its commits, the added lines of the files whose language has markers and
    // the description of its pull request.
    #[tokio::test]
    async fn the_facts_of_a_round_have_its_files_commits_added_lines_and_description() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        write(&repo, "gone.rs", b"x\n");
        let base = commit_all(&repo, "base");
        write(&repo, "src/lib.rs", b"fn a() {}\n#[allow(dead_code)]\nfn b() {}\n");
        write(&repo, "notes.md", b"#[ignore] is only text here\n");
        write(&repo, "data.json", b"{}\n");
        std::fs::remove_file(repo.join("gone.rs")).unwrap();
        let head = commit_all(&repo, "head");

        let facts = handle(&repo)
            .round_facts(&base, &head, "## Gate\npassed\n")
            .await
            .unwrap();

        assert_eq!(facts.description, "## Gate\npassed\n");
        assert_eq!(
            by_path(facts.files),
            [
                change("data.json", Change::Added, Lines::Text { added: 1, removed: 0 }),
                change("gone.rs", Change::Deleted, Lines::Text { added: 0, removed: 1 }),
                change("notes.md", Change::Added, Lines::Text { added: 1, removed: 0 }),
                change("src/lib.rs", Change::Added, Lines::Text { added: 3, removed: 0 }),
            ]
        );
        assert_eq!(facts.commits.len(), 1);
        assert_eq!(facts.commits[0].id, head);
        // only a file whose extension has markers is read for lines: not the document, not the data
        assert_eq!(
            facts.added,
            [
                added_line("src/lib.rs", 1, "fn a() {}"),
                added_line("src/lib.rs", 2, "#[allow(dead_code)]"),
                added_line("src/lib.rs", 3, "fn b() {}"),
            ]
        );
    }

    #[tokio::test]
    async fn the_facts_of_a_round_with_no_difference_are_empty_but_for_the_description() {
        let (_tmp, repo) = repository();
        write(&repo, "a.txt", b"one\n");
        let head = commit_all(&repo, "base");

        let facts = handle(&repo).round_facts(&head, &head, "text").await.unwrap();

        assert_eq!(facts.files, []);
        assert_eq!(facts.commits, []);
        assert_eq!(facts.added, []);
        assert_eq!(facts.description, "text");
    }

    /// Sets the permission bits of `name` in `repo` to `mode`.
    fn chmod(repo: &Path, name: &str, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(repo.join(name), std::fs::Permissions::from_mode(mode)).unwrap();
    }

    // Rule: the facts of a round name the files whose mode it changed, with lines added or not, by their path at the head;
    // a file that only gained lines, a new file and a moved one that kept its mode are not among them.
    #[tokio::test]
    async fn the_facts_of_a_round_name_the_files_whose_mode_changed() {
        let (_tmp, repo) = repository();
        write(&repo, "tests/run.sh", b"#!/bin/sh\n");
        chmod(&repo, "tests/run.sh", 0o755);
        write(&repo, "tests/plain.sh", b"#!/bin/sh\n");
        write(&repo, "tests/bare.sh", b"#!/bin/sh\n");
        write(&repo, "tests/old.sh", b"#!/bin/sh\necho one\necho two\n");
        let base = commit_all(&repo, "base");
        write(&repo, "tests/run.sh", b"#!/bin/sh\n# still a test\n");
        chmod(&repo, "tests/run.sh", 0o644);
        write(&repo, "tests/plain.sh", b"#!/bin/sh\necho more\n");
        chmod(&repo, "tests/bare.sh", 0o755);
        std::fs::rename(repo.join("tests/old.sh"), repo.join("tests/new.sh")).unwrap();
        write(&repo, "tests/added.sh", b"#!/bin/sh\n");
        chmod(&repo, "tests/added.sh", 0o755);
        let head = commit_all(&repo, "change modes");

        let facts = handle(&repo).round_facts(&base, &head, "").await.unwrap();

        let mut changed = facts.mode_changed.clone();
        changed.sort();
        assert_eq!(changed, [path("tests/bare.sh"), path("tests/run.sh")]);
    }

    // ---- range_unchanged_lines ----------------------------------------------------------------------------------------

    fn unchanged_line(path_text: &str, number: u32, text: &str) -> UnchangedLine {
        UnchangedLine {
            path: path(path_text),
            number,
            text: text.to_owned(),
        }
    }

    /// The lines `l1` to `l<count>` of a file, one on each line.
    fn numbered(count: u32) -> String {
        let lines: Vec<String> = (1..=count).map(|n| format!("l{n}")).collect();
        format!("{}\n", lines.join("\n"))
    }

    // Rule: the unchanged lines of a file are the ten lines before and after each changed place, numbered as in the file at
    // the head; the lines that were added are not among them.
    #[tokio::test]
    async fn the_unchanged_lines_are_ten_before_and_ten_after_a_change() {
        let (_tmp, repo) = repository();
        write(&repo, "src/a.rs", numbered(40).as_bytes());
        let base = commit_all(&repo, "base");
        write(&repo, "src/a.rs", numbered(40).replace("l20\n", "NEW\n").as_bytes());
        let head = commit_all(&repo, "head");

        let lines = handle(&repo)
            .range_unchanged_lines(&base, &head, &[path("src/a.rs")])
            .await
            .unwrap();

        let expected: Vec<UnchangedLine> = (10..=19)
            .chain(21..=30)
            .map(|n| unchanged_line("src/a.rs", n, &format!("l{n}")))
            .collect();
        assert_eq!(lines, expected);
    }

    // Rule: the numbers are those at the head, after lines were added before; the lines between two changes are unchanged.
    #[tokio::test]
    async fn the_unchanged_lines_are_numbered_at_the_head_and_include_those_between_changes() {
        let (_tmp, repo) = repository();
        write(&repo, "a.rs", b"one\ntwo\nthree\n");
        write(&repo, "b.rs", b"one\n");
        let base = commit_all(&repo, "base");
        write(&repo, "a.rs", b"NEW\none\ntwo\nNEWER\nthree\n");
        write(&repo, "b.rs", b"one\ntwo\n");
        let head = commit_all(&repo, "head");

        let lines = handle(&repo)
            .range_unchanged_lines(&base, &head, &[path("a.rs"), path("b.rs")])
            .await
            .unwrap();

        assert_eq!(
            lines,
            [
                unchanged_line("a.rs", 2, "one"),
                unchanged_line("a.rs", 3, "two"),
                unchanged_line("a.rs", 5, "three"),
                unchanged_line("b.rs", 1, "one"),
            ]
        );
    }

    #[tokio::test]
    async fn a_binary_a_deleted_a_missing_and_an_unchanged_file_have_no_unchanged_lines() {
        let (_tmp, repo) = repository();
        write(&repo, "gone.rs", b"x\n");
        write(&repo, "kept.rs", b"y\n");
        let base = commit_all(&repo, "base");
        std::fs::remove_file(repo.join("gone.rs")).unwrap();
        write(&repo, "image.rs", &[0, 1, 2, 3]);
        let head = commit_all(&repo, "head");

        let lines = handle(&repo)
            .range_unchanged_lines(
                &base,
                &head,
                &[path("gone.rs"), path("kept.rs"), path("image.rs"), path("nowhere.rs")],
            )
            .await
            .unwrap();

        assert_eq!(lines, []);
    }

    // Rule: the unchanged lines of a file that was renamed and edited are read against its old path.
    #[tokio::test]
    async fn the_unchanged_lines_of_a_renamed_and_edited_file_are_those_around_its_edit() {
        let (_tmp, repo) = repository();
        write(&repo, "old.rs", b"one\ntwo\nthree\nfour\n");
        let base = commit_all(&repo, "base");
        std::fs::remove_file(repo.join("old.rs")).unwrap();
        write(&repo, "new.rs", b"one\ntwo\nTHREE\nfour\n");
        let head = commit_all(&repo, "head");

        let lines = handle(&repo)
            .range_unchanged_lines(&base, &head, &[path("new.rs")])
            .await
            .unwrap();

        assert_eq!(
            lines,
            [
                unchanged_line("new.rs", 1, "one"),
                unchanged_line("new.rs", 2, "two"),
                unchanged_line("new.rs", 4, "four"),
            ]
        );
    }
}
