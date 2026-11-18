//! The history of the `agent` binary as a process, the way a person meets it: `agent daemon` in a temporary
//! `AGENTS_HOME` with a stand-in for the `claude` CLI. A history in the way is said once on the daemon's standard error and
//! shown by `agent status` while the task runs to its end; a daemon killed in the middle of a run (`SIGKILL`) leaves a
//! history that a daemon started again goes on with, a row for every run of the state and none twice.

#[cfg(test)]
mod history_process {
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output, Stdio};
    use std::time::{Duration, Instant};

    use agent_core::state::{RunEnd, State};
    use agent_runtime::history::Reader;
    use rustix::process::{Pid, Signal};

    /// The longest any wait of these tests lasts.
    const PATIENCE: Duration = Duration::from_secs(60);

    fn agent() -> Command {
        Command::new(env!("CARGO_BIN_EXE_agent"))
    }

    fn prompts() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts")
    }

    fn script(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
    }

    /// Runs git for the setup of a test.
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture Author")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A home with a scratch repository `example`, one agent and two investigators, and a daemon that is started when
    /// [`World::start`] is called; its standard error is kept in a file of the home.
    struct World {
        _dir: tempfile::TempDir,
        home: PathBuf,
        repo: PathBuf,
        daemon: Option<Child>,
    }

    impl World {
        fn new(scenario: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let home = root.join("home");
            std::fs::create_dir(&home).unwrap();
            let mut config = String::new();
            for (table, name, scenario) in [
                ("agents", "claude-1", scenario),
                ("investigators", "inv-opus", "sleep"),
                ("investigators", "inv-codex", "sleep"),
            ] {
                write!(
                    config,
                    "[{table}.{name}]\ncli = \"claude\"\nargs = [\"--scenario\", \"{scenario}\", \"--as\", \"{name}\", \"--home\", \"{}\"]\n",
                    home.display()
                )
                .unwrap();
            }
            write!(
                config,
                "[tiers]\nnormal = [\"claude-1\"]\n[binaries]\nclaude = \"{}\"\n[repos.example]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
                script("team-cli.sh").display(),
            )
            .unwrap();
            std::fs::write(home.join("agents.toml"), config).unwrap();
            let repo = root.join("example");
            std::fs::create_dir(&repo).unwrap();
            git(&repo, &["init", "-q", "-b", "main"]);
            git(
                &repo,
                &["remote", "add", "origin", "https://example.com/org/example.git"],
            );
            std::fs::write(repo.join("README.md"), "Hello\n").unwrap();
            git(&repo, &["add", "README.md"]);
            git(&repo, &["commit", "-q", "-m", "First"]);
            Self {
                _dir: dir,
                home,
                repo,
                daemon: None,
            }
        }

        fn log(&self) -> PathBuf {
            self.home.join("daemon.log")
        }

        /// Starts the daemon, its standard error appended to `daemon.log` in the home, and waits for its socket.
        fn start(&mut self) {
            let log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.log())
                .unwrap();
            // The socket of a daemon that was killed is still there: a wait for it would end before the new one listens.
            match std::fs::remove_file(self.home.join("agent.sock")) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("{error}"),
            }
            let daemon = agent()
                .arg("daemon")
                .arg("--prompts-dir")
                .arg(prompts())
                .env("AGENTS_HOME", &self.home)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .unwrap();
            let socket = self.home.join("agent.sock");
            eventually("the socket", || socket.exists());
            self.daemon = Some(daemon);
        }

        /// Ends the daemon by `signal` and waits for it.
        fn end(&mut self, signal: Signal) {
            let mut daemon = self.daemon.take().unwrap();
            let pid = Pid::from_raw(i32::try_from(daemon.id()).unwrap()).unwrap();
            rustix::process::kill_process(pid, signal).unwrap();
            daemon.wait().unwrap();
        }

        /// Runs `agent` with `args` in the scratch repository.
        fn agent(&self, args: &[&str]) -> Output {
            agent()
                .args(args)
                .current_dir(&self.repo)
                .env("AGENTS_HOME", &self.home)
                .output()
                .unwrap()
        }

        fn status(&self) -> String {
            let output = self.agent(&["status"]);
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            String::from_utf8(output.stdout).unwrap()
        }

        fn status_where(&self, what: &str, check: impl Fn(&str) -> bool) -> String {
            let deadline = Instant::now() + PATIENCE;
            loop {
                let status = self.status();
                if check(&status) {
                    return status;
                }
                assert!(Instant::now() < deadline, "timed out waiting for {what}:\n{status}");
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        fn state(&self) -> State {
            State::from_json(&std::fs::read_to_string(self.home.join("state.json")).unwrap()).unwrap()
        }

        fn runs(&self, task: &str) -> u32 {
            std::fs::read_to_string(self.home.join("worktrees/example").join(task).join(".fake-runs"))
                .map_or(0, |text| text.trim().parse().unwrap())
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            if self.daemon.is_some() {
                self.end(Signal::TERM);
            }
        }
    }

    /// Calls `check` until it returns true, for at most [`PATIENCE`].
    fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The task a `agent run` printed on its second line.
    fn first_task(output: &Output) -> String {
        let text = String::from_utf8(output.stdout.clone()).unwrap();
        text.lines()
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn a_history_in_the_way_is_said_once_on_stderr_the_task_runs_to_its_end_and_status_shows_the_lost_records() {
        let mut world = World::new("done");
        std::fs::create_dir(world.home.join("history.db")).unwrap();
        world.start();

        let output = world.agent(&["run", "claude-1", "Fix the typo in the README."]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let status = world.status_where("the task to be done", |status| {
            status.contains("solo") && status.contains("done")
        });

        let lines: Vec<&str> = status.lines().collect();
        let history_line = lines.last().unwrap();
        assert!(history_line.starts_with("history: not written: "), "{status}");
        assert!(history_line.contains("history.db"), "{history_line}");
        assert!(history_line.contains(" records lost"), "{history_line}");
        let lost: u64 = history_line
            .rsplit("; ")
            .next()
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(lost >= 3, "{history_line}");
        world.end(Signal::TERM);
        let log = std::fs::read_to_string(world.log()).unwrap();
        assert_eq!(log.matches("the history cannot keep some records").count(), 1, "{log}");
    }

    #[test]
    fn a_history_of_a_newer_version_is_said_once_on_stderr_at_start_and_left_untouched() {
        use agent_core::history::MaxTextBytes;
        use agent_runtime::history::{HistoryDb, Migration, schema::MIGRATIONS};

        let mut world = World::new("done");
        let mut migrations = MIGRATIONS.to_vec();
        let future_version = migrations.last().unwrap().version.checked_add(1).unwrap();
        migrations.push(Migration {
            version: future_version,
            sql: "create table from_the_future (x TEXT);",
        });
        let history = world.home.join("history.db");
        drop(HistoryDb::open_with(&history, &migrations, MaxTextBytes::default()).unwrap());
        let before = std::fs::read(&history).unwrap();
        world.start();

        let startup_log = std::fs::read_to_string(world.log()).unwrap();
        assert_eq!(
            startup_log.matches("the history cannot keep some records").count(),
            1,
            "{startup_log}"
        );
        assert!(
            startup_log.contains(&format!("version {future_version}")),
            "{startup_log}"
        );

        let output = world.agent(&["run", "claude-1", "Fix the typo in the README."]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let status = world.status_where("the task to be done", |status| {
            status.contains("solo") && status.contains("done")
        });
        assert!(status.contains(&format!("version {future_version}")), "{status}");
        assert!(lost_records(&status) >= 3, "{status}");
        world.end(Signal::TERM);

        let log = std::fs::read_to_string(world.log()).unwrap();
        assert_eq!(log.matches("the history cannot keep some records").count(), 1, "{log}");
        assert_eq!(std::fs::read(&history).unwrap(), before);
    }

    #[test]
    fn a_daemon_killed_in_the_middle_of_a_run_leaves_a_history_the_next_daemon_goes_on_with() {
        let mut world = World::new("slowdone,done");
        world.start();
        let output = world.agent(&["run", "claude-1", "Fix the typo in the README."]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let task = first_task(&output);
        eventually("the first run to have started", || world.runs(&task) >= 1);
        let history = world.home.join("history.db");
        eventually("the history to hold the run", || {
            Reader::open(&history).is_ok_and(|reader| {
                let state = world.state();
                state
                    .teams()
                    .first()
                    .is_some_and(|team| reader.runs_of_team(team.id()).is_ok_and(|runs| !runs.is_empty()))
            })
        });

        world.end(Signal::KILL);
        world.start();
        world.status_where("the task to be done", |status| {
            status.contains("solo") && status.contains("done")
        });
        world.end(Signal::TERM);

        let state = world.state();
        let team = state.teams()[0].id().clone();
        let runs: Vec<_> = state.teams()[0].crew().roster()[0]
            .1
            .runs()
            .iter()
            .map(|run| run.id().clone())
            .collect();
        assert!(runs.len() >= 2, "{runs:?}");
        let reader = Reader::open(&history).unwrap();
        let rows = reader.runs_of_team(&team).unwrap();
        let found: Vec<_> = rows.iter().map(|row| row.start.value.run.clone()).collect();
        assert_eq!(found, runs, "a row for every run of the state and none twice");
        assert_eq!(
            rows[0].end.as_ref().unwrap().value.end,
            RunEnd::NoStatus { exit_code: None }
        );
        assert!(rows[1..].iter().all(|row| row.end.is_some()));
    }
    /// The CLI health line carries a numeric count even when only some records fail.
    fn lost_records(status: &str) -> u64 {
        let line = status
            .lines()
            .find(|line| line.starts_with("history:"))
            .unwrap_or_else(|| panic!("missing history status: {status}"));
        line.rsplit("; ")
            .next()
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap()
    }

    /// Reap a deliberately broken daemon even when its graceful shutdown is the failing behavior.
    struct FailureWorld(World);

    impl Drop for FailureWorld {
        fn drop(&mut self) {
            if let Some(mut daemon) = self.0.daemon.take() {
                if let Err(error) = daemon.kill() {
                    eprintln!("cannot kill fixture daemon: {error}");
                }
                if let Err(error) = daemon.wait() {
                    eprintln!("cannot reap fixture daemon: {error}");
                }
            }
        }
    }

    #[test]
    fn sqlite_full_is_reported_once_on_stderr_and_the_task_finishes_with_visible_losses() {
        let mut guarded = FailureWorld(World::new("done"));
        let world = &mut guarded.0;
        world.start();
        let db = rusqlite::Connection::open(world.home.join("history.db")).unwrap();
        db.execute_batch(include_str!("fixtures/history-full.sql")).unwrap();
        let output = world.agent(&["run", "claude-1", "Finish the fixture task."]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let status = world.status_where("finished despite SQLite FULL", |status| {
            status
                .lines()
                .any(|line| line.contains("solo") && line.ends_with("done"))
                && lost_records(status) >= 2
        });
        assert!(lost_records(&status) >= 2, "{status}");
        world.end(Signal::TERM);
        let log = std::fs::read_to_string(world.log()).unwrap();
        assert_eq!(log.matches("the history cannot keep some records").count(), 1, "{log}");
        assert_eq!(log.matches("write_disk_full").count(), 1, "{log}");
        let reader = Reader::open(&world.home.join("history.db")).unwrap();
        let runs = reader.runs_of_team(world.state().teams()[0].id()).unwrap();
        assert_eq!(runs.len(), 1);
        assert!(runs[0].end.is_some());
        assert_eq!(reader.steps(&runs[0].start.value.run).unwrap(), Vec::new());
    }

    #[test]
    fn a_failing_insert_is_reported_once_on_stderr_and_the_task_finishes_with_visible_losses() {
        let mut guarded = FailureWorld(World::new("done"));
        let world = &mut guarded.0;
        world.start();
        let db = rusqlite::Connection::open(world.home.join("history.db")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_steps BEFORE INSERT ON steps BEGIN SELECT RAISE(FAIL, 'injected step failure'); END;").unwrap();
        let output = world.agent(&["run", "claude-1", "Finish the fixture task."]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let status = world.status_where("finished despite refused inserts", |status| {
            status
                .lines()
                .any(|line| line.contains("solo") && line.ends_with("done"))
                && lost_records(status) >= 2
        });
        assert!(lost_records(&status) >= 2, "{status}");
        world.end(Signal::TERM);
        let log = std::fs::read_to_string(world.log()).unwrap();
        assert_eq!(log.matches("the history cannot keep some records").count(), 1, "{log}");
        assert!(log.contains("write_refused"), "{log}");
        let reader = Reader::open(&world.home.join("history.db")).unwrap();
        let runs = reader.runs_of_team(world.state().teams()[0].id()).unwrap();
        assert_eq!(runs.len(), 1);
        assert!(runs[0].end.is_some());
    }

    #[test]
    fn a_full_history_queue_does_not_hold_the_task_and_reports_overflow_once() {
        let mut world = World::new("stream:burst");
        let scripts = world.home.join("fake-scripts");
        std::fs::create_dir(&scripts).unwrap();
        let event = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"one step\"}]}}\n";
        std::fs::write(scripts.join("burst.jsonl"), event.repeat(12_000)).unwrap();
        world.start();
        let db = rusqlite::Connection::open(world.home.join("history.db")).unwrap();
        db.execute_batch("BEGIN IMMEDIATE;").unwrap();
        let output = world.agent(&["run", "claude-1", "Finish the fixture task."]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        // The writer remains blocked until after the task is done: no timing assumption releases it.
        let status = world.status_where("finished while the history writer is blocked", |status| {
            status
                .lines()
                .any(|line| line.contains("solo") && line.ends_with("done"))
        });
        assert!(lost_records(&status) >= 1, "{status}");
        let log = std::fs::read_to_string(world.log()).unwrap();
        assert_eq!(log.matches("queue_full").count(), 1, "{log}");
        db.execute_batch("ROLLBACK;").unwrap();
        world.end(Signal::TERM);
        let log = std::fs::read_to_string(world.log()).unwrap();
        assert_eq!(log.matches("queue_full").count(), 1, "{log}");
        assert_eq!(
            db.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
    }
    /// Restore fixture permissions before its temporary home is removed.
    struct ReadOnlyScratch(PathBuf);

    impl Drop for ReadOnlyScratch {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt as _;
            if let Err(error) = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700)) {
                eprintln!("cannot restore scratch directory permissions: {error}");
            }
        }
    }

    #[test]
    fn failed_git_snapshots_report_once_and_status_counts_each_missing_record() {
        use std::os::unix::fs::PermissionsExt as _;

        let mut guarded = FailureWorld(World::new("done"));
        let world = &mut guarded.0;
        let scratch = world.home.join("readonly-scratch");
        std::fs::create_dir(&scratch).unwrap();
        std::fs::set_permissions(&scratch, std::fs::Permissions::from_mode(0o500)).unwrap();
        let _restore = ReadOnlyScratch(scratch.clone());
        assert_eq!(
            std::fs::File::create(scratch.join("probe")).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let log = std::fs::File::create(world.log()).unwrap();
        world.daemon = Some(
            agent()
                .args(["daemon", "--prompts-dir"])
                .arg(prompts())
                .env("AGENTS_HOME", &world.home)
                .env("TMPDIR", &scratch)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
        eventually("the socket", || world.home.join("agent.sock").exists());
        for _ in 0..2 {
            let output = world.agent(&["run", "claude-1", "Finish the fixture task."]);
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        }
        let status = world.status_where("both tasks done and missing snapshots counted", |status| {
            status
                .lines()
                .filter(|line| line.contains("solo") && line.ends_with("done"))
                .count()
                == 2
                && status.contains("history:")
                && lost_records(status) == 2
        });
        assert_eq!(lost_records(&status), 2);
        world.end(Signal::TERM);
        let log = std::fs::read_to_string(world.log()).unwrap();
        assert_eq!(log.matches("capture_changes").count(), 1, "{log}");
        let reader = Reader::open(&world.home.join("history.db")).unwrap();
        for team in world.state().teams() {
            let runs = reader.runs_of_team(team.id()).unwrap();
            assert_eq!(runs.len(), 1);
            assert!(runs[0].end.is_some());
            assert_eq!(reader.changes(&runs[0].start.value.run).unwrap(), None);
        }
    }
    #[test]
    fn log_follow_observes_runs_once_across_daemon_kill_restart_and_retention() {
        use agent_core::history::MaxTextBytes;
        use agent_runtime::history::{HistoryDb, Retention};
        use std::io::{BufRead, BufReader};
        use std::sync::mpsc;
        let mut world = World::new("gated,gated");
        world.start();
        let mut follower = agent()
            .args(["log", "-f", "--json"])
            .env("AGENTS_HOME", &world.home)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = follower.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                sender.send(line.unwrap()).unwrap();
            }
        });
        let next = || serde_json::from_str::<serde_json::Value>(&receiver.recv_timeout(PATIENCE).unwrap()).unwrap();
        let mut seen = vec![next()];
        while seen.last().unwrap()["event"] != "daemon_started" {
            seen.push(next());
        }
        let output = world.agent(&["run", "claude-1", "Watch this run."]);
        assert!(output.status.success(), "{output:?}");
        loop {
            seen.push(next());
            if seen.last().unwrap()["event"] == "run_started" {
                break;
            }
        }
        let first_run = seen.last().unwrap()["run"].clone();
        let task = first_task(&output);
        let worktree = world.home.join("worktrees/example").join(task.as_str());
        eventually("the fake CLI barrier", || worktree.join(".fake-committed-1").exists());
        world.end(Signal::KILL);
        // SIGKILL cannot clean up its child: release this test's fake CLI before restarting.
        std::fs::write(worktree.join(".fake-gate-done-1"), "done\n").unwrap();
        let path = world.home.join("history.db");
        let mut db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
        db.retain(&Retention {
            closed_before: None,
            logs_before: Some("2099-01-01T00:00:00Z".parse().unwrap()),
            home: world.home.clone(),
        })
        .unwrap();
        drop(db);
        world.start();
        loop {
            seen.push(next());
            if seen.last().unwrap()["event"] == "run_started" {
                break;
            }
        }
        assert_ne!(seen.last().unwrap()["run"], first_run);
        world.end(Signal::TERM);
        loop {
            seen.push(next());
            if seen.last().unwrap()["event"] == "daemon_stopped" {
                break;
            }
        }
        follower.kill().unwrap();
        follower.wait().unwrap();
        reader.join().unwrap();
        seen.extend(
            receiver
                .try_iter()
                .map(|line| serde_json::from_str::<serde_json::Value>(&line).unwrap()),
        );
        let ids: Vec<_> = seen.iter().map(|row| row["id"].as_i64().unwrap()).collect();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{ids:?}");
        let starts: Vec<_> = seen
            .iter()
            .filter(|row| row["event"] == "run_started")
            .map(|row| row["run"].as_str().unwrap().to_owned())
            .collect();
        let recorded: Vec<_> = Reader::open(&path)
            .unwrap()
            .daemon_logs()
            .unwrap()
            .iter()
            .filter(|row| row.event() == agent_core::log::LogEvent::RunStarted)
            .map(|row| row.run().unwrap().to_string())
            .collect();
        assert_eq!(starts, recorded);
        assert_eq!(starts.len(), 2);
    }
    /// Captures regular-file bytes and modification times, including SQLite side files.
    fn diagnostic_snapshot(
        root: &Path,
    ) -> std::io::Result<std::collections::BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)>> {
        let mut files = std::collections::BTreeMap::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(directory)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                let path = entry.path();
                if kind.is_dir() {
                    pending.push(path);
                } else if kind.is_file() {
                    let relative = path.strip_prefix(root).map_err(std::io::Error::other)?;
                    files.insert(
                        relative.to_path_buf(),
                        (std::fs::read(&path)?, entry.metadata()?.modified()?),
                    );
                }
            }
        }
        Ok(files)
    }

    #[test]
    fn debug_reads_written_history_and_lock_owner_without_changing_a_running_daemons_home() {
        use std::os::unix::fs::PermissionsExt;

        let mut world = World::new("done");
        std::fs::write(world.home.join("secrets.toml"), "").unwrap();
        std::fs::set_permissions(world.home.join("secrets.toml"), std::fs::Permissions::from_mode(0o600)).unwrap();
        world.start();
        let output = world.agent(&["run", "claude-1", "Diagnose the fixture task."]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        world.status_where("the task to finish", |status| {
            status.contains("solo") && status.contains("done")
        });
        let state = world.state();
        let team = state.teams().first().unwrap().id();
        // Use the existing readiness helper: the event writer can finish just after the status transition.
        eventually("the final history record", || {
            Reader::open(&world.home.join("history.db")).is_ok_and(|reader| {
                reader
                    .runs_of_team(team)
                    .is_ok_and(|runs| !runs.is_empty() && runs.iter().all(|run| run.end.is_some()))
            })
        });
        let destination = tempfile::tempdir().unwrap();
        let before = diagnostic_snapshot(&world.home).unwrap();
        let pid = world.daemon.as_ref().unwrap().id();
        let output = world.agent(&[
            "debug",
            "--team",
            team.as_str(),
            "--out",
            destination.path().to_str().unwrap(),
        ]);
        let after = diagnostic_snapshot(&world.home).unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(
            before, after,
            "debug must preserve the home, including database, WAL and SHM bytes"
        );
        let bundle = std::fs::read_dir(destination.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let summary = std::fs::read_to_string(bundle.join("SUMMARY.md")).unwrap();
        let process = std::fs::read_to_string(bundle.join("daemon-process.txt")).unwrap();
        let history = std::fs::read_to_string(bundle.join("history.txt")).unwrap();
        assert!(process.contains(&pid.to_string()), "{process}");
        assert!(!summary.contains("No daemon holds this home."), "{summary}");
        assert!(history.contains("Finished the work."), "{history}");
        assert!(summary.contains("Finished the work."), "{summary}");
        world.end(Signal::TERM);
    }
}
