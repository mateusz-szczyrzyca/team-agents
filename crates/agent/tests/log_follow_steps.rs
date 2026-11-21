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
    use agent_core::history::{AgentIdentity, MaxTextBytes, Record, RunEndRecord, RunStart, RunWhy, Step, TeamStart};
    use agent_core::ids::{RoundId, RunId, TaskId, TeamId};
    use agent_core::kinds::{CliKind, Role, Tier};
    use agent_core::log::{LogEvent, LogRecord, LogTaskScope};
    use agent_core::state::{RunEnd, Timestamp};
    use agent_runtime::history::{HistoryDb, LogFilter, LogWindow, Reader, Store};

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

    /// Gives the following the time to read a line it was not asked for, and to stop before a writer commits.
    fn settle() {
        eprintln!("SETTLE");
        std::thread::sleep(std::time::Duration::from_millis(200));
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
            let line = self.lines.recv_timeout(WAIT).expect("the following prints a line");
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/trace.log")
            {
                use std::io::Write as _;
                let _ = writeln!(file, "LINE {line}");
            }
            line
        }

        /// Stops the following.
        fn stop(mut self) {
            self.child.kill().expect("the following stops");
            self.child.wait().expect("the following is reaped");
            self.reader.join().expect("the reading thread ends");
        }
    }

    #[test]
    fn a_follow_prints_the_steps_of_a_run_as_they_are_recorded() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("history.db");
        let mut db = history(&path);

        let follower = Follower::start(dir.path(), &["--task", "task-20261121-132909-0001", "-n", "1"]);
        settle();
        db.write(&[step(2, 3, ActivityEvent::Command("cargo test".to_owned()))])
            .expect("the step is written");

        let opening = follower.line();
        settle();
        assert!(
            opening.ends_with(" run run-20261121-132909-0001 started by claude-1 on task task-20261121-132909-0001 of team team-20261121-132909-4qem (solo) in round round-20261121-132909-q1w2"),
            "{opening}"
        );
        assert_eq!(follower.line(), "2026-11-21T12:50:03Z command: cargo test");
        follower.stop();
    }

    #[test]
    fn a_follow_opens_with_the_newest_lines_and_goes_on_with_the_rows() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("history.db");
        let mut db = history(&path);
        db.write_logs(&[row(2)]).expect("the row is written");

        let follower = Follower::start(dir.path(), &["--task", "task-20261121-132909-0001"]);
        let opening = [follower.line(), follower.line()];
        settle();
        assert_eq!(
            [opening[0].clone(), opening[1].clone()],
            [
                "2026-11-21T12:50:01Z command: cargo build".to_owned(),
                "2026-11-21T12:50:02Z info task_status_changed task-20261121-132909-0001 team team-20261121-132909-4qem rounds round-20261121-132909-q1w2: running ← queued".to_owned(),
            ]
        );
        db.write_logs(&[row(5)]).expect("the next row is written");
        assert_eq!(
            follower.line(),
            "2026-11-21T12:50:05Z info task_status_changed task-20261121-132909-0001 team team-20261121-132909-4qem rounds round-20261121-132909-q1w2: running ← queued"
        );
        follower.stop();
    }

    #[test]
    fn a_follow_says_how_the_run_ended() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("history.db");
        let mut db = history(&path);

        let follower = Follower::start(dir.path(), &["--task", "task-20261121-132909-0001", "-n", "0"]);
        settle();
        db.write(&[Record::RunEnd(Box::new(RunEndRecord {
            run: run(),
            at: at(4),
            end: RunEnd::Reported {
                outcome: agent_core::status::RunOutcome::Done,
            },
            final_message: Some("All done.\nSTATUS: DONE".to_owned()),
        }))])
        .expect("the end is written");

        // The opening of the text comes before the new rows, and `-n 0` leaves it empty of rows.
        settle();
        let end = follower.line();
        assert!(
            end.starts_with("2026-11-21T12:50:04Z info run_ended task-20261121-132909-0001 (claude-1)"),
            "{end}"
        );
        assert!(end.ends_with(": ended reported DONE"), "{end}");
        follower.stop();
    }

    #[test]
    fn a_follow_with_no_task_prints_a_row_alone() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("history.db");
        let mut db = history(&path);
        db.write_logs(&[row(2)]).expect("the row is written");

        let follower = Follower::start(dir.path(), &["-n", "1"]);
        settle();
        db.write_logs(&[row(5)]).expect("the row is written");

        assert_eq!(
            follower.line(),
            "2026-11-21T12:50:02Z info task_status_changed task-20261121-132909-0001 team team-20261121-132909-4qem rounds round-20261121-132909-q1w2: running ← queued"
        );
        assert_eq!(
            follower.line(),
            "2026-11-21T12:50:05Z info task_status_changed task-20261121-132909-0001 team team-20261121-132909-4qem rounds round-20261121-132909-q1w2: running ← queued"
        );
        follower.stop();
    }

    #[test]
    fn a_follow_of_a_task_reads_a_new_table_after_the_daemon_restarted_the_writer() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("history.db");
        let db = history(&path);

        let follower = Follower::start(dir.path(), &["--task", "task-20261121-132909-0001", "-n", "0"]);
        settle();
        drop(db);
        let mut db = HistoryDb::open(&path, MaxTextBytes::default()).expect("the history reopens");
        db.write_logs(&[row(6)]).expect("the row is written");

        assert_eq!(
            follower.line(),
            "2026-11-21T12:50:06Z info task_status_changed task-20261121-132909-0001 team team-20261121-132909-4qem rounds round-20261121-132909-q1w2: running ← queued"
        );
        follower.stop();
        let reader = Reader::open(&path).expect("the history reads");
        let rows = reader
            .log_rows(&LogFilter::default(), LogWindow::Newest(10))
            .expect("the rows are read");
        assert_eq!(rows.len(), 1);
    }
}
