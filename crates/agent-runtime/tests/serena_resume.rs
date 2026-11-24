//! The count of a session's searches of code without Serena over a restart of the daemon: a session that goes on in a
//! resumed run, carried out by a new engine, keeps the count its earlier runs made and is never reminded twice. Against
//! the stand-in for `claude` of `tests/fixtures/run/fake-claude.sh`, with a real history that both engines share.

#[cfg(test)]
mod serena_resume {
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::path::Path;
    use std::process::Command;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Duration;

    use agent_core::cli::{McpServerKind, RunMode};
    use agent_core::config::{AbsolutePath, CliArgs, CommandLine, Config, Language, Secrets};
    use agent_core::history::{AgentIdentity, HistoryEvent, MaxTextBytes, Record, RunStart, RunWhy};
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

    /// The session both runs are of.
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

    /// What the two engines of a test share: the home with its history, the worktree and the parts of an engine.
    struct Rig {
        _home_dir: tempfile::TempDir,
        home: Home,
        worktree: tempfile::TempDir,
        parts: EngineParts,
        history: History,
        _started: Started,
        task: TaskId,
        team: TeamId,
    }

    /// A rig whose `agents.toml` ends with `section`.
    async fn rig(section: &str) -> Rig {
        let home_dir = tempfile::tempdir().unwrap();
        let home = Home::new(&abs(home_dir.path()));
        let store = Arc::new(StateStore::open(home.clone()).await.unwrap());
        let fake = format!("{}/tests/fixtures/run/fake-claude.sh", env!("CARGO_MANIFEST_DIR"));
        let config = Config::from_toml_str(&format!(
            "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{fake}\"\n{section}"
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
                capacity: NonZeroUsize::new(4096).unwrap(),
            },
        )
        .unwrap();
        let parts = EngineParts {
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
        };
        Rig {
            _home_dir: home_dir,
            home,
            worktree,
            parts,
            history: started.history.clone(),
            _started: started,
            task: TaskId::from_str("task-20261124-100000-t1t1").unwrap(),
            team: TeamId::from_str("team-20261124-100000-t0t0").unwrap(),
        }
    }

    impl Rig {
        /// An engine that knows nothing of the runs of the engines before it, as the daemon after a restart.
        fn new_engine(&self) -> RunEngine {
            RunEngine::new(self.parts.clone()).with_history(self.history.clone())
        }

        /// Carries out the run `run` of the session with `scenario` in `engine`, as a new session or a resumed one, and
        /// asserts that it went to its end.
        async fn run(&self, engine: &RunEngine, run: &str, mode: RunMode, scenario: &str) {
            std::fs::write(self.worktree.path().join(".fake-scenario"), scenario).unwrap();
            let run = RunId::from_str(run).unwrap();
            self.history.record(Record::Run(Box::new(RunStart {
                run: run.clone(),
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
                session: Some(SESSION.parse().unwrap()),
            })));
            let mut mcp_servers = BTreeMap::new();
            mcp_servers.insert(
                McpServerKind::Serena,
                CommandLine::try_from(vec!["serena".to_owned(), "start-mcp-server".to_owned()]).unwrap(),
            );
            let job = RunJob {
                task: self.task.clone(),
                run,
                cli: CliKind::Claude,
                model: None,
                extra_args: CliArgs::try_from(Vec::<String>::new()).unwrap(),
                mode,
                prompt: "Do the item.".to_owned(),
                working_dir: abs(self.worktree.path()),
                identity: CommitIdentity::new("claude-1 (agent)".to_owned(), "agents@example.com".to_owned(), None),
                mcp_servers,
                context: None,
                output: None,
            };
            let verdict = engine.run(job, &CancellationToken::new()).await.unwrap();
            assert!(
                matches!(verdict, RunVerdict::Finished(_)),
                "the run goes to its end: {verdict:?}"
            );
        }

        /// The runs and the counts of the reminders the history holds, in order.
        fn reminders(&self) -> Vec<(String, u64)> {
            self.history.flush();
            Reader::open(&self.home.history_file())
                .unwrap()
                .team_records(&self.team)
                .unwrap()
                .into_iter()
                .filter_map(|sequenced| match sequenced.record {
                    Record::Event(event) => match event.event {
                        HistoryEvent::SerenaReminded { run, searches, .. } => Some((run.as_str().to_owned(), searches)),
                        _ => None,
                    },
                    _ => None,
                })
                .collect()
        }
    }

    fn new_session() -> RunMode {
        RunMode::New {
            session_id: SESSION.parse().unwrap(),
        }
    }

    fn resumed() -> RunMode {
        RunMode::Resume {
            session_id: SESSION.parse().unwrap(),
        }
    }

    // Rule: a session reminded before a restart is not reminded again by the resumed run of the new engine.
    #[tokio::test]
    async fn a_session_reminded_before_a_restart_is_not_reminded_again() {
        let rig = rig("").await;
        let before = rig.new_engine();
        rig.run(&before, "run-20261124-100001-r1r1", new_session(), "serena-reads")
            .await;
        assert_eq!(rig.reminders(), [("run-20261124-100001-r1r1".to_owned(), 40)]);

        let after = rig.new_engine();
        rig.run(&after, "run-20261124-100002-r2r2", resumed(), "serena-reads")
            .await;

        assert_eq!(rig.reminders(), [("run-20261124-100001-r1r1".to_owned(), 40)]);
    }

    // Rule: a resumed session keeps the count of its earlier runs over a restart: it is reminded at the bound of the session.
    #[tokio::test]
    async fn a_resumed_session_keeps_its_count_over_a_restart() {
        let rig = rig("[serena]\nremind_after = 50\n").await;
        let before = rig.new_engine();
        rig.run(&before, "run-20261124-100001-r1r1", new_session(), "serena-reads")
            .await;
        assert_eq!(rig.reminders(), []);

        let after = rig.new_engine();
        rig.run(&after, "run-20261124-100002-r2r2", resumed(), "serena-reads")
            .await;

        assert_eq!(rig.reminders(), [("run-20261124-100002-r2r2".to_owned(), 50)]);
    }

    // Rule: a new session of the same task counts anew, over a restart too.
    #[tokio::test]
    async fn a_new_session_counts_anew_after_a_restart() {
        let rig = rig("").await;
        let before = rig.new_engine();
        rig.run(&before, "run-20261124-100001-r1r1", new_session(), "serena-reads")
            .await;

        let after = rig.new_engine();
        let other = RunMode::New {
            session_id: "5a1f0c7e-2b3d-4e8f-9a6b-1c2d3e4f5a6b".parse().unwrap(),
        };
        rig.run(&after, "run-20261124-100002-r2r2", other, "serena-reads").await;

        assert_eq!(
            rig.reminders(),
            [
                ("run-20261124-100001-r1r1".to_owned(), 40),
                ("run-20261124-100002-r2r2".to_owned(), 40)
            ]
        );
    }
}
