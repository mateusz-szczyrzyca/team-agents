//! A follow prints the steps of the runs it covers as they are recorded, the end of a run and what is waited for.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader};
    use std::num::NonZeroU32;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    use agent_core::cli::ActivityEvent;
    use agent_core::history::{AgentIdentity, MaxTextBytes, Record, RunStart, RunWhy, Step, TeamStart};
    use agent_core::ids::{RoundId, RunId, TaskId, TeamId};
    use agent_core::kinds::{CliKind, Role, Tier};
    use agent_core::log::{LogEvent, LogRecord, LogTaskScope};
    use agent_core::state::Timestamp;
    use agent_runtime::history::{HistoryDb, Store};

    /// How long a test waits for the following to print a line the daemon has written.
    const WAIT: Duration = Duration::from_secs(10);

    /// When `second` is, after 12:50:00 UTC.
    fn at(second: u32) -> Timestamp {
        format!("2026-11-21T12:50:{second:02}Z")
            .parse()
            .expect("a literal time is valid")
    }

    /// The task of the history.
    fn task() -> TaskId {
        "task-20261121-132909-0001".parse().expect("a literal id is valid")
    }

    /// The run of the history.
    fn run() -> RunId {
        "run-20261121-132909-0001".parse().expect("a literal id is valid")
    }

    /// The team of the history.
    fn team() -> TeamId {
        "team-20261121-132909-4qem".parse().expect("a literal id is valid")
    }

    /// The round of the history.
    fn round() -> RoundId {
        "round-20261121-132909-q1w2".parse().expect("a literal id is valid")
    }

    /// The start of the run, with `why`.
    fn start(why: RunWhy) -> RunStart {
        RunStart {
            run: run(),
            team: team(),
            task: task(),
            at: at(0),
            agent: AgentIdentity {
                name: "claude-1".to_owned(),
                tier: Tier::Normal,
                cli: CliKind::Claude,
                model: None,
            },
            role: Role::Solo,
            rounds: vec![round()],
            why,
            prompt: String::new(),
            worktree: "/home/example/worktrees/example/task"
                .parse()
                .expect("a literal path is valid"),
            branch: "agents/example/task".parse().expect("a literal branch is valid"),
            session: None,
        }
    }

    /// The `number`th step of the run, at `second`.
    fn step(number: u32, second: u32, event: ActivityEvent) -> Record {
        Record::Step(Box::new(Step {
            run: run(),
            number: NonZeroU32::new(number).expect("a step number starts at one"),
            at: at(second),
            event,
        }))
    }

    /// A status row of the task at `second`.
    fn row(second: u32) -> LogRecord {
        LogRecord {
            at: at(second),
            module: "test".to_owned(),
            event: LogEvent::TaskStatusChanged,
            team: Some(agent_core::log::LogTeamScope::new(team(), [round()])),
            task: Some(LogTaskScope::new(task(), None)),
            message: "task status changed".to_owned(),
            fields: [
                ("from".to_owned(), serde_json::json!("queued")),
                ("to".to_owned(), serde_json::json!("running")),
            ]
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
            cut: agent_core::history::Cut::default(),
        }
    }

    /// The history: a team, a round and a first run with a step before the follow starts.
    fn history(path: &Path) -> HistoryDb {
        let mut db = HistoryDb::open(path, MaxTextBytes::default()).expect("a temporary history opens");
        db.write(&[
            Record::Team(Box::new(TeamStart {
                team: team(),
                at: at(0),
                repo: "example".to_owned(),
                base_branch: "main".parse().expect("a literal branch is valid"),
                merge: agent_core::state::MergeMode::NoMerge,
                task_list: String::new(),
                plan: Vec::new(),
            })),
            Record::Run(Box::new(start(RunWhy::FirstRun))),
            step(1, 1, ActivityEvent::Command("cargo build".to_owned())),
        ])
        .expect("the first run is written");
        db
    }

    /// A following of `agent log` in `home` that ends when the child does.
    struct Follower {
        /// The command.
        child: Child,
        /// Its lines, as it prints them.
        lines: Receiver<String>,
        /// The thread that reads them.
        reader: std::thread::JoinHandle<()>,
    }

    impl Follower {
        /// Starts `agent log` with `args` in `home`.
        fn start(home: &Path, args: &[&str]) -> Self {
            let mut child = Command::new(env!("CARGO_BIN_EXE_agent"))
                .env_clear()
                .env("AGENTS_HOME", home)
                .args(["log", "--follow"])
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("agent log follows");
            let stdout = child.stdout.take().expect("the following has output");
            let (sender, lines) = channel();
            let reader = std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    match line {
                        Ok(line) => {
                            if sender.send(line).is_err() {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                }
            });
            Self { child, lines, reader }
        }

        /// The next line the following prints.
        fn line(&self) -> String {
            self.lines.recv_timeout(WAIT).expect("the following prints a line")
        }

        /// Stops the following.
        fn stop(mut self) {
            self.child.kill().expect("the following stops");
            self.child.wait().expect("the following is reaped");
            self.reader.join().expect("the reading thread ends");
        }
    }

    /// A row of `event` of the task and of its run, at `second`.
    fn run_row(event: LogEvent, second: u32) -> LogRecord {
        let mut record = row(second);
        record.event = event;
        record.task = Some(LogTaskScope::new(task(), Some(run())));
        record.fields = [
            ("agent".to_owned(), serde_json::json!("claude-1")),
            ("end".to_owned(), serde_json::json!("Reported { outcome: Done }")),
        ]
        .into_iter()
        .collect();
        record
    }

    #[test]
    fn a_follow_prints_the_steps_of_a_run_as_they_are_recorded_once_each() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("history.db");
        let mut db = history(&path);

        let follower = Follower::start(dir.path(), &["--task", "task-20261121-132909-0001", "-n", "1"]);
        // The opening is the newest line of the scope, with the run it belongs to: the follow has read the history.
        assert!(
            follower
                .line()
                .contains(" run run-20261121-132909-0001 started by claude-1")
        );
        assert_eq!(follower.line(), "2026-11-21T12:50:01Z command: cargo build");
        db.write(&[step(2, 3, ActivityEvent::Command("cargo test".to_owned()))])
            .expect("the step is written");
        assert_eq!(follower.line(), "2026-11-21T12:50:03Z command: cargo test");
        db.write(&[step(
            3,
            4,
            ActivityEvent::Text("The tests pass.\nNext line.".to_owned()),
        )])
        .expect("the step is written");
        assert_eq!(follower.line(), "2026-11-21T12:50:04Z The tests pass.…");
        follower.stop();
    }

    #[test]
    fn a_follow_says_how_a_run_ended_and_goes_on_with_the_next_run_of_the_task() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("history.db");
        let mut db = history(&path);

        let follower = Follower::start(dir.path(), &["--task", "task-20261121-132909-0001", "-n", "1"]);
        assert!(
            follower
                .line()
                .contains(" run run-20261121-132909-0001 started by claude-1")
        );
        assert_eq!(follower.line(), "2026-11-21T12:50:01Z command: cargo build");
        db.write_logs(&[run_row(LogEvent::RunEnded, 5)])
            .expect("the end is written");
        let end = follower.line();
        assert!(end.starts_with("2026-11-21T12:50:05Z info run_ended "), "{end}");
        assert!(end.ends_with(": ended reported DONE"), "{end}");
        // No daemon listens in the temporary home: the line says that it was not asked.
        let wait = follower.line();
        assert!(
            wait.starts_with(
                "round round-20261121-132909-q1w2: what it waits for is not known, the daemon was not asked: "
            ),
            "{wait}"
        );
        let mut second = start(RunWhy::Continuation);
        second.run = "run-20261121-132909-0002".parse().expect("a literal id is valid");
        second.at = at(9);
        db.write(&[
            Record::Run(Box::new(second)),
            Record::Step(Box::new(Step {
                run: "run-20261121-132909-0002".parse().expect("a literal id is valid"),
                number: NonZeroU32::new(1).expect("a step number starts at one"),
                at: at(10),
                event: ActivityEvent::Command("git status".to_owned()),
            })),
        ])
        .expect("the next run is written");
        assert!(
            follower
                .line()
                .contains(" run run-20261121-132909-0002 started by claude-1")
        );
        assert_eq!(follower.line(), "2026-11-21T12:50:10Z command: git status");
        follower.stop();
    }

    #[test]
    fn a_follow_with_no_steps_prints_the_records_alone() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("history.db");
        let mut db = history(&path);
        db.write_logs(&[row(2)]).expect("the row is written");

        let follower = Follower::start(
            dir.path(),
            &["--task", "task-20261121-132909-0001", "-n", "1", "--no-steps"],
        );
        assert!(
            follower
                .line()
                .starts_with("2026-11-21T12:50:02Z info task_status_changed ")
        );
        db.write(&[step(2, 3, ActivityEvent::Command("cargo test".to_owned()))])
            .expect("the step is written");
        db.write_logs(&[row(4)]).expect("the row is written");
        assert!(
            follower
                .line()
                .starts_with("2026-11-21T12:50:04Z info task_status_changed ")
        );
        follower.stop();
    }

    #[test]
    fn a_follow_goes_on_after_the_writer_of_the_history_was_restarted() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("history.db");
        let db = history(&path);

        let follower = Follower::start(dir.path(), &["--task", "task-20261121-132909-0001", "-n", "1"]);
        assert!(
            follower
                .line()
                .contains(" run run-20261121-132909-0001 started by claude-1")
        );
        assert_eq!(follower.line(), "2026-11-21T12:50:01Z command: cargo build");
        drop(db);
        let mut db = HistoryDb::open(&path, MaxTextBytes::default()).expect("the history reopens");
        db.write(&[step(2, 3, ActivityEvent::Command("cargo test".to_owned()))])
            .expect("the step is written");
        assert_eq!(follower.line(), "2026-11-21T12:50:03Z command: cargo test");
        follower.stop();
    }

    #[test]
    fn a_follow_of_a_round_names_the_agent_of_each_step() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("history.db");
        let mut db = history(&path);
        let mut other = start(RunWhy::FirstRun);
        other.run = "run-20261121-132909-0002".parse().expect("a literal id is valid");
        other.task = "task-20261121-132909-0002".parse().expect("a literal id is valid");
        other.agent.name = "codex-1".to_owned();
        db.write(&[
            Record::Run(Box::new(other)),
            Record::Step(Box::new(Step {
                run: "run-20261121-132909-0002".parse().expect("a literal id is valid"),
                number: NonZeroU32::new(1).expect("a step number starts at one"),
                at: at(2),
                event: ActivityEvent::Command("cargo clippy".to_owned()),
            })),
        ])
        .expect("the second run is written");

        let follower = Follower::start(dir.path(), &["--round", "round-20261121-132909-q1w2", "-n", "10"]);
        let mut found = Vec::new();
        while !found
            .iter()
            .any(|line: &String| line.ends_with("command: cargo clippy"))
        {
            found.push(follower.line());
        }
        follower.stop();

        assert!(
            found.contains(&"2026-11-21T12:50:01Z claude-1 command: cargo build".to_owned()),
            "{found:?}"
        );
        assert!(
            found.contains(&"2026-11-21T12:50:02Z codex-1 command: cargo clippy".to_owned()),
            "{found:?}"
        );
    }
}
