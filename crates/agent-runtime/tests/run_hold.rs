//! The watchdog held off a run: a run that waits for something the daemon does for it (the gate) is silent, and is not
//! cut for it, against a stand-in for the `claude` CLI (`tests/fixtures/run/fake-claude.sh`).

#[cfg(test)]
mod run_hold {
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::path::Path;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use agent_core::cli::{RunMode, SessionId};
    use agent_core::config::{AbsolutePath, CliArgs, Config, Language, Secrets};
    use agent_core::decide::outcome::StoppedBy;
    use agent_core::ids::{Clock, RunId, TaskId};
    use agent_core::kinds::CliKind;
    use agent_core::state::RunEnd;
    use agent_runtime::clock::SharedClock;
    use agent_runtime::harness_prompts::HarnessPrompts;
    use agent_runtime::process::Supervisor;
    use agent_runtime::run::{EngineParts, RunEngine, RunJob, RunTimings, RunVerdict, WatchdogHold};
    use agent_runtime::store::{Home, StateStore};
    use time::UtcDateTime;
    use tokio_util::sync::CancellationToken;

    /// The silence after which a run is hung, in these tests.
    const HANG: Duration = Duration::from_millis(300);

    /// Several times the hang limit: a run that the watchdog watches does not last this long.
    const SEVERAL_HANGS: Duration = Duration::from_millis(1200);

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(20);

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

    struct Env {
        home: Home,
        _home_dir: tempfile::TempDir,
        worktree: tempfile::TempDir,
        engine: RunEngine,
    }

    async fn env() -> Env {
        let home_dir = tempfile::tempdir().unwrap();
        let home = Home::new(&abs(home_dir.path()));
        let store = Arc::new(StateStore::open(home.clone()).await.unwrap());
        let fake = format!("{}/tests/fixtures/run/fake-claude.sh", env!("CARGO_MANIFEST_DIR"));
        let config = Config::from_toml_str(&format!(
            "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{fake}\"\n"
        ))
        .unwrap();
        let texts = HarnessPrompts::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
            &Language::default(),
        )
        .await
        .unwrap();
        let engine = RunEngine::new(EngineParts {
            store,
            texts: Arc::new(texts),
            config: Arc::new(config),
            secrets: Arc::new(Secrets::parse("", 0o600).unwrap()),
            supervisor: Supervisor::new(NonZeroUsize::new(4).unwrap()),
            clock: SharedClock::new(FixedClock(
                agent_core::state::Timestamp::from_str("2026-11-12T10:00:00Z")
                    .unwrap()
                    .as_utc(),
            )),
            local_offset: time::UtcOffset::UTC,
            timings: RunTimings {
                poll: Duration::from_millis(20),
                hang: HANG,
                stop_grace: Duration::from_secs(5),
            },
        });
        Env {
            home,
            _home_dir: home_dir,
            worktree: tempfile::tempdir().unwrap(),
            engine,
        }
    }

    impl Env {
        fn job(&self, scenario: &str) -> RunJob {
            std::fs::write(self.worktree.path().join(".fake-scenario"), scenario).unwrap();
            RunJob {
                task: TaskId::from_str("task-20261112-100000-t1t1").unwrap(),
                run: RunId::from_str("run-20261112-100001-r1r1").unwrap(),
                cli: CliKind::Claude,
                model: None,
                extra_args: CliArgs::try_from(Vec::<String>::new()).unwrap(),
                mode: RunMode::New {
                    session_id: SessionId::from_str(SESSION).unwrap(),
                },
                prompt: "Do the item.".to_owned(),
                working_dir: abs(self.worktree.path()),
                mcp_servers: BTreeMap::new(),
            }
        }
    }

    async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn a_silent_run_is_not_cut_for_as_long_as_the_watchdog_is_held() {
        let env = env().await;
        let hold = WatchdogHold::new();
        let guard = hold.hold();
        let marker = env.worktree.path().join(".fake-child");
        let cancel = CancellationToken::new();
        let job = env.job("hang");

        let engine = env.engine.clone();
        let token = cancel.clone();
        let running = tokio::spawn(async move { engine.run_held(job, &token, &hold).await });
        eventually("the run to start", || marker.is_file()).await;
        tokio::time::sleep(SEVERAL_HANGS).await;
        let cut = running.is_finished();
        cancel.cancel();
        let verdict = running.await.unwrap().unwrap();
        drop(guard);

        assert!(!cut, "the watchdog cut a run whose gate was being waited for");
        assert!(matches!(verdict, RunVerdict::Interrupted { .. }), "{verdict:?}");
    }

    #[tokio::test]
    async fn the_silence_of_a_run_is_counted_from_the_end_of_the_hold() {
        let env = env().await;
        let hold = WatchdogHold::new();
        let guard = hold.hold();
        let marker = env.worktree.path().join(".fake-child");
        let job = env.job("hang");

        let engine = env.engine.clone();
        let held = hold.clone();
        let running = tokio::spawn(async move { engine.run_held(job, &CancellationToken::new(), &held).await });
        eventually("the run to start", || marker.is_file()).await;
        tokio::time::sleep(SEVERAL_HANGS).await;
        let released = Instant::now();
        drop(guard);
        let verdict = running.await.unwrap().unwrap();
        let waited = released.elapsed();

        let RunVerdict::Finished(run) = verdict else {
            panic!("{verdict:?}");
        };
        assert_eq!(run.classified.end, RunEnd::Hung);
        assert_eq!(run.finish.stopped_by, StoppedBy::Watchdog);
        assert!(
            waited >= HANG,
            "the watchdog cut the run {waited:?} after the hold ended, before the limit of {HANG:?}"
        );
    }

    #[tokio::test]
    async fn a_run_that_was_never_held_is_cut_when_it_is_silent() {
        let env = env().await;
        let hold = WatchdogHold::new();

        let verdict = env
            .engine
            .run_held(env.job("hang"), &CancellationToken::new(), &hold)
            .await
            .unwrap();

        let RunVerdict::Finished(run) = verdict else {
            panic!("{verdict:?}");
        };
        assert_eq!(run.finish.stopped_by, StoppedBy::Watchdog);
    }

    #[tokio::test]
    async fn the_run_gets_the_home_in_agents_home_so_that_its_agent_commands_find_the_daemon() {
        let env = env().await;

        env.engine
            .run(env.job("env-home"), &CancellationToken::new())
            .await
            .unwrap();

        let recorded = std::fs::read_to_string(env.worktree.path().join(".fake-home")).unwrap();
        assert_eq!(recorded.trim(), env.home.root().to_str().unwrap());
    }
}
