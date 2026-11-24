//! The reminder of Serena in a run that goes on: a run with Serena that searches code without it past the bound is
//! told once, and goes on to its end; a run without Serena, a run that called Serena and work that is not code are
//! never told. Against the stand-in for `claude` of `tests/fixtures/run/fake-claude.sh`, in a real git repository,
//! with a real history. The stand-in writes each message the bus brings it to `.fake-reminder`, so that a test reads
//! what the run was told while it went on.

#[cfg(test)]
mod serena_reminder {
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::path::Path;
    use std::process::Command;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Duration;

    use agent_core::config::{AbsolutePath, CliArgs, CommandLine, Config, Language, Secrets};
    use agent_core::history::{AgentIdentity, EventRecord, HistoryEvent, MaxTextBytes, Record, RunStart, RunWhy};
    use agent_core::ids::{BranchName, Clock, RunId, TaskId, TeamId};
    use agent_core::kinds::{CliKind, Role, Tier};
    use agent_core::state::Timestamp;
    use agent_runtime::clock::SharedClock;
    use agent_runtime::git::CommitIdentity;
    use agent_runtime::harness_prompts::HarnessPrompts;
    use agent_runtime::history::{History, HistoryDb, Reader, Setup, Started, TracingReporter};
    use agent_runtime::process::Supervisor;
    use agent_runtime::run::{EngineParts, RunEngine, RunJob, RunTimings, RunVerdict};
    use agent_runtime::store::{Home, StateStore};
    use time::UtcDateTime;
    use tokio_util::sync::CancellationToken;

    /// The time of the clock of every test.
    const NOW: &str = "2026-11-24T10:00:00Z";

    /// The session the stand-in names first.
    const SESSION: &str = "0c4e9a52-7a11-4d0b-8f3e-5b2c6d1e9a07";

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

    fn git(dir: &Path, args: &[&str]) {
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
    }

    /// What a run is given: its configuration file's `[serena]` section, and whether it has Serena.
    struct Given {
        /// The section written into `agents.toml`; the default of forty searches when there is none.
        section: Option<&'static str>,
        /// Whether the run has Serena among its MCP servers.
        serena: bool,
    }

    impl Given {
        /// A run with Serena that is reminded at the default of forty searches.
        const DEFAULT: Self = Self {
            section: None,
            serena: true,
        };
    }

    struct Rig {
        _home_dir: tempfile::TempDir,
        home: Home,
        worktree: tempfile::TempDir,
        engine: RunEngine,
        history: History,
        _started: Option<Started>,
        run: RunId,
        task: TaskId,
        team: TeamId,
    }

    async fn rig(given: &Given) -> Rig {
        let home_dir = tempfile::tempdir().unwrap();
        let home = Home::new(&abs(home_dir.path()));
        let store = Arc::new(StateStore::open(home.clone()).await.unwrap());
        let section = given.section.map_or_else(String::new, str::to_owned);
        let config = Config::from_toml_str(&format!(
            "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{}\"\n{section}",
            fake_cli()
        ))
        .unwrap();
        let texts = HarnessPrompts::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
            &Language::default(),
        )
        .await
        .unwrap();
        let worktree = tempfile::tempdir().unwrap();
        git(worktree.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(worktree.path().join("README.md"), "hello\n").unwrap();
        git(worktree.path(), &["add", "README.md"]);
        git(worktree.path(), &["commit", "-q", "-m", "first"]);
        std::fs::write(worktree.path().join(".git/info/exclude"), ".fake-*\n").unwrap();
        let db = HistoryDb::open(&home.history_file(), MaxTextBytes::default()).unwrap();
        let started = History::start(
            db,
            Setup {
                reporter: Arc::new(TracingReporter),
                capacity: NonZeroUsize::new(1024).unwrap(),
            },
        )
        .unwrap();
        let engine = RunEngine::new(EngineParts {
            store,
            texts: Arc::new(texts),
            config: Arc::new(config),
            secrets: Arc::new(Secrets::default()),
            supervisor: Supervisor::new(NonZeroUsize::new(4).unwrap()),
            clock: SharedClock::new(FixedClock(now().as_utc())),
            local_offset: time::UtcOffset::UTC,
            timings: RunTimings {
                poll: Duration::from_millis(20),
                hang: Duration::from_secs(20),
                stop_grace: Duration::from_secs(5),
            },
            agent_binary_dir: abs(Path::new("/opt/bin")),
        })
        .with_history(started.history.clone());
        Rig {
            _home_dir: home_dir,
            home,
            worktree,
            engine,
            history: started.history.clone(),
            _started: Some(started),
            run: RunId::from_str("run-20261124-100001-r1r1").unwrap(),
            task: TaskId::from_str("task-20261124-100000-t1t1").unwrap(),
            team: TeamId::from_str("team-20261124-100000-t0t0").unwrap(),
        }
    }

    impl Rig {
        /// Records the run's start, as the owner does before the engine runs it.
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

        fn job(&self, given: &Given) -> RunJob {
            let mut mcp_servers = BTreeMap::new();
            if given.serena {
                mcp_servers.insert(
                    agent_core::cli::McpServerKind::Serena,
                    CommandLine::try_from(vec!["serena".to_owned(), "start-mcp-server".to_owned()]).unwrap(),
                );
            }
            RunJob {
                task: self.task.clone(),
                run: self.run.clone(),
                cli: CliKind::Claude,
                model: None,
                extra_args: CliArgs::try_from(Vec::<String>::new()).unwrap(),
                mode: agent_core::cli::RunMode::New {
                    session_id: SESSION.parse().unwrap(),
                },
                prompt: "Do the item.".to_owned(),
                working_dir: abs(self.worktree.path()),
                identity: CommitIdentity::new("claude-1 (agent)".to_owned(), "agents@example.com".to_owned(), None),
                mcp_servers,
                context: None,
                output: None,
            }
        }

        /// Runs the stand-in with `scenario` to its end and asserts the run went to its end.
        async fn run(&self, scenario: &str, given: &Given) {
            std::fs::write(self.worktree.path().join(".fake-scenario"), scenario).unwrap();
            self.record_run_start();
            let verdict = self
                .engine
                .run(self.job(given), &CancellationToken::new())
                .await
                .unwrap();
            assert!(
                matches!(verdict, RunVerdict::Finished(_)),
                "the run goes to its end: {verdict:?}"
            );
        }

        /// The reminder records of the run's history, in order.
        fn reminder_records(&self) -> Vec<EventRecord> {
            self.records()
                .into_iter()
                .filter(|record| matches!(record.event, HistoryEvent::SerenaReminded { .. }))
                .collect()
        }

        /// Every event of the history of the test's team, as the history holds it.
        fn records(&self) -> Vec<EventRecord> {
            self.history.flush();
            let events: Vec<EventRecord> = Reader::open(&self.home.history_file())
                .unwrap()
                .team_records(&self.team)
                .unwrap()
                .into_iter()
                .filter_map(|sequenced| match sequenced.record {
                    Record::Event(event) => Some(*event),
                    Record::Team(_)
                    | Record::Round(_)
                    | Record::Run(_)
                    | Record::Launch(_)
                    | Record::Step(_)
                    | Record::RunEnd(_)
                    | Record::Changes(_)
                    | Record::Snapshot(_)
                    | Record::Session(_) => None,
                })
                .collect();
            events
        }
    }

    // Rule: a run with Serena that reads code past the bound is reminded once, and goes on to its end.
    #[tokio::test]
    async fn a_run_with_serena_that_reads_code_past_the_bound_is_reminded_once_and_goes_on() {
        let rig = rig(&Given::DEFAULT).await;

        rig.run("serena-reads", &Given::DEFAULT).await;

        let told = rig.reminder_records();
        assert_eq!(told.len(), 1, "one reminder record: {told:?}");
        let HistoryEvent::SerenaReminded { searches, .. } = &told.first().unwrap().event else {
            panic!("a reminder: {told:?}");
        };
        assert_eq!(*searches, 40);
    }

    // Rule: the reminder is a record of the history of the run's task, with the run, the count and the CLI.
    #[tokio::test]
    async fn the_reminder_of_a_run_is_a_record_of_its_history() {
        let rig = rig(&Given::DEFAULT).await;

        rig.run("serena-reads", &Given::DEFAULT).await;
        let records = rig.records();

        let found: Vec<&EventRecord> = records
            .iter()
            .filter(|record| matches!(record.event, HistoryEvent::SerenaReminded { .. }))
            .collect();
        let [record] = found.as_slice() else {
            panic!("one reminder record: {found:?}");
        };
        let HistoryEvent::SerenaReminded {
            task,
            run,
            searches,
            cli,
            ..
        } = &record.event
        else {
            panic!("a reminder: {record:?}");
        };
        assert_eq!(*task, rig.task);
        assert_eq!(*run, rig.run);
        assert_eq!(*searches, 40);
        assert_eq!(*cli, CliKind::Claude);
    }

    // Rule: a run that lays down files of code on its way is not reminded for them.
    #[tokio::test]
    async fn files_a_run_works_on_are_no_search_of_code() {
        let rig = rig(&Given::DEFAULT).await;

        rig.run("serena-writes", &Given::DEFAULT).await;

        assert_eq!(rig.reminder_records(), Vec::new());
    }

    // Rule: at the bound of the configuration, not at the default, the reminder comes.
    #[tokio::test]
    async fn the_reminder_comes_at_the_bound_of_the_configuration() {
        let after_three = Given {
            section: Some("[serena]\nremind_after = 3\n"),
            serena: true,
        };
        let rig = rig(&after_three).await;

        rig.run("serena-three", &after_three).await;

        let told = rig.reminder_records();
        assert_eq!(told.len(), 1, "one reminder record: {told:?}");
        let HistoryEvent::SerenaReminded { searches, .. } = &told.first().unwrap().event else {
            panic!("a reminder: {told:?}");
        };
        assert_eq!(*searches, 3);
    }

    // Rule: a run without Serena is never reminded, however much code it reads.
    #[tokio::test]
    async fn a_run_without_serena_is_never_reminded() {
        let rig = rig(&Given::DEFAULT).await;
        let without = Given {
            section: None,
            serena: false,
        };

        rig.run("serena-reads", &without).await;

        assert_eq!(rig.reminder_records(), Vec::new());
    }

    // Rule: a run that calls Serena is never reminded, and its history tells of no reminder.
    #[tokio::test]
    async fn a_run_that_calls_serena_is_never_reminded() {
        let rig = rig(&Given::DEFAULT).await;

        rig.run("serena-calls", &Given::DEFAULT).await;

        assert_eq!(rig.reminder_records(), Vec::new());
    }

    // Rule: work that is not code never reaches the bound.
    #[tokio::test]
    async fn a_run_that_reads_text_only_is_never_reminded() {
        let rig = rig(&Given::DEFAULT).await;

        rig.run("serena-text", &Given::DEFAULT).await;

        assert_eq!(rig.reminder_records(), Vec::new());
    }

    // Rule: `remind_after = 0` turns the reminder off.
    #[tokio::test]
    async fn a_run_of_a_reminder_turned_off_is_never_reminded() {
        let off = Given {
            section: Some("[serena]\nremind_after = 0\n"),
            serena: true,
        };
        let rig = rig(&off).await;

        rig.run("serena-reads", &off).await;

        assert_eq!(rig.reminder_records(), Vec::new());
    }

    // Rule: the engine logs the reminder as `serena_reminded` in the daemon's log, with the task, the run and the count.
    #[tokio::test]
    async fn the_engine_logs_the_reminder_in_the_log_of_the_daemon() {
        let rig = rig(&Given::DEFAULT).await;
        let layer = agent_runtime::history::HistoryLayer::new(
            agent_core::log::LogLevel::Debug,
            SharedClock::new(FixedClock(now().as_utc())),
            Secrets::default(),
            Some(rig.history.clone()),
        );
        let dispatch: tracing::Dispatch =
            tracing_subscriber::layer::SubscriberExt::with(tracing_subscriber::registry(), layer).into();

        tracing::instrument::WithSubscriber::with_subscriber(rig.run("serena-reads", &Given::DEFAULT), dispatch).await;
        rig.history.flush();

        let rows = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
        let told: Vec<_> = rows
            .iter()
            .filter(|row| row.event() == agent_core::log::LogEvent::SerenaReminded)
            .collect();
        let [row] = told.as_slice() else {
            panic!("one log row of the reminder: {told:?}");
        };
        assert_eq!(row.task(), Some(&rig.task));
        assert_eq!(row.run(), Some(&rig.run));
    }
}
