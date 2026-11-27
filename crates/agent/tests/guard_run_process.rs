//! The `Step:` trailer in a run: the fake `claude` commits in the worktree of a task whose plan has `For:` lines, with
//! the git the run engine starts, the hooks it puts in front of it and the real `agent guard-commit`. A commit with no
//! trailer is refused with what to add; the same commit with `Step: 1.1` is made.

#[cfg(test)]
mod guard_run_process {
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::path::Path;
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;

    use agent_core::cli::{RunMode, SessionId};
    use agent_core::config::{AbsolutePath, CliArgs, Config, Language, RepoName, Secrets};
    use agent_core::ids::{Clock, TaskId};
    use agent_core::kinds::CliKind;
    use agent_runtime::clock::SharedClock;
    use agent_runtime::git::CommitIdentity;
    use agent_runtime::harness_prompts::HarnessPrompts;
    use agent_runtime::process::Supervisor;
    use agent_runtime::run::{EngineParts, RunEngine, RunJob, RunTimings, RunVerdict};
    use agent_runtime::store::{Home, StateStore};
    use time::UtcDateTime;
    use tokio_util::sync::CancellationToken;

    /// The task whose worktree the run commits in.
    const TASK: &str = "task-20261121-190924-k3x9";

    /// A clock that stands still: the run does not read it for anything the test looks at.
    #[derive(Debug, Clone)]
    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> UtcDateTime {
            "2026-11-12T10:00:00Z"
                .parse::<agent_core::state::Timestamp>()
                .unwrap()
                .as_utc()
        }
    }

    fn absolute(path: &Path) -> AbsolutePath {
        path.to_str().unwrap().parse().unwrap()
    }

    /// Runs `git` in `dir` as a plain person with no configuration of the machine and returns what it printed.
    fn git(dir: &Path, args: &[&str]) -> String {
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
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// What the fake `claude` wrote in the worktree of its run, as the files say.
    struct Trial {
        _temp: tempfile::TempDir,
        worktree: std::path::PathBuf,
    }

    impl Trial {
        /// The text of a file the fake CLI wrote in the worktree.
        fn file(&self, name: &str) -> String {
            std::fs::read_to_string(self.worktree.join(name))
                .unwrap()
                .trim()
                .to_owned()
        }
    }

    /// A run of the fake `claude` on the scenario `commit-no-step` in the worktree of a task of a temporary home, whose
    /// plan has `For:` lines.
    async fn trial() -> Trial {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home_path = root.join("home");
        std::fs::create_dir(&home_path).unwrap();
        let home = Home::new(&absolute(&home_path));
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("../agent-runtime/tests/fixtures/run/fake-claude.sh");
        std::fs::write(
            home.config_file(),
            format!(
                "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{}\"\n\
                 [repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n[repos.example.checks]\ntest_paths = [\"tests/\"]\n",
                fake.display()
            ),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
            home_path.join("prompts"),
        )
        .unwrap();
        let repo = root.join("example");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("README.md"), "readme\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "Start"]);
        git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        let worktree = home.worktree_dir(
            &"example".parse::<RepoName>().unwrap(),
            &TASK.parse::<TaskId>().unwrap(),
        );
        std::fs::create_dir_all(worktree.parent().unwrap()).unwrap();
        git(
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
        std::fs::create_dir_all(worktree.join(".agent")).unwrap();
        std::fs::write(
            worktree.join(".agent/plan.md"),
            "Do it.\n\n1. Work\n[ ] 1.1 First\n    For: R1\n[ ] 1.2 Second\n    For: R1\n",
        )
        .unwrap();
        std::fs::write(worktree.join(".fake-scenario"), "commit-no-step").unwrap();
        // The hook the run's git is given runs `agent guard-commit`, found on the `PATH` of the run: the real binary.
        let bin = root.join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_agent"), bin.join("agent")).unwrap();
        let store = Arc::new(StateStore::open(home).await.unwrap());
        let config = Config::from_toml_str(&format!("[binaries]\nclaude = \"{}\"\n", fake.display())).unwrap();
        let texts = HarnessPrompts::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
            &Language::default(),
        )
        .await
        .unwrap();
        let engine = RunEngine::new(EngineParts {
            store,
            config: Arc::new(config),
            texts: Arc::new(texts),
            secrets: Arc::new(Secrets::default()),
            supervisor: Supervisor::new(NonZeroUsize::MIN),
            clock: SharedClock::new(FixedClock),
            local_offset: time::UtcOffset::UTC,
            timings: RunTimings {
                poll: Duration::from_millis(20),
                hang: Duration::from_secs(20),
                stop_grace: Duration::from_secs(5),
            },
            agent_binary_dir: absolute(&bin),
        });
        let session_id: SessionId = "0c4e9a52-7a11-4d0b-8f3e-5b2c6d1e9a07".parse().unwrap();
        let job = RunJob {
            task: TASK.parse().unwrap(),
            run: "run-20261112-100001-r1r1".parse().unwrap(),
            cli: CliKind::Claude,
            model: None,
            extra_args: CliArgs::default(),
            mode: RunMode::New { session_id },
            prompt: "Commit.".to_owned(),
            working_dir: absolute(&worktree),
            identity: CommitIdentity::new("Fixture".to_owned(), "fixture@example.com".to_owned(), None),
            mcp_servers: BTreeMap::new(),
            context: None,
            output: None,
        };
        let RunVerdict::Finished(_) = engine.run(job, &CancellationToken::new()).await.unwrap() else {
            panic!("the fake CLI must finish");
        };
        Trial { _temp: temp, worktree }
    }

    // Rule: a commit of a run with no `Step:` trailer is refused with the text `commit-refused-step.md` gives, and the
    // same commit with `Step: 1.1` is made.
    #[tokio::test]
    async fn a_commit_of_a_run_with_no_step_trailer_is_refused_and_made_once_it_names_a_step() {
        let trial = trial().await;

        let refused_with = trial.file(".fake-commit-err");
        let retried_with = trial.file(".fake-retry-code");

        assert_eq!(trial.file(".fake-commit-code"), "1");
        assert!(refused_with.contains("This commit was not made."), "{refused_with}");
        assert!(
            refused_with.contains("The `Step:` lines of this message name: nothing."),
            "{refused_with}"
        );
        assert!(
            refused_with.contains("The steps of your plan are: `1.1`, `1.2`."),
            "{refused_with}"
        );
        assert_eq!(retried_with, "0");
        assert_eq!(git(&trial.worktree, &["rev-list", "--count", "HEAD"]), "2");
        assert_eq!(
            git(
                &trial.worktree,
                &["log", "-1", "--format=%(trailers:key=Step,valueonly)"]
            ),
            "1.1"
        );
    }
}
