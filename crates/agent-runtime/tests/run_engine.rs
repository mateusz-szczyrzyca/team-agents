//! The run engine against a stand-in for the `claude` CLI (`tests/fixtures/run/fake-claude.sh`): every way a run can
//! end, the files and the environment it gets, its activity, the watchdog and the stop of a whole process group.

#[cfg(test)]
mod run_engine {
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use agent_core::cli::{McpServerKind, RunMode, SessionId};
    use agent_core::config::{AbsolutePath, CliArgs, CommandLine, Config, Language, Secrets};
    use agent_core::decide::outcome::{ExitStatus, PauseReason, StoppedBy};
    use agent_core::ids::{Clock, RunId, TaskId};
    use agent_core::kinds::CliKind;
    use agent_core::state::{RunEnd, Timestamp};
    use agent_core::status::RunOutcome;
    use agent_runtime::clock::SharedClock;
    use agent_runtime::harness_prompts::HarnessPrompts;
    use agent_runtime::process::Supervisor;
    use agent_runtime::run::{
        EngineParts, FinishedRun, MAX_PROMPT_ARGUMENT_BYTES, Progress, RunEngine, RunError, RunJob, RunTimings,
        RunVerdict,
    };
    use agent_runtime::store::{Home, StateStore};
    use time::UtcDateTime;
    use tokio_util::sync::CancellationToken;

    /// The time of the clock of every test.
    const NOW: &str = "2026-11-12T10:00:00Z";

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(20);

    const SESSION: &str = "0c4e9a52-7a11-4d0b-8f3e-5b2c6d1e9a07";

    struct FixedClock(UtcDateTime);

    impl Clock for FixedClock {
        fn now(&self) -> UtcDateTime {
            self.0
        }
    }

    fn fake_cli() -> String {
        format!("{}/tests/fixtures/run/fake-claude.sh", env!("CARGO_MANIFEST_DIR"))
    }

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
    }

    fn now() -> Timestamp {
        Timestamp::from_str(NOW).unwrap()
    }

    fn after(span: time::Duration) -> Timestamp {
        Timestamp::from_utc(now().as_utc() + span).unwrap()
    }

    /// Whether the process is gone: not in the process table, or a zombie that only waits to be reaped.
    fn is_gone(pid: i32) -> bool {
        let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return true;
        };
        text.rsplit_once(") ")
            .and_then(|(_, rest)| rest.chars().next())
            .is_none_or(|state| matches!(state, 'Z' | 'X'))
    }

    async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    struct Env {
        home: Home,
        _home_dir: tempfile::TempDir,
        worktree: tempfile::TempDir,
        engine: RunEngine,
        task: TaskId,
    }

    fn timings() -> RunTimings {
        RunTimings {
            poll: Duration::from_millis(20),
            hang: Duration::from_secs(2),
            stop_grace: Duration::from_secs(5),
        }
    }

    async fn env_with(binary: &str, secrets: &str, timings: RunTimings) -> Env {
        let home_dir = tempfile::tempdir().unwrap();
        let home = Home::new(&abs(home_dir.path()));
        let store = Arc::new(StateStore::open(home.clone()).await.unwrap());
        let config = Config::from_toml_str(&format!(
            "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{binary}\"\n"
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
            secrets: Arc::new(Secrets::parse(secrets, 0o600).unwrap()),
            supervisor: Supervisor::new(NonZeroUsize::new(4).unwrap()),
            clock: SharedClock::new(FixedClock(now().as_utc())),
            local_offset: time::UtcOffset::UTC,
            timings,
        });
        Env {
            home,
            _home_dir: home_dir,
            worktree: tempfile::tempdir().unwrap(),
            engine,
            task: TaskId::from_str("task-20261112-100000-t1t1").unwrap(),
        }
    }

    async fn env() -> Env {
        env_with(&fake_cli(), "", timings()).await
    }

    impl Env {
        fn job(&self, scenario: &str, mode: RunMode) -> RunJob {
            std::fs::write(self.worktree.path().join(".fake-scenario"), scenario).unwrap();
            RunJob {
                task: self.task.clone(),
                run: RunId::from_str("run-20261112-100001-r1r1").unwrap(),
                cli: CliKind::Claude,
                model: None,
                extra_args: CliArgs::try_from(Vec::<String>::new()).unwrap(),
                mode,
                prompt: "Do the item.".to_owned(),
                working_dir: abs(self.worktree.path()),
                mcp_servers: BTreeMap::new(),
            }
        }

        fn new_session() -> RunMode {
            RunMode::New {
                session_id: SessionId::from_str(SESSION).unwrap(),
            }
        }

        async fn finished(&self, scenario: &str) -> FinishedRun {
            let job = self.job(scenario, Self::new_session());
            match self.engine.run(job, &CancellationToken::new()).await.unwrap() {
                RunVerdict::Finished(run) => *run,
                other @ RunVerdict::Interrupted { .. } => panic!("{other:?}"),
            }
        }

        fn recorded(&self, name: &str) -> String {
            std::fs::read_to_string(self.worktree.path().join(name)).unwrap()
        }

        fn child(&self) -> i32 {
            self.recorded(".fake-child").trim().parse().unwrap()
        }
    }

    #[tokio::test]
    async fn a_run_that_reports_done_is_finished_with_its_session_and_final_message() {
        let env = env().await;

        let run = env.finished("done").await;

        assert_eq!(
            run.classified.end,
            RunEnd::Reported {
                outcome: RunOutcome::Done
            }
        );
        assert_eq!(run.classified.pause, None);
        assert_eq!(run.finish.exit, ExitStatus::Code(0));
        assert_eq!(run.finish.stopped_by, StoppedBy::Nobody);
        assert_eq!(run.session, Some(SessionId::from_str(SESSION).unwrap()));
        assert_eq!(run.final_message.as_deref(), Some("Finished the item.\nSTATUS: DONE"));
        assert_eq!(run.progress, Progress::Made);
    }

    #[tokio::test]
    async fn a_new_run_gives_the_cli_its_session_and_its_prompt() {
        let env = env().await;

        env.finished("done").await;

        let args = env.recorded(".fake-args");
        let args: Vec<&str> = args.lines().collect();
        assert!(
            args.windows(2).any(|pair| pair == ["--session-id", SESSION]),
            "{args:?}"
        );
        assert_eq!(args.last(), Some(&"Do the item."));
    }

    #[tokio::test]
    async fn a_resumed_run_names_the_session_it_resumes() {
        let env = env().await;
        let mode = RunMode::Resume {
            session_id: SessionId::from_str("ses-7").unwrap(),
        };

        let verdict = env
            .engine
            .run(env.job("done", mode), &CancellationToken::new())
            .await
            .unwrap();

        let RunVerdict::Finished(run) = verdict else {
            panic!("{verdict:?}")
        };
        assert_eq!(run.session, Some(SessionId::from_str("ses-7").unwrap()));
        let args = env.recorded(".fake-args");
        assert!(
            args.lines()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|pair| pair == ["--resume", "ses-7"])
        );
    }

    #[tokio::test]
    async fn the_activity_of_a_run_is_logged_in_the_task_directory_in_order() {
        let env = env().await;

        env.finished("done").await;

        let log = std::fs::read_to_string(env.home.activity_log(&env.task)).unwrap();
        let events: Vec<serde_json::Value> = log.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        let kinds: Vec<&str> = events.iter().map(|event| event["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["info", "text", "info"]);
        assert_eq!(events[1]["text"], "working");
        assert_eq!(events[0]["at"], NOW);
    }

    #[tokio::test]
    async fn the_output_of_a_run_is_kept_in_its_directory() {
        let env = env().await;
        let job = env.job("done", Env::new_session());
        let dir = env.home.run_dir(&job.task, &job.run);

        env.engine.run(job, &CancellationToken::new()).await.unwrap();

        assert!(
            std::fs::read_to_string(dir.join("out.jsonl"))
                .unwrap()
                .contains("STATUS: DONE")
        );
        assert!(dir.join("err.log").is_file());
        assert!(dir.join("mcp.json").is_file(), "the adapter's files are laid down");
        assert!(dir.join("claude-settings.json").is_file());
    }

    #[tokio::test]
    async fn the_end_of_a_run_without_a_status_line_is_no_status_with_its_exit_code() {
        // One run ID names one directory: each run gets its own home.
        let clean = env().await.finished("nostatus").await;
        let crashed = env().await.finished("crash").await;

        assert_eq!(clean.classified.end, RunEnd::NoStatus { exit_code: Some(0) });
        assert_eq!(crashed.classified.end, RunEnd::NoStatus { exit_code: Some(3) });
        assert_eq!(crashed.session, None);
        assert_eq!(crashed.progress, Progress::Nothing);
    }

    #[tokio::test]
    async fn a_failed_status_is_reported_as_it_is() {
        let env = env().await;

        let run = env.finished("fail").await;

        assert_eq!(
            run.classified.end,
            RunEnd::Reported {
                outcome: RunOutcome::Failed
            }
        );
    }

    #[tokio::test]
    async fn an_exhausted_quota_pauses_the_task_until_the_reset_time_the_cli_gave() {
        let env = env().await;

        let run = env.finished("quota").await;

        let reset = after(time::Duration::hours(3));
        assert_eq!(run.classified.end, RunEnd::QuotaExhausted { resets_at: Some(reset) });
        let pause = run.classified.pause.unwrap();
        assert_eq!(pause.reason, PauseReason::Quota);
        assert_eq!(
            pause.until,
            after(time::Duration::hours(3) + time::Duration::minutes(1))
        );
        assert!(pause.evidence.contains("usage limit"), "{pause:?}");
    }

    #[tokio::test]
    async fn a_rate_limit_pauses_the_task_for_the_configured_minutes() {
        let env = env().await;

        let run = env.finished("ratelimit").await;

        assert_eq!(run.classified.end, RunEnd::RateLimited);
        let pause = run.classified.pause.unwrap();
        assert_eq!(pause.reason, PauseReason::RateLimit);
        assert_eq!(pause.until, after(time::Duration::minutes(6)));
    }

    #[tokio::test]
    async fn a_line_of_output_that_is_not_json_does_not_fail_the_run() {
        let env = env().await;

        let run = env.finished("garbage").await;

        assert_eq!(
            run.classified.end,
            RunEnd::Reported {
                outcome: RunOutcome::Done
            }
        );
    }

    #[tokio::test]
    async fn the_status_after_more_output_than_one_look_reads_is_still_seen() {
        // No look at the output before the CLI ends: the end reads all of its 18 MB at once.
        let slow = RunTimings {
            poll: Duration::from_mins(10),
            hang: Duration::from_mins(20),
            stop_grace: Duration::from_secs(5),
        };
        let env = env_with(&fake_cli(), "", slow).await;

        let run = env.finished("flood").await;

        assert_eq!(
            run.classified.end,
            RunEnd::Reported {
                outcome: RunOutcome::Done
            }
        );
        assert_eq!(run.final_message.as_deref(), Some("Finished the item.\nSTATUS: DONE"));
    }

    #[tokio::test]
    async fn a_silent_run_is_killed_by_the_watchdog_with_everything_it_started() {
        let env = env().await;

        let run = env.finished("hang").await;

        assert_eq!(run.classified.end, RunEnd::Hung);
        assert_eq!(run.classified.pause, None);
        assert_eq!(run.finish.stopped_by, StoppedBy::Watchdog);
        assert_eq!(run.finish.exit, ExitStatus::Signal);
        assert_eq!(
            run.session,
            Some(SessionId::from_str(SESSION).unwrap()),
            "the session is kept for the resume"
        );
        // The signal is delivered a moment after it is sent.
        let child = env.child();
        eventually("the child of the run to be gone", || is_gone(child)).await;
    }

    #[tokio::test]
    async fn a_run_that_ends_by_itself_leaves_no_process_it_started() {
        let env = env().await;

        let run = env.finished("done-child").await;

        assert_eq!(
            run.classified.end,
            RunEnd::Reported {
                outcome: RunOutcome::Done
            }
        );
        assert_eq!(run.finish.stopped_by, StoppedBy::Nobody);
        // The signal is delivered a moment after it is sent.
        let child = env.child();
        eventually("the child of the run to be gone", || is_gone(child)).await;
    }

    #[tokio::test]
    async fn a_cancelled_run_is_stopped_whole_and_keeps_its_session() {
        let env = env().await;
        let cancel = CancellationToken::new();
        let job = env.job("hang", Env::new_session());
        let marker = env.worktree.path().join(".fake-child");

        let engine = env.engine.clone();
        let token = cancel.clone();
        let running = tokio::spawn(async move { engine.run(job, &token).await });
        eventually("the child of the run", || marker.is_file()).await;
        cancel.cancel();
        let verdict = running.await.unwrap().unwrap();

        assert_eq!(
            verdict,
            RunVerdict::Interrupted {
                session: Some(SessionId::from_str(SESSION).unwrap())
            }
        );
        // The signal is delivered a moment after it is sent.
        let child = env.child();
        eventually("the child of the run to be gone", || is_gone(child)).await;
    }

    #[tokio::test]
    async fn a_program_that_does_not_exist_is_a_spawn_error() {
        let env = env_with("/nonexistent/claude", "", timings()).await;

        let error = env
            .engine
            .run(env.job("done", Env::new_session()), &CancellationToken::new())
            .await
            .unwrap_err();

        assert!(
            matches!(
                &error,
                RunError::Spawn {
                    cli: CliKind::Claude,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_missing_working_directory_is_a_spawn_error() {
        let env = env().await;
        let mut job = env.job("done", Env::new_session());
        job.working_dir = abs(&env.worktree.path().join("missing"));

        let error = env.engine.run(job, &CancellationToken::new()).await.unwrap_err();

        assert!(matches!(&error, RunError::Spawn { .. }), "{error:?}");
    }

    #[tokio::test]
    async fn the_secrets_and_the_path_of_the_agents_are_the_environment_of_the_run() {
        let env = env_with(&fake_cli(), "[agent_env]\nFAKE_SECRET = \"s3cret\"\n", timings()).await;

        env.finished("env").await;

        assert_eq!(
            env.recorded(".fake-env"),
            "PATH=/usr/local/bin:/usr/bin:/bin\nFAKE_SECRET=s3cret\n"
        );
    }

    #[tokio::test]
    async fn the_mcp_servers_of_the_job_are_in_the_run_configuration() {
        let env = env().await;
        let mut job = env.job("done", Env::new_session());
        job.mcp_servers.insert(
            McpServerKind::Agentbus,
            CommandLine::try_from(vec!["/opt/bin/agentbus".to_owned(), "serve".to_owned()]).unwrap(),
        );
        let mcp = env.home.run_dir(&job.task, &job.run).join("mcp.json");

        env.engine.run(job, &CancellationToken::new()).await.unwrap();

        let text = std::fs::read_to_string(mcp).unwrap();
        assert!(text.contains("/opt/bin/agentbus") && text.contains("serve"), "{text}");
    }

    #[tokio::test]
    async fn a_prompt_too_long_for_a_command_line_is_given_as_a_file() {
        let env = env().await;
        let mut job = env.job("done", Env::new_session());
        job.prompt = "x".repeat(MAX_PROMPT_ARGUMENT_BYTES + 1);
        let file = env.home.run_dir(&job.task, &job.run).join("prompt.md");

        env.engine.run(job, &CancellationToken::new()).await.unwrap();

        assert_eq!(
            std::fs::read_to_string(&file).unwrap().len(),
            MAX_PROMPT_ARGUMENT_BYTES + 1
        );
        let args = env.recorded(".fake-args");
        assert!(args.contains(file.to_str().unwrap()), "{args}");
        assert!(
            !args.contains("xxxxxxxxxx"),
            "the prompt itself was given as an argument"
        );
    }

    #[tokio::test]
    async fn a_prompt_of_the_largest_size_for_a_command_line_is_still_an_argument() {
        let env = env().await;
        let mut job = env.job("done", Env::new_session());
        job.prompt = "y".repeat(MAX_PROMPT_ARGUMENT_BYTES);

        env.engine.run(job, &CancellationToken::new()).await.unwrap();

        let args = env.recorded(".fake-args");
        assert_eq!(args.lines().last().map(str::len), Some(MAX_PROMPT_ARGUMENT_BYTES));
    }

    #[tokio::test]
    async fn what_the_run_keeps_in_the_worktree_is_kept_out_of_git_once() {
        let env = env().await;
        let status = Command::new("git")
            .current_dir(env.worktree.path())
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(status.success());
        let serena = CommandLine::try_from(
            [
                "serena",
                "start-mcp-server",
                "--project",
                env.worktree.path().to_str().unwrap(),
            ]
            .map(str::to_owned)
            .to_vec(),
        )
        .unwrap();
        let exclude: PathBuf = env.worktree.path().join(".git/info/exclude");

        for _ in 0..2 {
            let mut job = env.job("done", Env::new_session());
            job.mcp_servers.insert(McpServerKind::Serena, serena.clone());
            env.engine.run(job, &CancellationToken::new()).await.unwrap();
        }

        let text = std::fs::read_to_string(exclude).unwrap();
        assert_eq!(text.lines().filter(|line| *line == ".serena/").count(), 1, "{text}");
    }

    #[tokio::test]
    async fn a_worktree_that_is_no_repository_cannot_keep_its_files_out_of_git() {
        let env = env().await;
        let mut job = env.job("done", Env::new_session());
        job.mcp_servers.insert(
            McpServerKind::Serena,
            CommandLine::try_from(vec!["serena".to_owned(), "start-mcp-server".to_owned()]).unwrap(),
        );

        let error = env.engine.run(job, &CancellationToken::new()).await.unwrap_err();

        assert!(
            matches!(&error, RunError::Exclude { worktree, .. } if *worktree == env.worktree.path()),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_run_directory_that_cannot_be_made_is_an_error_naming_the_file() {
        let env = env().await;
        let job = env.job("done", Env::new_session());
        let task_dir = env.home.task_dir(&job.task);
        std::fs::create_dir_all(&task_dir).unwrap();
        // A file where the directory of the runs should be.
        std::fs::write(task_dir.join("runs"), "").unwrap();

        let error = env.engine.run(job, &CancellationToken::new()).await.unwrap_err();

        assert!(
            matches!(&error, RunError::LayDown { path, .. } if path.starts_with(&task_dir)),
            "{error:?}"
        );
    }

    #[test]
    fn the_timings_of_the_configuration_are_its_hang_limit_a_look_a_second_and_ten_seconds_of_grace() {
        let config = Config::from_toml_str("[watchdog]\nhang_minutes = 7\n").unwrap();

        let timings = RunTimings::from_config(config.watchdog());

        assert_eq!(
            timings,
            RunTimings {
                poll: Duration::from_secs(1),
                hang: Duration::from_mins(7),
                stop_grace: Duration::from_secs(10),
            }
        );
    }
}
