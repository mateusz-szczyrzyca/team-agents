//! The reminder of Serena in a run of opencode that goes on: a run with Serena that reads or finds code without it past
//! the bound is told once and goes on to its end, and a run that calls Serena, or reads text, is never told. Against the
//! stand-in for `opencode` of `tests/fixtures/run/fake-opencode.sh`, in a real git repository, with a real history.

#[cfg(test)]
mod serena_reminder_opencode {
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
        format!("{}/tests/fixtures/run/fake-opencode.sh", env!("CARGO_MANIFEST_DIR"))
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
            "[agents.claude-1]\ncli = \"opencode\"\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nopencode = \"{}\"\n{section}",
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
                    cli: CliKind::Opencode,
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
                cli: CliKind::Opencode,
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

    // Rule: opencode's `read` tool over code counts, and the run is reminded once and goes on to its end.
    #[tokio::test]
    async fn a_run_of_opencode_that_reads_code_past_the_bound_is_reminded_once_and_goes_on() {
        let rig = rig(&Given::DEFAULT).await;

        rig.run("serena-reads", &Given::DEFAULT).await;

        let told = rig.reminder_records();
        let [record] = told.as_slice() else {
            panic!("one reminder record: {told:?}");
        };
        let HistoryEvent::SerenaReminded { searches, cli, .. } = &record.event else {
            panic!("a reminder: {told:?}");
        };
        assert_eq!((*searches, *cli), (40, CliKind::Opencode));
    }

    // Rule: opencode's `glob` tool over code counts as a search of code.
    #[tokio::test]
    async fn a_run_of_opencode_that_globs_code_past_the_bound_is_reminded_once() {
        let rig = rig(&Given::DEFAULT).await;

        rig.run("serena-globs", &Given::DEFAULT).await;

        assert_eq!(rig.reminder_records().len(), 1);
    }

    // Rule: a run that called Serena is never reminded.
    #[tokio::test]
    async fn a_run_of_opencode_that_calls_serena_is_never_reminded() {
        let rig = rig(&Given::DEFAULT).await;

        rig.run("serena-calls", &Given::DEFAULT).await;

        assert_eq!(rig.reminder_records(), Vec::new());
    }

    // Rule: work that is not code never counts.
    #[tokio::test]
    async fn a_run_of_opencode_that_reads_text_only_is_never_reminded() {
        let rig = rig(&Given::DEFAULT).await;

        rig.run("serena-text", &Given::DEFAULT).await;

        assert_eq!(rig.reminder_records(), Vec::new());
    }
}
