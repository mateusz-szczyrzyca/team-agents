//! The environment of a run of an agent with an account: the daemon's, `[agent_env]`, the account's `env` and the
//! account's secrets, the later one winning; and no value of an account's secret in a log, an error, a debug text or the
//! history. Against the stand-in for `claude` of `tests/fixtures/run/fake-claude.sh`, which writes the values it sees.

#[cfg(test)]
mod account_environment {
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::path::Path;
    use std::str::FromStr;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use agent_core::cli::{RunMode, SessionId};
    use agent_core::config::{AbsolutePath, AccountName, CliArgs, Config, Language, Secrets};
    use agent_core::history::{AgentIdentity, MaxTextBytes, Record, RunStart, RunWhy};
    use agent_core::ids::{BranchName, Clock, RunId, TaskId, TeamId};
    use agent_core::kinds::{CliKind, Role, Tier};
    use agent_core::state::{RunEnd, Timestamp};
    use agent_core::status::RunOutcome;
    use agent_runtime::clock::SharedClock;
    use agent_runtime::git::CommitIdentity;
    use agent_runtime::harness_prompts::HarnessPrompts;
    use agent_runtime::history::{History, HistoryDb, Setup, TracingReporter};
    use agent_runtime::process::Supervisor;
    use agent_runtime::run::{EngineParts, RunEngine, RunError, RunJob, RunTimings, RunVerdict, WatchdogHold};
    use agent_runtime::store::{Home, StateStore};
    use time::UtcDateTime;
    use tokio_util::sync::CancellationToken;

    /// The value of an account's secret that must show nowhere.
    const MARKER: &str = "MARKER-7f3a";

    /// The one variable the daemon's own environment is known to hold in a test: Cargo sets it for the test binary.
    const DAEMON_VARIABLE: &str = "CARGO_PKG_NAME";

    /// Its value there.
    const DAEMON_VALUE: &str = env!("CARGO_PKG_NAME");

    /// The account of every test.
    const ACCOUNT: &str = "comp-claude";

    /// The time of the clock of every test.
    const NOW: &str = "2026-11-12T10:00:00Z";

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> UtcDateTime {
            NOW.parse::<Timestamp>().unwrap().as_utc()
        }
    }

    fn absolute(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
    }

    fn account() -> AccountName {
        ACCOUNT.parse().unwrap()
    }

    fn fake_cli() -> String {
        format!("{}/tests/fixtures/run/fake-claude.sh", env!("CARGO_MANIFEST_DIR"))
    }

    /// A configuration whose account has the variables `env` (lines of a TOML table) and an investigator that uses it.
    fn config_text(binary: &str, env: &str) -> String {
        format!(
            "[binaries]\nclaude = \"{binary}\"\n[accounts.{ACCOUNT}]\nowner = \"comp\"\n[accounts.{ACCOUNT}.env]\n{env}\n\
             [investigators.comp-inv]\ncli = \"claude\"\naccount = \"{ACCOUNT}\"\n"
        )
    }

    /// A secrets file with `[agent_env]` of `agent_env` and `[account_env.comp-claude]` of `secrets`.
    fn secrets_text(agent_env: &str, secrets: &str) -> String {
        format!("[agent_env]\n{agent_env}\n[account_env.{ACCOUNT}]\n{secrets}\n")
    }

    /// What a test builds and keeps alive.
    struct Rig {
        _home: tempfile::TempDir,
        tree: tempfile::TempDir,
        engine: RunEngine,
    }

    async fn rig(config: &str, secrets: &str, history: Option<History>) -> Rig {
        let home_dir = tempfile::tempdir().unwrap();
        let home = Home::new(&absolute(home_dir.path()));
        let store = Arc::new(StateStore::open(home.clone()).await.unwrap());
        let texts = HarnessPrompts::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
            &Language::default(),
        )
        .await
        .unwrap();
        let engine = RunEngine::new(EngineParts {
            store,
            config: Arc::new(Config::from_toml_str(config).unwrap()),
            texts: Arc::new(texts),
            secrets: Arc::new(Secrets::parse(secrets, 0o600).unwrap()),
            supervisor: Supervisor::new(NonZeroUsize::MIN),
            clock: SharedClock::new(FixedClock),
            local_offset: time::UtcOffset::UTC,
            timings: RunTimings {
                poll: Duration::from_millis(20),
                hang: Duration::from_secs(20),
                stop_grace: Duration::from_secs(5),
            },
            agent_binary_dir: absolute(Path::new("/usr/bin")),
        });
        let engine = match history {
            Some(history) => engine.with_history(history),
            None => engine,
        };
        Rig {
            _home: home_dir,
            tree: tempfile::tempdir().unwrap(),
            engine,
        }
    }

    fn task() -> TaskId {
        "task-20261112-100000-t1t1".parse().unwrap()
    }

    fn run_id() -> RunId {
        "run-20261112-100001-r1r1".parse().unwrap()
    }

    impl Rig {
        fn job(&self, scenario: &str) -> RunJob {
            std::fs::write(self.tree.path().join(".fake-scenario"), scenario).unwrap();
            RunJob {
                task: task(),
                run: run_id(),
                cli: CliKind::Claude,
                model: None,
                extra_args: CliArgs::default(),
                mode: RunMode::New {
                    session_id: "0c4e9a52-7a11-4d0b-8f3e-5b2c6d1e9a07".parse::<SessionId>().unwrap(),
                },
                prompt: "Record the environment.".to_owned(),
                working_dir: absolute(self.tree.path()),
                identity: CommitIdentity::new("Fixture".to_owned(), "fixture@example.com".to_owned(), None),
                mcp_servers: BTreeMap::new(),
                context: None,
                output: None,
            }
        }

        /// Runs `scenario` for an agent with `account` and returns how the engine answered.
        async fn run(&self, scenario: &str, account: Option<&AccountName>) -> Result<RunVerdict, RunError> {
            self.engine
                .run_held_as(
                    self.job(scenario),
                    account,
                    &CancellationToken::new(),
                    &WatchdogHold::new(),
                )
                .await
        }

        /// Runs the stand-in so that it records `names`, and returns what it saw: `<unset>` for a name it did not see.
        async fn saw(&self, names: &[&str], account: Option<&AccountName>) -> BTreeMap<String, String> {
            let verdict = self
                .run(&format!("env-names {}", names.join(" ")), account)
                .await
                .unwrap();
            let RunVerdict::Finished(run) = verdict else {
                panic!("the fake CLI must finish");
            };
            assert_eq!(
                run.classified.end,
                RunEnd::Reported {
                    outcome: RunOutcome::Done
                }
            );
            std::fs::read_to_string(self.tree.path().join(".fake-env-names"))
                .unwrap()
                .lines()
                .map(|line| {
                    let (name, value) = line.split_once('=').unwrap();
                    (name.to_owned(), value.to_owned())
                })
                .collect()
        }
    }

    /// One row of the order: where a name is set, and the value the run must see.
    struct Row {
        /// What the row tests, for the failure message.
        case: &'static str,
        /// The variable.
        name: &'static str,
        /// Its value in `[agent_env]`, if set there.
        agent_env: Option<&'static str>,
        /// Its value in the account's `env`, if set there.
        env: Option<&'static str>,
        /// Its value in the account's secrets, if set there.
        secret: Option<&'static str>,
        /// What the run sees.
        expected: &'static str,
    }

    // Rule: a variable of the daemon's, of `[agent_env]`, of the account's `env` and of its secrets: the later wins.
    #[tokio::test]
    async fn environment_order() {
        let rows = [
            Row {
                case: "only the daemon's",
                name: DAEMON_VARIABLE,
                agent_env: None,
                env: None,
                secret: None,
                expected: DAEMON_VALUE,
            },
            Row {
                case: "only [agent_env]",
                name: "ONLY_AGENT_ENV",
                agent_env: Some("from-agent-env"),
                env: None,
                secret: None,
                expected: "from-agent-env",
            },
            Row {
                case: "only the account's env",
                name: "ONLY_ACCOUNT_ENV",
                agent_env: None,
                env: Some("from-account-env"),
                secret: None,
                expected: "from-account-env",
            },
            Row {
                case: "only the account's secrets",
                name: "ONLY_ACCOUNT_SECRET",
                agent_env: None,
                env: None,
                secret: Some("from-account-secret"),
                expected: "from-account-secret",
            },
            Row {
                case: "the daemon's and [agent_env]",
                name: DAEMON_VARIABLE,
                agent_env: Some("from-agent-env"),
                env: None,
                secret: None,
                expected: "from-agent-env",
            },
            Row {
                case: "[agent_env] and the account's env",
                name: "TWO_ENVS",
                agent_env: Some("from-agent-env"),
                env: Some("from-account-env"),
                secret: None,
                expected: "from-account-env",
            },
            Row {
                case: "the account's env and secrets",
                name: "TWO_ACCOUNT",
                agent_env: None,
                env: Some("from-account-env"),
                secret: Some("from-account-secret"),
                expected: "from-account-secret",
            },
            Row {
                case: "all four",
                name: DAEMON_VARIABLE,
                agent_env: Some("from-agent-env"),
                env: Some("from-account-env"),
                secret: Some("from-account-secret"),
                expected: "from-account-secret",
            },
        ];
        for row in rows {
            let line =
                |value: Option<&str>| value.map_or_else(String::new, |value| format!("{} = \"{value}\"", row.name));
            let rig = rig(
                &config_text(&fake_cli(), &line(row.env)),
                &secrets_text(&line(row.agent_env), &line(row.secret)),
                None,
            )
            .await;

            let seen = rig.saw(&[row.name], Some(&account())).await;

            assert_eq!(seen[row.name], row.expected, "{}", row.case);
        }
    }

    // Rule: an agent without an account gets the environment of today: none of the account's variables.
    #[tokio::test]
    async fn a_run_without_an_account_sees_none_of_the_accounts_variables() {
        let rig = rig(
            &config_text(&fake_cli(), "ACCOUNT_ENV = \"from-account-env\""),
            &secrets_text(
                "AGENT_ENV = \"from-agent-env\"",
                "ACCOUNT_SECRET = \"from-account-secret\"",
            ),
            None,
        )
        .await;

        let seen = rig.saw(&["AGENT_ENV", "ACCOUNT_ENV", "ACCOUNT_SECRET"], None).await;

        assert_eq!(seen["AGENT_ENV"], "from-agent-env");
        assert_eq!(seen["ACCOUNT_ENV"], "<unset>");
        assert_eq!(seen["ACCOUNT_SECRET"], "<unset>");
    }

    /// Log lines a test keeps.
    #[derive(Clone, Default)]
    struct Kept(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Kept {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    // Rule: the log tells the account and the names of its variables at debug level, and never a value.
    #[tokio::test]
    async fn account_secret_redacted_in_log() {
        let rig = rig(
            &config_text(&fake_cli(), "CLAUDE_CONFIG_DIR = \"/home/x/.claude-comp\""),
            &secrets_text("", &format!("COMP_API_KEY = \"{MARKER}\"")),
            None,
        )
        .await;
        let kept = Kept::default();
        let logs = kept.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || logs.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();

        tracing::instrument::WithSubscriber::with_subscriber(
            rig.saw(&["COMP_API_KEY"], Some(&account())),
            tracing::Dispatch::new(subscriber),
        )
        .await;

        let text = String::from_utf8(kept.0.lock().unwrap().clone()).unwrap();
        assert!(text.contains("run_account_environment"), "{text}");
        assert!(text.contains(ACCOUNT), "{text}");
        assert!(
            text.contains("CLAUDE_CONFIG_DIR") && text.contains("COMP_API_KEY"),
            "{text}"
        );
        assert!(!text.contains(MARKER), "{text}");
        assert!(!text.contains("/home/x/.claude-comp"), "{text}");
    }

    // Rule: an error of a run with an account holds no value of the account's secrets.
    #[tokio::test]
    async fn account_secret_redacted_in_error() {
        let rig = rig(
            &config_text("/nonexistent/claude-for-the-account-test", ""),
            &secrets_text("", &format!("COMP_API_KEY = \"{MARKER}\"")),
            None,
        )
        .await;

        let error = rig.run("done", Some(&account())).await.unwrap_err();

        assert!(!format!("{error:?}").contains(MARKER));
        assert!(!format!("{error}").contains(MARKER));
        assert!(!std::error::Error::source(&error).is_some_and(|source| format!("{source:?}").contains(MARKER)));
    }

    // Rule: the debug text of the engine, which holds the secrets, shows no value of an account's secrets.
    #[tokio::test]
    async fn account_secret_redacted_in_debug() {
        let rig = rig(
            &config_text(&fake_cli(), ""),
            &secrets_text("", &format!("COMP_API_KEY = \"{MARKER}\"")),
            None,
        )
        .await;

        assert!(!format!("{:?}", rig.engine).contains(MARKER));
    }

    // Rule: a value of an account's secret that the CLI says is cut from the history; the names of its variables are
    // recorded with the launch.
    #[tokio::test]
    async fn account_secret_redacted_in_history() {
        let home_for_db = tempfile::tempdir().unwrap();
        let db_path = home_for_db.path().join("history.db");
        let db = HistoryDb::open(&db_path, MaxTextBytes::default()).unwrap();
        let secrets = secrets_text("", &format!("COMP_API_KEY = \"{MARKER}\""));
        let started = History::start_with_secrets(
            db,
            Setup {
                reporter: Arc::new(TracingReporter),
                capacity: NonZeroUsize::new(1024).unwrap(),
            },
            Secrets::parse(&secrets, 0o600).unwrap(),
        )
        .unwrap();
        let rig = rig(&config_text(&fake_cli(), ""), &secrets, Some(started.history.clone())).await;
        started.history.record(Record::Run(Box::new(RunStart {
            run: run_id(),
            team: "team-20261112-100000-t0t0".parse::<TeamId>().unwrap(),
            task: task(),
            at: NOW.parse().unwrap(),
            agent: AgentIdentity {
                name: "comp-inv".to_owned(),
                tier: Tier::Normal,
                cli: CliKind::Claude,
                model: None,
            },
            role: Role::Solo,
            rounds: Vec::new(),
            why: RunWhy::FirstRun,
            prompt: "Do the item.".to_owned(),
            worktree: absolute(rig.tree.path()),
            branch: BranchName::from_str("main").unwrap(),
            session: None,
        })));

        let seen = rig.saw(&["COMP_API_KEY"], Some(&account())).await;
        started.writer.close().unwrap();

        assert_eq!(seen["COMP_API_KEY"], MARKER, "the CLI saw the secret");
        let mut found = 0;
        for entry in std::fs::read_dir(home_for_db.path()).unwrap() {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            found += 1;
            assert!(
                !bytes.windows(MARKER.len()).any(|window| window == MARKER.as_bytes()),
                "{path:?}"
            );
        }
        assert!(found >= 1);
    }
}
