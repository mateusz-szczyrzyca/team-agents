//! The count of a session's searches of code without Serena across the runs of the engine: the steps the history holds.

#![expect(
    clippy::expect_used,
    reason = "a failed expectation is a failed test, as the workspace tests allow (clippy.toml)"
)]

use std::num::NonZeroU32;
use std::str::FromStr;

use agent_core::cli::ActivityEvent;
use agent_core::config::AbsolutePath;
use agent_core::history::{AgentIdentity, MaxTextBytes, Record, RunStart, RunWhy, Step};
use agent_core::ids::{RunId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::serena::RemindAfter;
use agent_core::state::Timestamp;
use agent_runtime::history::{HistoryDb, Reader, Store};
use agent_runtime::run::serena::{SessionWatch, Watched};

fn at() -> Timestamp {
    Timestamp::from_str("2026-11-24T10:00:00Z").expect("a time")
}

/// A search of code of the shell.
fn search(number: u32) -> Record {
    Record::Step(Box::new(Step {
        run: run(),
        number: NonZeroU32::new(number).expect("a number"),
        at: at(),
        event: ActivityEvent::Command(format!("cat src/file{number}.rs")),
    }))
}

fn run() -> RunId {
    RunId::from_str("run-20261124-100000-r1r1").expect("a run")
}

fn team() -> TeamId {
    TeamId::from_str("team-20261124-100000-t1t1").expect("a team")
}

fn task() -> TaskId {
    TaskId::from_str("task-20261124-100000-t1t1").expect("a task")
}

fn start() -> Record {
    Record::Run(Box::new(RunStart {
        run: run(),
        team: team(),
        task: task(),
        at: at(),
        agent: AgentIdentity {
            name: "claude-1".to_owned(),
            tier: Tier::Normal,
            cli: CliKind::Claude,
            model: None,
        },
        role: Role::Solo,
        rounds: vec![],
        why: RunWhy::FirstRun,
        prompt: "Do the item.".to_owned(),
        worktree: AbsolutePath::try_from("/work/tree".to_owned()).expect("a path"),
        branch: "main".parse().expect("a branch"),
        session: None,
    }))
}

#[test]
fn a_reminder_of_a_run_that_ended_is_read_from_the_history_of_its_task() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("history.db");
    let mut database = HistoryDb::open(&path, MaxTextBytes::default()).expect("a database");
    database.write(&[start(), search(1)]).expect("a write");
    drop(database);

    let reader = Reader::open(&path).expect("a reader");
    let steps = reader.steps(&run()).expect("the steps of the run");

    assert_eq!(steps.len(), 1);
    let mut tally = agent_core::serena::SessionTally::new();
    let after = RemindAfter::new(2).expect("2");
    assert!(tally.note(&steps[0].value.event, after).is_none());
    assert_eq!(
        tally.note(&ActivityEvent::Command("cat src/other.rs".to_owned()), after),
        Some(agent_core::serena::Reminder { searches: 2 })
    );
}

#[test]
fn a_watch_of_a_session_that_goes_on_is_not_reminded_again() {
    let mut watch = SessionWatch::after_reminder(12);
    let after = RemindAfter::new(12).expect("12");

    let told: Vec<Watched> = (0..12)
        .map(|_| watch.note(&ActivityEvent::Command("cat src/main.rs".to_owned()), after))
        .collect();

    assert_eq!(told, vec![Watched::Quiet; 12]);
}

/// The start of the run `run()` in the session `session`.
fn start_in(session: &str) -> Record {
    let Record::Run(mut start) = start() else {
        unreachable!("start() is the start of a run")
    };
    start.session = Some(session.parse().expect("a session"));
    Record::Run(start)
}

#[test]
fn a_session_run_under_a_higher_bound_is_reminded_when_it_reaches_a_lower_one() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("history.db");
    let mut database = HistoryDb::open(&path, MaxTextBytes::default()).expect("a database");
    let mut records = vec![start_in("session-1")];
    records.extend((1..=40).map(search));
    database.write(&records).expect("a write");
    drop(database);
    let lower = RemindAfter::new(40).expect("40");

    let mut watch = SessionWatch::from_history(&path, &task(), "session-1", lower).expect("the history of the session");

    assert_eq!(
        watch.note(&ActivityEvent::Command("cat src/one_more.rs".to_owned()), lower),
        Watched::Due { searches: 41 }
    );
}
