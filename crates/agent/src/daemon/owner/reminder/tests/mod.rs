//! The reminder of Serena that the history holds for a run, as the daemon reads it back.
//!
//! The records of a history come in the order the daemon happened to write them: a daemon that slept between two turns writes
//! the reminder of the run it cut after the round of the next item, or after the start of that run itself when it is briefed.
//! A reminder therefore belongs to its run while no other run of the team starts after it in that order.

use std::str::FromStr;
use std::sync::Arc;

use agent_core::config::AbsolutePath;
use agent_core::history::{
    AgentIdentity, ItemText, MaxTextBytes, Record, RoundOpening, RoundSubject, RunStart, RunWhy,
};
use agent_core::ids::{BranchName, Clock, ItemId, RoundId, RunId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::state::Timestamp;
use agent_runtime::history::{DEFAULT_QUEUE, History, HistoryDb, Reader, Setup, TracingReporter};
use tempfile::TempDir;
use time::UtcDateTime;

use super::reminder_of;

/// The team of the fixture.
fn team() -> TeamId {
    TeamId::from_str("team-20261124-100000-t0t0").expect("the team")
}

/// Another team of the fixture.
fn other_team() -> TeamId {
    TeamId::from_str("team-20261124-100000-t9t9").expect("the other team")
}

/// The task of the fixture.
fn task() -> TaskId {
    TaskId::from_str("task-20261124-100000-t1t1").expect("the task")
}

/// Another task of the fixture.
fn other_task() -> TaskId {
    TaskId::from_str("task-20261124-100000-t2t2").expect("the other task")
}

/// The time `seconds` after the fixture's start.
fn at(seconds: i64) -> Timestamp {
    Timestamp::from_utc(UtcDateTime::from_unix_timestamp(1_763_978_400 + seconds).expect("a time")).expect("a time")
}

/// A clock that gives the time `seconds` after the fixture's start.
struct Later(i64);

impl Clock for Later {
    fn now(&self) -> UtcDateTime {
        UtcDateTime::from_unix_timestamp(1_763_978_400 + self.0).expect("a time")
    }
}

/// A history in a directory of its own, with the records `write` hands to it, written and closed, and its reader.
fn history(write: impl FnOnce(&History)) -> (TempDir, Reader) {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("history.db");
    let db = HistoryDb::open(&path, MaxTextBytes::default()).expect("a history");
    let started = History::start(
        db,
        Setup {
            reporter: Arc::new(TracingReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .expect("the history starts");
    write(&started.history);
    started.history.flush();
    started.writer.close().expect("the history closes");
    let reader = Reader::open(&path).expect("the history opens for reading");
    (dir, reader)
}

/// Records the start of `run` of `task` in `team`, at `seconds` after the fixture's start.
fn run_start(history: &History, run: RunId, team: TeamId, task: TaskId, seconds: i64) {
    history.record(Record::Run(Box::new(RunStart {
        run,
        team,
        task,
        at: at(seconds),
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
        worktree: AbsolutePath::from_str("/work").expect("a path"),
        branch: BranchName::from_str("main").expect("a branch"),
        session: None,
    })));
}

/// Records the opening of a round of `team` on item `item`, at `seconds` after the fixture's start.
fn round_opened(history: &History, team: TeamId, item: &str, seconds: i64) {
    history.record(Record::Round(Box::new(RoundOpening {
        round: RoundId::from_str(&format!("round-20261124-1000{seconds:02}-r0r0")).expect("a round"),
        team,
        at: at(seconds),
        subject: RoundSubject::Item(ItemText {
            item: ItemId::from_str(item).expect("an item"),
            title: "The item".to_owned(),
            description: "Do the thing.".to_owned(),
            expected_result: None,
            checks: None,
        }),
    })));
}

#[test]
fn the_reminder_of_the_run_that_ended_is_the_count_and_the_cli_the_history_holds() {
    let run = RunId::from_str("run-20261124-100001-r1r1").expect("a run");
    let (_dir, reader) = history(|history| {
        run_start(history, run.clone(), team(), task(), 0);
        history.serena_reminded(&run, &task(), CliKind::Opencode, 40, 7, &Later(10));
    });

    let found = reminder_of(&reader, &run, &team());

    assert_eq!(found.expect("the history reads"), Some((40, CliKind::Opencode)));
}

#[test]
fn a_run_the_history_never_started_is_not_reminded() {
    let run = RunId::from_str("run-20261124-100001-r1r1").expect("a run");
    let (_dir, reader) = history(|history| {
        run_start(
            history,
            RunId::from_str("run-20261124-100002-r2r2").expect("a run"),
            team(),
            task(),
            0,
        );
        history.serena_reminded(&run, &task(), CliKind::Claude, 40, 7, &Later(10));
    });

    let found = reminder_of(&reader, &run, &team());

    assert_eq!(found.expect("the history reads"), None);
}

#[test]
fn the_reminder_of_another_run_of_the_task_is_not_this_runs() {
    let run = RunId::from_str("run-20261124-100002-r2r2").expect("a run");
    let (_dir, reader) = history(|history| {
        history.serena_reminded(
            &RunId::from_str("run-20261124-100001-r1r1").expect("a run"),
            &task(),
            CliKind::Claude,
            12,
            7,
            &Later(0),
        );
        run_start(history, run.clone(), team(), task(), 20);
        history.serena_reminded(&run, &task(), CliKind::Opencode, 40, 7, &Later(30));
    });

    let found = reminder_of(&reader, &run, &team());

    assert_eq!(found.expect("the history reads"), Some((40, CliKind::Opencode)));
}

#[test]
fn the_run_of_the_reminder_may_be_written_after_the_reminder() {
    let run = RunId::from_str("run-20261124-100001-r1r1").expect("a run");
    let (_dir, reader) = history(|history| {
        // The daemon cut the run, wrote its end and its reminder, and only then looked at the team that briefs the run it
        // starts again, the same run in the state.
        history.serena_reminded(&run, &task(), CliKind::Claude, 40, 7, &Later(10));
        run_start(history, run.clone(), team(), task(), 20);
    });

    let found = reminder_of(&reader, &run, &team());

    assert_eq!(found.expect("the history reads"), Some((40, CliKind::Claude)));
}

#[test]
fn the_next_run_of_the_task_ends_the_reminder_of_this_one() {
    let run = RunId::from_str("run-20261124-100001-r1r1").expect("a run");
    let (_dir, reader) = history(|history| {
        run_start(history, run.clone(), team(), task(), 0);
        history.serena_reminded(&run, &task(), CliKind::Claude, 40, 7, &Later(10));
        run_start(
            history,
            RunId::from_str("run-20261124-100002-r2r2").expect("a run"),
            team(),
            task(),
            20,
        );
        history.serena_reminded(
            &run,
            &task(),
            CliKind::Claude,
            41,
            7,
            &Later(30),
        );
    });

    let found = reminder_of(&reader, &run, &team());

    assert_eq!(found.expect("the history reads"), Some((40, CliKind::Claude)));
}

#[test]
fn a_round_of_the_team_after_the_run_does_not_take_its_reminder_away() {
    let run = RunId::from_str("run-20261124-100001-r1r1").expect("a run");
    let (_dir, reader) = history(|history| {
        round_opened(history, team(), "1.1", 0);
        run_start(history, run.clone(), team(), task(), 10);
        round_opened(history, team(), "1.2", 20);
        history.serena_reminded(&run, &task(), CliKind::Claude, 40, 7, &Later(30));
    });

    let found = reminder_of(&reader, &run, &team());

    assert_eq!(found.expect("the history reads"), Some((40, CliKind::Claude)));
}

#[test]
fn a_run_of_another_team_after_the_run_leaves_its_reminder_alone() {
    let run = RunId::from_str("run-20261124-100001-r1r1").expect("a run");
    let (_dir, reader) = history(|history| {
        run_start(history, run.clone(), team(), task(), 0);
        history.serena_reminded(&run, &task(), CliKind::Claude, 40, 7, &Later(10));
        run_start(
            history,
            RunId::from_str("run-20261124-100003-r3r3").expect("a run"),
            other_team(),
            other_task(),
            40,
        );
        round_opened(history, other_team(), "1.1", 50);
    });

    let found = reminder_of(&reader, &run, &team());

    assert_eq!(found.expect("the history reads"), Some((40, CliKind::Claude)));
}

#[test]
fn a_history_without_records_reminds_nobody() {
    let run = RunId::from_str("run-20261124-100001-r1r1").expect("a run");
    let (_dir, reader) = history(|_history| {});

    let found = reminder_of(&reader, &run, &team());

    assert_eq!(found.expect("the history reads"), None);
}
