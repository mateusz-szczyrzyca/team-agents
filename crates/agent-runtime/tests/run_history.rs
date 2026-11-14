//! What the run engine records in the history: how each process was started (the command line the CLI saw, the names —
//! never the values — of its environment), every step in order, and what the run changed in its worktree. Against the
//! stand-in for `claude` of `tests/fixtures/run/fake-claude.sh`, in a real git repository, with a real history database.

#[cfg(test)]
mod run_history {
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::path::Path;
    use std::process::Command;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use agent_core::cli::{ActivityEvent, McpServerKind, RunMode, SessionId};
    use agent_core::config::{AbsolutePath, CliArgs, Config, EnvName, Language, Secrets};
    use agent_core::decide::checks::{Change, FileChange, Lines};
    use agent_core::history::{AgentIdentity, MaxTextBytes, Record, RunStart, RunWhy};
    use agent_core::ids::{BranchName, Clock, RunId, TaskId, TeamId};
    use agent_core::kinds::{CliKind, Role, Tier};
    use agent_core::state::{CommitId, RunEnd, Timestamp};
    use agent_core::status::RunOutcome;
    use agent_runtime::clock::SharedClock;
    use agent_runtime::harness_prompts::HarnessPrompts;
    use agent_runtime::history::{History, HistoryDb, OpenError, Reader, Setup, Started, TracingReporter};
    use agent_runtime::process::Supervisor;
    use agent_runtime::run::{EngineParts, MAX_PROMPT_ARGUMENT_BYTES, RunEngine, RunJob, RunTimings, RunVerdict};
    use agent_runtime::store::{Home, StateStore};
    use time::UtcDateTime;
    use tokio_util::sync::CancellationToken;

    /// The time of the clock of every test.
    const NOW: &str = "2026-11-12T10:00:00Z";

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(20);

    /// A session ID.
    const SESSION: &str = "0c4e9a52-7a11-4d0b-8f3e-5b2c6d1e9a07";

    /// The value of a secret that must never reach the history.
    const SECRET: &str = "marker-4f1b9c0e-secret-value";

    struct FixedClock(UtcDateTime);

    impl Clock for FixedClock {
        fn now(&self) -> UtcDateTime {
            self.0
        }
    }

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
    }

    fn now() -> Timestamp {
        Timestamp::from_str(NOW).unwrap()
    }

    fn fake_cli() -> String {
        format!("{}/tests/fixtures/run/fake-claude.sh", env!("CARGO_MANIFEST_DIR"))
    }

    /// Runs real git for the setup of a test.
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
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim_end().to_owned()
    }

    /// Where a test's run works: a repository with one commit, or a directory that is none.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Tree {
        Repository,
        Plain,
    }

    struct Rig {
        _home_dir: tempfile::TempDir,
        home: Home,
        worktree: tempfile::TempDir,
        engine: RunEngine,
        history: History,
        started: Option<Started>,
        run: RunId,
        task: TaskId,
        team: TeamId,
    }

    /// The history a rig records in.
    enum Kind {
        /// A real database.
        Database,
        /// A database that could not be opened.
        Broken,
    }

    async fn rig(tree: Tree, kind: Kind) -> Rig {
        let home_dir = tempfile::tempdir().unwrap();
        let home = Home::new(&abs(home_dir.path()));
        let store = Arc::new(StateStore::open(home.clone()).await.unwrap());
        let config = Config::from_toml_str(&format!(
            "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{}\"\n",
            fake_cli()
        ))
        .unwrap();
        let texts = HarnessPrompts::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
            &Language::default(),
        )
        .await
        .unwrap();
        let secrets = format!("[agent_env]\nFAKE_SECRET = \"{SECRET}\"\n");
        let worktree = tempfile::tempdir().unwrap();
        if tree == Tree::Repository {
            git(worktree.path(), &["init", "-q", "-b", "main"]);
            std::fs::write(worktree.path().join("README.md"), "hello\n").unwrap();
            git(worktree.path(), &["add", "README.md"]);
            git(worktree.path(), &["commit", "-q", "-m", "first"]);
            let exclude = worktree.path().join(".git/info/exclude");
            std::fs::write(exclude, ".fake-*\n").unwrap();
        }
        let (history, started) = match kind {
            Kind::Database => {
                let db = HistoryDb::open(&home.history_file(), MaxTextBytes::default()).unwrap();
                let started = History::start(
                    db,
                    Setup {
                        reporter: Arc::new(TracingReporter),
                        capacity: NonZeroUsize::new(1024).unwrap(),
                    },
                )
                .unwrap();
                (started.history.clone(), Some(started))
            }
            Kind::Broken => {
                let in_the_way = home.history_file();
                std::fs::create_dir(&in_the_way).unwrap();
                let error: OpenError = HistoryDb::open(&in_the_way, MaxTextBytes::default()).unwrap_err();
                (History::unavailable(&error, Arc::new(TracingReporter)), None)
            }
        };
        let engine = RunEngine::new(EngineParts {
            store,
            texts: Arc::new(texts),
            config: Arc::new(config),
            secrets: Arc::new(Secrets::parse(&secrets, 0o600).unwrap()),
            supervisor: Supervisor::new(NonZeroUsize::new(4).unwrap()),
            clock: SharedClock::new(FixedClock(now().as_utc())),
            local_offset: time::UtcOffset::UTC,
            timings: RunTimings {
                poll: Duration::from_millis(20),
                hang: Duration::from_secs(5),
                stop_grace: Duration::from_secs(5),
            },
        })
        .with_history(history.clone());
        Rig {
            _home_dir: home_dir,
            home,
            worktree,
            engine,
            history,
            started,
            run: RunId::from_str("run-20261112-100001-r1r1").unwrap(),
            task: TaskId::from_str("task-20261112-100000-t1t1").unwrap(),
            team: TeamId::from_str("team-20261112-100000-t0t0").unwrap(),
        }
    }

    impl Rig {
        /// The record the owner hands in when it starts the run: the engine's records hang on it.
        fn record_run_start(&self) {
            self.history.record(Record::Run(Box::new(RunStart {
                run: self.run.clone(),
                team: self.team.clone(),
                task: self.task.clone(),
                at: now(),
                agent: AgentIdentity {
                    name: "claude-1".to_owned(),
                    tier: Tier::Normal,
                    cli: CliKind::Claude,
                    model: None,
                },
                role: Role::Solo,
                rounds: Vec::new(),
                why: RunWhy::FirstRun,
                prompt: "Do the item.".to_owned(),
                worktree: abs(self.worktree.path()),
                branch: BranchName::from_str("main").unwrap(),
                session: None,
            })));
        }

        fn job(&self, scenario: &str, prompt: &str) -> RunJob {
            std::fs::write(self.worktree.path().join(".fake-scenario"), scenario).unwrap();
            RunJob {
                task: self.task.clone(),
                run: self.run.clone(),
                cli: CliKind::Claude,
                model: None,
                extra_args: CliArgs::try_from(Vec::<String>::new()).unwrap(),
                mode: RunMode::New {
                    session_id: SessionId::from_str(SESSION).unwrap(),
                },
                prompt: prompt.to_owned(),
                working_dir: abs(self.worktree.path()),
                mcp_servers: BTreeMap::<McpServerKind, agent_core::config::CommandLine>::new(),
            }
        }

        /// Runs `scenario` to its end, with the run's start recorded first, and returns how it ended.
        async fn run(&self, scenario: &str, prompt: &str) -> RunEnd {
            self.record_run_start();
            let verdict = self
                .engine
                .run(self.job(scenario, prompt), &CancellationToken::new())
                .await
                .unwrap();
            let RunVerdict::Finished(finished) = verdict else {
                panic!("{verdict:?}")
            };
            finished.classified.end
        }

        fn reader(&self) -> Reader {
            self.history.flush();
            Reader::open(&self.home.history_file()).unwrap()
        }

        fn recorded(&self, name: &str) -> String {
            std::fs::read_to_string(self.worktree.path().join(name)).unwrap()
        }

        fn head(&self) -> CommitId {
            git(self.worktree.path(), &["rev-parse", "HEAD"]).parse().unwrap()
        }
    }

    fn done() -> RunEnd {
        RunEnd::Reported {
            outcome: RunOutcome::Done,
        }
    }

    // Rule: the launch has the command line the CLI saw, the directory, the head, and the names of the environment.
    #[tokio::test]
    async fn the_launch_of_a_run_is_the_command_line_the_cli_saw_with_the_names_of_its_environment() {
        let rig = rig(Tree::Repository, Kind::Database).await;
        let head = rig.head();

        assert_eq!(rig.run("env", "Do the item.").await, done());

        let launch = rig.reader().run(&rig.run).unwrap().unwrap().launch.unwrap().value;
        assert_eq!(launch.program, fake_cli());
        assert_eq!(
            launch.args,
            rig.recorded(".fake-args")
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
        assert_eq!(launch.args.last().map(String::as_str), Some("Do the item."));
        assert_eq!(launch.working_dir, abs(rig.worktree.path()));
        assert_eq!(launch.head, Some(head));
        assert_eq!(launch.at, now());
        assert!(!launch.prompt_in_file);
        let names: Vec<&str> = launch.env_names.iter().map(EnvName::as_str).collect();
        assert_eq!(names, ["AGENTS_HOME", "FAKE_SECRET", "PATH"]);
    }

    // Rule: a prompt too long for a command line is recorded as sent through a file; the CLI saw the pointer.
    #[tokio::test]
    async fn a_prompt_too_long_for_the_command_line_is_recorded_as_sent_through_a_file() {
        let rig = rig(Tree::Repository, Kind::Database).await;
        let prompt = "x".repeat(MAX_PROMPT_ARGUMENT_BYTES + 1);

        rig.run("done", &prompt).await;

        let launch = rig.reader().run(&rig.run).unwrap().unwrap().launch.unwrap().value;
        assert!(launch.prompt_in_file);
        // The stand-in writes each argument and a line break; the pointer to the file has line breaks of its own.
        assert_eq!(format!("{}\n", launch.args.join("\n")), rig.recorded(".fake-args"));
        assert_ne!(launch.args.last(), Some(&prompt));
        assert!(launch.args.last().is_some_and(|last| last.contains("prompt.md")));
    }

    // Rule: no value of a secret is in the history, in the database or in its log.
    #[tokio::test]
    async fn the_value_of_a_secret_is_nowhere_in_the_files_of_the_history() {
        let rig = rig(Tree::Repository, Kind::Database).await;

        rig.run("env", "Do the item.").await;
        let launch = rig.reader().run(&rig.run).unwrap().unwrap().launch.unwrap().value;
        assert!(
            launch.env_names.iter().any(|name| name.as_str() == "FAKE_SECRET"),
            "the name is recorded"
        );
        rig.started.unwrap().writer.close().unwrap();

        let mut found = 0;
        for entry in std::fs::read_dir(rig.home.root()).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().unwrap().to_string_lossy().starts_with("history.db") {
                let bytes = std::fs::read(&path).unwrap();
                found += 1;
                assert!(
                    !bytes.windows(SECRET.len()).any(|window| window == SECRET.as_bytes()),
                    "{path:?}"
                );
            }
        }
        assert!(found >= 1);
    }

    // Rule: every step of the run is recorded, in order, numbered from 1, with its time: what the activity log shows.
    #[tokio::test]
    async fn the_steps_of_a_run_are_recorded_in_order_with_their_numbers_and_times() {
        let rig = rig(Tree::Repository, Kind::Database).await;

        rig.run("done", "Do the item.").await;

        let steps = rig.reader().steps(&rig.run).unwrap();
        let seen: Vec<(u32, ActivityEvent)> = steps
            .iter()
            .map(|step| (step.value.number.get(), step.value.event.clone()))
            .collect();
        assert_eq!(
            seen,
            [
                (1, ActivityEvent::Info("session started, model fake-model".to_owned())),
                (2, ActivityEvent::Text("working".to_owned())),
                (3, ActivityEvent::Info("result: success".to_owned())),
            ]
        );
        assert!(steps.iter().all(|step| step.value.at == now()));
    }

    // Rule: what the run committed and what it left in the tree are recorded from git.
    #[tokio::test]
    async fn the_changes_of_a_run_are_its_commits_its_files_and_what_it_left_uncommitted() {
        let rig = rig(Tree::Repository, Kind::Database).await;
        let before = rig.head();

        rig.run("commit", "Do the item.").await;

        let changes = rig.reader().changes(&rig.run).unwrap().unwrap().value;
        let after = rig.head();
        assert_eq!(changes.head_before, Some(before));
        assert_eq!(changes.head_after, Some(after.clone()));
        assert_eq!(changes.commits.len(), 1);
        assert_eq!(changes.commits[0].id, after);
        assert_eq!(changes.commits[0].subject, "Work of the run");
        assert_eq!(
            changes.files,
            [FileChange {
                path: "work.txt".parse().unwrap(),
                change: Change::Added,
                lines: Lines::Text { added: 1, removed: 0 }
            }]
        );
        assert_eq!(
            changes.uncommitted,
            [FileChange {
                path: "left.txt".parse().unwrap(),
                change: Change::Added,
                lines: Lines::Text { added: 2, removed: 0 }
            }]
        );
    }

    // Rule: a run that is stopped is recorded as far as it got: the engine looks at the tree when the process is gone.
    #[tokio::test]
    async fn a_run_that_is_stopped_records_what_it_changed_before_it_was() {
        let rig = rig(Tree::Repository, Kind::Database).await;
        rig.record_run_start();
        let cancel = CancellationToken::new();
        let job = rig.job("commit-busy", "Do the item.");
        let running = {
            let engine = rig.engine.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move { engine.run(job, &cancel).await })
        };
        let left = rig.worktree.path().join("left.txt");
        let deadline = Instant::now() + PATIENCE;
        while !left.exists() {
            assert!(Instant::now() < deadline, "the run did not leave its file");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        cancel.cancel();
        let verdict = running.await.unwrap().unwrap();

        assert!(matches!(verdict, RunVerdict::Interrupted { .. }), "{verdict:?}");
        let changes = rig.reader().changes(&rig.run).unwrap().unwrap().value;
        assert_eq!(changes.commits.len(), 1);
        assert_eq!(changes.uncommitted.len(), 1);
    }

    // Rule: a worktree that is no repository has no heads and no changes, and the run is the same.
    #[tokio::test]
    async fn a_run_in_a_directory_that_is_no_repository_records_no_head_and_no_changes_and_ends_as_it_does() {
        let rig = rig(Tree::Plain, Kind::Database).await;

        assert_eq!(rig.run("done", "Do the item.").await, done());

        let reader = rig.reader();
        let row = reader.run(&rig.run).unwrap().unwrap();
        assert_eq!(row.launch.unwrap().value.head, None);
        assert_eq!(reader.changes(&rig.run).unwrap(), None);
        assert_eq!(reader.steps(&rig.run).unwrap().len(), 3);
    }

    // Rule: a history that cannot be written changes nothing about the run, and what it lost is counted.
    #[tokio::test]
    async fn a_history_that_cannot_be_written_does_not_change_how_a_run_ends_and_counts_what_it_lost() {
        let rig = rig(Tree::Repository, Kind::Broken).await;

        assert_eq!(rig.run("commit", "Do the item.").await, done());

        // The start of the run, its launch, its three steps and its changes.
        assert_eq!(rig.history.health().lost, 6);
        assert!(rig.recorded("work.txt").contains("the work"));
    }
}
