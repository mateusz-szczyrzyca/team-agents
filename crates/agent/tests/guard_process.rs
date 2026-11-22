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
}
