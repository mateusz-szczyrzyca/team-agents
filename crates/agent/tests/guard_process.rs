//! `agent guard-commit` as git runs it: the `commit-msg` hook the harness puts into the worktree of a task, with the
//! real `agent` binary and real git in a temporary home. A commit that takes lines out of a test its `RED:` commit froze
//! is refused with the rule and what to do; the same change as a `TEST-FIX:` commit, a commit that only adds tests and
//! an amend of a local commit pass; nothing is judged outside a task's worktree.

#[cfg(test)]
mod guard_process {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::str::FromStr;

    use agent_core::config::AbsolutePath;
    use agent_runtime::commit_guard::COMMIT_REFUSED_STATUS;
    use agent_runtime::commit_guard::hooks::install_hooks;
    use agent_runtime::git::{CommitIdentity, Git};

    /// The name of the task whose worktree the tests commit in.
    const TASK: &str = "task-20261121-190924-k3x9";

    /// Runs `git` in `dir` as a plain person with no configuration of the machine; `hooks` is `core.hooksPath`.
    fn git_with(dir: &Path, home: &Path, hooks: Option<&Path>, args: &[&str]) -> Output {
        let mut command = Command::new("git");
        command.args(["-c", "commit.gpgsign=false"]);
        if let Some(hooks) = hooks {
            command.arg("-c").arg(format!("core.hooksPath={}", hooks.display()));
        }
        command
            .args(args)
            .current_dir(dir)
            .env("AGENTS_HOME", home)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture Author")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .output()
            .unwrap()
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8(output.stderr.clone()).unwrap()
    }

    /// A home with a repository `example` whose tests are under `tests/`, and the worktree of a task of it on a branch
    /// that left `main`, with the hooks of the agent's runs.
    struct World {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        repo: PathBuf,
        worktree: PathBuf,
        hooks: PathBuf,
    }

    impl World {
        async fn new(checks: bool) -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().canonicalize().unwrap();
            let home = root.join("home");
            std::fs::create_dir(&home).unwrap();
            let mut config = String::from(
                "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n\
                 [repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
            );
            if checks {
                config.push_str("[repos.example.checks]\ntest_paths = [\"tests/\"]\n");
            }
            std::fs::write(home.join("agents.toml"), config).unwrap();
            // The home of an installation has the prompts the harness's texts are read from.
            std::os::unix::fs::symlink(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
                home.join("prompts"),
            )
            .unwrap();
            let repo = root.join("example");
            std::fs::create_dir(&repo).unwrap();
            let run = |dir: &Path, args: &[&str]| {
                let output = git_with(dir, &home, None, args);
                assert!(output.status.success(), "{args:?}: {}", stderr(&output));
            };
            run(&repo, &["init", "-q", "-b", "main"]);
            std::fs::write(repo.join("README.md"), "readme\n").unwrap();
            run(&repo, &["add", "-A"]);
            run(&repo, &["commit", "-q", "-m", "Start"]);
            run(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
            let worktree = home.join("worktrees/example").join(TASK);
            std::fs::create_dir_all(worktree.parent().unwrap()).unwrap();
            run(
                &repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    "feature",
                    worktree.to_str().unwrap(),
                    "main",
                ],
            );
            let git = Git::new(
                AbsolutePath::from_str(worktree.to_str().unwrap()).unwrap(),
                CommitIdentity::new("t".to_owned(), "t@example.com".to_owned(), None),
            );
            let hooks = install_hooks(&git, &root.join("hooks"), Path::new(env!("CARGO_BIN_EXE_agent")))
                .await
                .unwrap();
            Self {
                _tmp: tmp,
                home,
                repo,
                worktree,
                hooks,
            }
        }

        fn git(&self, dir: &Path, args: &[&str]) -> Output {
            git_with(dir, &self.home, Some(&self.hooks), args)
        }

        /// Writes `text` to `name` in the worktree and commits everything with `subject`.
        fn commit(&self, name: &str, text: &str, subject: &str) -> Output {
            let path = self.worktree.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
            assert!(self.git(&self.worktree, &["add", "-A"]).status.success());
            self.git(&self.worktree, &["commit", "-q", "-m", subject])
        }

        /// The first `RED:` commit of the round, which freezes `tests/a.rs`.
        fn red(&self) {
            let output = self.commit("tests/a.rs", "one\ntwo\nthree\n", "RED: parse a line");
            assert!(output.status.success(), "{}", stderr(&output));
        }

        fn count(&self) -> usize {
            let output = self.git(&self.worktree, &["rev-list", "--count", "HEAD"]);
            String::from_utf8(output.stdout).unwrap().trim().parse().unwrap()
        }
    }

    // Rule: a commit that takes lines out of a frozen test, outside a TEST-FIX commit, is refused with the rule and what
    // to do, and what was staged stays staged.
    #[tokio::test]
    async fn a_frozen_test_edited_outside_a_test_fix_is_refused_with_the_rule() {
        let world = World::new(true).await;
        world.red();
        let before = world.count();

        let output = world.commit("tests/a.rs", "one\nTWO\nthree\n", "Format the test");

        assert!(!output.status.success());
        let text = stderr(&output);
        assert!(text.contains("This commit was not made."), "{text}");
        assert!(text.contains("`tests/a.rs`"), "{text}");
        assert!(text.contains("TEST-FIX:"), "{text}");
        assert_eq!(world.count(), before);
        let staged = world.git(&world.worktree, &["diff", "--cached", "--name-only"]);
        assert_eq!(String::from_utf8(staged.stdout).unwrap().trim(), "tests/a.rs");
    }

    // Rule: the same change as a TEST-FIX commit passes.
    #[tokio::test]
    async fn the_same_change_as_a_test_fix_commit_is_made() {
        let world = World::new(true).await;
        world.red();
        let before = world.count();

        let output = world.commit(
            "tests/a.rs",
            "one\nTWO\nthree\n",
            "TEST-FIX: the case named the wrong line",
        );

        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(world.count(), before + 1);
    }

    // Rule: a commit that only adds tests passes.
    #[tokio::test]
    async fn a_commit_that_only_adds_tests_is_made() {
        let world = World::new(true).await;
        world.red();

        let output = world.commit("tests/a.rs", "one\ntwo\nthree\nfour\n", "Cover one more case");

        assert!(output.status.success(), "{}", stderr(&output));
    }

    // Rule: an amend replaces the last commit, so it is judged against the parent of that commit.
    #[tokio::test]
    async fn an_amend_of_a_local_commit_is_judged_against_its_parent() {
        let world = World::new(true).await;
        world.red();
        std::fs::write(world.worktree.join("tests/b.rs"), "new\n").unwrap();
        let added = world.commit("tests/a.rs", "one\ntwo\nthree\nfour\n", "Cover one more case");
        assert!(added.status.success(), "{}", stderr(&added));
        std::fs::write(world.worktree.join("tests/a.rs"), "one\ntwo\nthree\n").unwrap();
        assert!(world.git(&world.worktree, &["add", "-A"]).status.success());

        let amended = world.git(
            &world.worktree,
            &["commit", "-q", "--amend", "-m", "Cover one more case"],
        );

        assert!(amended.status.success(), "{}", stderr(&amended));
    }

    // Rule: only the worktree of a task of the home is judged: the human's own checkout is not.
    #[tokio::test]
    async fn a_commit_outside_the_worktree_of_a_task_is_not_judged() {
        let world = World::new(true).await;
        std::fs::create_dir_all(world.repo.join("tests")).unwrap();
        std::fs::write(world.repo.join("tests/a.rs"), "one\ntwo\n").unwrap();
        assert!(world.git(&world.repo, &["add", "-A"]).status.success());
        assert!(
            world
                .git(&world.repo, &["commit", "-q", "-m", "RED: a test"])
                .status
                .success()
        );
        std::fs::write(world.repo.join("tests/a.rs"), "ONE\ntwo\n").unwrap();
        assert!(world.git(&world.repo, &["add", "-A"]).status.success());

        let output = world.git(&world.repo, &["commit", "-q", "-m", "Format the test"]);

        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(stderr(&output), "");
    }

    // Rule: a repository without a checks table has no tests to freeze.
    #[tokio::test]
    async fn a_repository_without_a_checks_table_is_not_judged() {
        let world = World::new(false).await;
        world.red();

        let output = world.commit("tests/a.rs", "one\nTWO\nthree\n", "Format the test");

        assert!(output.status.success(), "{}", stderr(&output));
    }

    // Rule: where the guard cannot judge, it says so and lets the commit be made: the checks before the review look at
    // the commit again.
    #[tokio::test]
    async fn a_home_that_does_not_load_is_told_and_the_commit_is_made() {
        let world = World::new(true).await;
        world.red();
        std::fs::write(world.home.join("agents.toml"), "this is not a configuration [\n").unwrap();

        let output = world.commit("tests/a.rs", "one\nTWO\nthree\n", "Format the test");

        assert!(output.status.success(), "{}", stderr(&output));
        assert!(stderr(&output).contains("guard-commit"), "{}", stderr(&output));
    }

    // Rule: the command refuses with a status of its own, which is not the general failure status of `agent`, and ends
    // with 0 where it allows the commit or cannot judge.
    #[tokio::test]
    async fn the_command_refuses_with_a_status_of_its_own() {
        let world = World::new(true).await;
        world.red();
        std::fs::write(world.worktree.join("tests/a.rs"), "one\nTWO\nthree\n").unwrap();
        assert!(world.git(&world.worktree, &["add", "-A"]).status.success());
        let message = world.worktree.join("message.txt");
        std::fs::write(&message, "Format the test\n").unwrap();
        let ask = |dir: &Path| {
            Command::new(env!("CARGO_BIN_EXE_agent"))
                .arg("guard-commit")
                .arg(&message)
                .current_dir(dir)
                .env("AGENTS_HOME", &world.home)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap()
        };

        let refused = ask(&world.worktree);
        let elsewhere = ask(&world.repo);

        assert_eq!(refused.status.code(), Some(i32::from(COMMIT_REFUSED_STATUS)));
        assert_ne!(refused.status.code(), Some(1), "{}", stderr(&refused));
        assert!(
            stderr(&refused).contains("agent: the commit is refused"),
            "{}",
            stderr(&refused)
        );
        assert_eq!(elsewhere.status.code(), Some(0));
    }

    // Rule: `git commit -a` stages with an index of its own while the hook runs: what it takes out of a frozen test is
    // seen and refused, and what it only adds is made.
    #[tokio::test]
    async fn git_commit_a_is_judged_by_what_it_stages() {
        let world = World::new(true).await;
        world.red();
        std::fs::write(world.worktree.join("tests/a.rs"), "one\ntwo\nthree\nfour\n").unwrap();

        let added = world.git(&world.worktree, &["commit", "-q", "-a", "-m", "Cover one more case"]);

        assert!(added.status.success(), "{}", stderr(&added));
        let before = world.count();
        std::fs::write(world.worktree.join("tests/a.rs"), "one\nTWO\nthree\nfour\n").unwrap();

        let edited = world.git(&world.worktree, &["commit", "-q", "-a", "-m", "Format the test"]);

        assert!(!edited.status.success());
        assert!(
            stderr(&edited).contains("This commit was not made."),
            "{}",
            stderr(&edited)
        );
        assert_eq!(world.count(), before);
    }

    // Rule: the commit of a real merge is not judged: what the other side brings is not an edit of this commit.
    #[tokio::test]
    async fn the_commit_of_a_real_merge_is_not_refused() {
        let world = World::new(true).await;
        world.red();
        let run = |args: &[&str]| {
            let output = world.git(&world.worktree, args);
            assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        };
        run(&["switch", "-q", "-c", "other"]);
        std::fs::write(world.worktree.join("tests/a.rs"), "ONE\ntwo\nthree\n").unwrap();
        run(&["add", "-A"]);
        // Made without the hook: the branch being merged was not made in this run.
        run(&["commit", "-q", "--no-verify", "-m", "Tidy the test elsewhere"]);
        run(&["switch", "-q", "feature"]);
        std::fs::write(world.worktree.join("src.txt"), "work\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-q", "-m", "Do the work"]);
        let before = world.count();

        let merged = world.git(
            &world.worktree,
            &["merge", "-q", "--no-ff", "-m", "Merge other", "other"],
        );

        assert!(merged.status.success(), "{}", stderr(&merged));
        assert_eq!(world.count(), before + 2);
    }

    // Rule: with `commit_guard = false` under the checks table, nothing is refused; the table itself is as it was.
    #[tokio::test]
    async fn a_repository_with_the_guard_switched_off_is_not_judged() {
        let world = World::new(true).await;
        world.red();
        let config = std::fs::read_to_string(world.home.join("agents.toml")).unwrap();
        std::fs::write(
            world.home.join("agents.toml"),
            config.replace(
                "test_paths = [\"tests/\"]",
                "test_paths = [\"tests/\"]\ncommit_guard = false",
            ),
        )
        .unwrap();

        let output = world.commit("tests/a.rs", "one\nTWO\nthree\n", "Format the test");

        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(stderr(&output), "");
    }
    /// Writes the plan of the task into its worktree, as the agent keeps it; `with_for` gives each step a `For:` line.
    fn write_plan(world: &World, with_for: bool) {
        let for_line = if with_for { "    For: R1\n" } else { "" };
        std::fs::create_dir_all(world.worktree.join(".agent")).unwrap();
        std::fs::write(
            world.worktree.join(".agent/plan.md"),
            format!("Do it.\n\n1. Work\n[ ] 1.1 First\n{for_line}[ ] 1.2 Second\n{for_line}"),
        )
        .unwrap();
    }

    // Rule: in the worktree of a task whose plan has `For:` lines, a commit with no `Step:` trailer is refused with
    // what to add, and the same commit with `Step: 1.1` is made.
    #[tokio::test]
    async fn a_commit_with_no_step_trailer_is_refused_and_made_once_it_names_a_step() {
        let world = World::new(true).await;
        write_plan(&world, true);
        let before = world.count();

        let refused = world.commit("src/a.rs", "fn a() {}\n", "Add a");

        assert!(!refused.status.success());
        let text = stderr(&refused);
        assert!(text.contains("This commit was not made."), "{text}");
        assert!(
            text.contains("The `Step:` lines of this message name: nothing."),
            "{text}"
        );
        assert!(text.contains("The steps of your plan are: `1.1`, `1.2`."), "{text}");
        assert_eq!(world.count(), before);

        let made = world.git(&world.worktree, &["commit", "-q", "-m", "Add a", "-m", "Step: 1.1"]);

        assert!(made.status.success(), "{}", stderr(&made));
        assert_eq!(world.count(), before + 1);
    }

    // Rule: a trailer that names a step the plan does not have is refused with what it named.
    #[tokio::test]
    async fn a_commit_that_names_a_step_the_plan_does_not_have_is_refused() {
        let world = World::new(true).await;
        write_plan(&world, true);

        std::fs::write(world.worktree.join("a.txt"), "a\n").unwrap();
        assert!(world.git(&world.worktree, &["add", "-A"]).status.success());
        let output = world.git(&world.worktree, &["commit", "-q", "-m", "Add a", "-m", "Step: 7.3"]);

        assert!(!output.status.success());
        let text = stderr(&output);
        assert!(text.contains("name: `7.3`."), "{text}");
    }

    // Rule: a plan with no `For:` line (legacy) does not check commits.
    #[tokio::test]
    async fn a_plan_with_no_for_line_leaves_the_commits_unchecked() {
        let world = World::new(true).await;
        write_plan(&world, false);

        let output = world.commit("src/a.rs", "fn a() {}\n", "Add a");

        assert!(output.status.success(), "{}", stderr(&output));
    }

    // Rule: the step is asked for also where the repository has no `[checks]` table.
    #[tokio::test]
    async fn the_step_trailer_is_asked_for_without_a_checks_table() {
        let world = World::new(false).await;
        write_plan(&world, true);

        let output = world.commit("src/a.rs", "fn a() {}\n", "Add a");

        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("This commit was not made."),
            "{}",
            stderr(&output)
        );
    }

    // Rule: a merge commit names a step as well; the harness's own commits are made without these hooks.
    #[tokio::test]
    async fn a_merge_commit_names_a_step_too() {
        let world = World::new(true).await;
        let run = |args: &[&str]| {
            let output = world.git(&world.worktree, args);
            assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        };
        run(&["checkout", "-q", "-b", "other"]);
        let other = world.commit("src/other.rs", "fn o() {}\n", "Other work");
        assert!(other.status.success(), "{}", stderr(&other));
        run(&["checkout", "-q", "feature"]);
        let mine = world.commit("src/mine.rs", "fn m() {}\n", "My work");
        assert!(mine.status.success(), "{}", stderr(&mine));
        write_plan(&world, true);

        let refused = world.git(
            &world.worktree,
            &["merge", "-q", "--no-ff", "-m", "Merge other", "other"],
        );
        assert!(!refused.status.success());
        assert!(
            stderr(&refused).contains("This commit was not made."),
            "{}",
            stderr(&refused)
        );
        run(&["merge", "--abort"]);

        let made = world.git(
            &world.worktree,
            &[
                "merge",
                "-q",
                "--no-ff",
                "-m",
                "Merge other",
                "-m",
                "Step: 1.2",
                "other",
            ],
        );
        assert!(made.status.success(), "{}", stderr(&made));
    }

    /// A world with one commit `Other work` on the branch `other` and one on `feature`, both made before the plan has
    /// `For:` lines; returns it with the plan written.
    async fn world_with_two_branches() -> World {
        let world = World::new(true).await;
        let run = |args: &[&str]| {
            let output = world.git(&world.worktree, args);
            assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        };
        run(&["checkout", "-q", "-b", "other"]);
        let other = world.commit("src/other.rs", "fn o() {}\n", "Other work");
        assert!(other.status.success(), "{}", stderr(&other));
        run(&["checkout", "-q", "feature"]);
        let mine = world.commit("src/mine.rs", "fn m() {}\n", "My work");
        assert!(mine.status.success(), "{}", stderr(&mine));
        write_plan(&world, true);
        world
    }

    /// A world in which `other` and `feature` each wrote `src/shared.rs` and `feature` wrote it again, before the plan
    /// has `For:` lines; it is returned with the plan written.
    async fn world_with_a_conflict() -> World {
        let world = World::new(true).await;
        let run = |args: &[&str]| {
            let output = world.git(&world.worktree, args);
            assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        };
        run(&["checkout", "-q", "-b", "other"]);
        let other = world.commit("src/shared.rs", "fn o() {}\n", "Other work");
        assert!(other.status.success(), "{}", stderr(&other));
        run(&["checkout", "-q", "feature"]);
        for (text, subject) in [("fn a() {}\n", "My work"), ("fn b() {}\n", "More of my work")] {
            let made = world.commit("src/shared.rs", text, subject);
            assert!(made.status.success(), "{}", stderr(&made));
        }
        write_plan(&world, true);
        world
    }

    /// Resolves the conflict git stopped at and makes its commit with the message git prepared.
    fn commit_the_resolution(world: &World) -> Output {
        let added = world.git(&world.worktree, &["add", "-A"]);
        assert!(added.status.success(), "{}", stderr(&added));
        world.git(&world.worktree, &["commit", "-q", "--no-edit"])
    }

    // Rule: git runs no `commit-msg` hook for a plain cherry-pick, so a cherry-pick that stops at a conflict is asked
    // for its step where the agent commits the resolution, with the message of the picked commit.
    #[tokio::test]
    async fn a_cherry_pick_resolution_whose_message_has_no_step_trailer_is_refused() {
        let world = world_with_a_conflict().await;
        let stopped = world.git(&world.worktree, &["cherry-pick", "other"]);
        assert!(!stopped.status.success(), "{}", stderr(&stopped));

        let refused = commit_the_resolution(&world);

        assert!(!refused.status.success());
        assert!(
            stderr(&refused).contains("This commit was not made."),
            "{}",
            stderr(&refused)
        );
    }

    // Rule: the same holds for a revert that stops at a conflict, whose message git writes with no trailer.
    #[tokio::test]
    async fn a_revert_resolution_whose_message_has_no_step_trailer_is_refused() {
        let world = world_with_a_conflict().await;
        let stopped = world.git(&world.worktree, &["revert", "--no-edit", "HEAD~1"]);
        assert!(!stopped.status.success(), "{}", stderr(&stopped));

        let refused = commit_the_resolution(&world);

        assert!(!refused.status.success());
        assert!(
            stderr(&refused).contains("This commit was not made."),
            "{}",
            stderr(&refused)
        );
    }

    // Rule: a rebase that replays a commit whose message has no `Step:` trailer is refused at that commit.
    #[tokio::test]
    async fn a_rebase_that_replays_a_commit_with_no_step_trailer_is_refused() {
        let world = world_with_two_branches().await;

        let refused = world.git(
            &world.worktree,
            &[
                "-c",
                "core.editor=true",
                "-c",
                "sequence.editor=sed -i s/^pick/reword/",
                "rebase",
                "-i",
                "other",
            ],
        );

        assert!(!refused.status.success());
        assert!(
            stderr(&refused).contains("This commit was not made."),
            "{}",
            stderr(&refused)
        );
        let abort = world.git(&world.worktree, &["rebase", "--abort"]);
        assert!(abort.status.success(), "{}", stderr(&abort));
    }
}
