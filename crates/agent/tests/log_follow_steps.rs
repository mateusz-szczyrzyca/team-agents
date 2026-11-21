//! A follow prints the steps of the runs it covers as they are recorded, the end of a run and what is waited for.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::num::NonZeroU32;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use agent_core::cli::ActivityEvent;
use agent_core::history::{
    AgentIdentity, MaxTextBytes, Record, RunChanges, RunEndRecord, RunStart, RunWhy, Step, TeamStart,
};
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

/// The history: a team, a round, a first run with a step before the follow starts and a status row.
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
    db.write_logs(&[row(2)]).expect("the row is written");
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
        Self {
            child,
            lines,
            reader,
        }
    }

    /// The next line the following prints.
    fn line(&self) -> String {
        self.lines
            .recv_timeout(WAIT)
            .expect("the following prints a line")
    }

    /// Stops the following and gives the lines it printed that a test did not take.
    fn stop(mut self) -> Vec<String> {
        self.child.kill().expect("the following stops");
        self.child.wait().expect("the following is reaped");
        self.reader.join().expect("the reading thread ends");
        self.lines.try_iter().collect()
    }
}

#[test]
fn a_follow_of_a_task_prints_the_run_of_a_step_before_the_step() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = history(&path);

    let follower = Follower::start(dir.path(), &["--task", "task-20261121-132909-0001", "-n", "0"]);
    db.write(&[step(2, 3, ActivityEvent::Command("cargo test".to_owned()))])
        .expect("the step is written");

    let run = follower.line();
    assert_eq!(
        run,
        "2026-11-21T12:50:03Z run run-20261121-132909-0001 started by claude-1 on task task-20261121-132909-0001 of team team-20261121-132909-4qem (solo) in round round-20261121-132909-q1w2"
    );
    assert_eq!(follower.line(), "2026-11-21T12:50:03Z command: cargo test");
    let rest = follower.stop();
    assert_eq!(rest, Vec::<String>::new());
}

#[test]
fn a_follow_of_a_task_opens_with_the_newest_lines_and_goes_on_with_rows() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = history(&path);

    let follower = Follower::start(dir.path(), &["--task", "task-20261121-132909-0001"]);
    let opening = [follower.line(), follower.line()];
    assert_eq!(
        opening,
        [
            "2026-11-21T12:50:01Z command: cargo build".to_owned(),
            "2026-11-21T12:50:02Z info task_status_changed task-20261121-132909-0001 team team-20261121-132909-4qem rounds round-20261121-132909-q1w2: running ← queued".to_owned(),
        ]
    );
    db.write(&[Record::Run(Box::new(start(RunWhy::Continuation)))])
        .expect("the next run is written");
    assert_eq!(
        follower.line(),
        "2026-11-21T12:50:00Z run run-20261121-132909-0001 started by claude-1 on task task-20261121-132909-0001 of team team-20261121-132909-4qem (solo) in round round-20261121-132909-q1w2"
    );
    let rest = follower.stop();
    assert_eq!(rest, Vec::<String>::new());
}

#[test]
fn a_follow_of_a_task_says_how_the_run_ended_and_what_waits_next() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = history(&path);

    let follower = Follower::start(dir.path(), &["--task", "task-20261121-132909-0001", "-n", "0"]);
    db.write(&[
        Record::RunEnd(Box::new(RunEndRecord {
            run: run(),
            at: at(4),
            end: RunEnd::Reported {
                outcome: agent_core::status::RunOutcome::Done,
            },
            final_message: Some("All done.\nSTATUS: DONE".to_owned()),
        })),
        Record::Changes(Box::new(RunChanges {
            run: run(),
            head_before: None,
            head_after: None,
            commits: Vec::new(),
            files: Vec::new(),
            uncommitted: Vec::new(),
        })),
    ])
    .expect("the end is written");

    let found = [follower.line(), follower.line()];
    assert!(
        found[0].starts_with("2026-11-21T12:50:00Z run run-20261121-132909-0001 started by claude-1"),
        "{found:?}"
    );
    assert!(
        found[1].contains("ended reported DONE") || found[1].contains("changes"),
        "{found:?}"
    );
    follower.stop();
}

#[test]
fn a_follow_with_no_task_prints_a_row_alone() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = history(&path);

    let follower = Follower::start(dir.path(), &["-n", "0"]);
    db.write_logs(&[row(5)]).expect("the row is written");

    assert_eq!(
        follower.line(),
        "2026-11-21T12:50:05Z info task_status_changed task-20261121-132909-0001 team team-20261121-132909-4qem rounds round-20261121-132909-q1w2: running ← queued"
    );
    assert_eq!(follower.stop(), Vec::<String>::new());
}

#[test]
fn a_follow_of_a_task_reads_a_new_table_after_the_daemon_restarted_the_writer() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let mut db = history(&path);

    let follower = Follower::start(dir.path(), &["--task", "task-20261121-132909-0001", "-n", "0"]);
    drop(db);
    let mut db = HistoryDb::open(&path, MaxTextBytes::default()).expect("the history reopens");
    db.write_logs(&[row(6)]).expect("the row is written");

    assert_eq!(
        follower.line(),
        "2026-11-21T12:50:06Z info task_status_changed task-20261121-132909-0001 team team-20261121-132909-4qem rounds round-20261121-132909-q1w2: running ← queued"
    );
    assert_eq!(follower.stop(), Vec::<String>::new());
    let reader = Reader::open(&path).expect("the history reads");
    let rows = reader
        .log_rows(&LogFilter::default(), LogWindow::Newest(10))
        .expect("the rows are read");
    assert_eq!(rows.len(), 2);
}
