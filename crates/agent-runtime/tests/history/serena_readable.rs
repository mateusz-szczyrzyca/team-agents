//! A reminder of Serena of a run the history was never told of is not recorded: the record carries the team and the rounds of
//! the run, which the history learns when the run starts, and a record without them would be one no reader can place. It
//! counts as lost.

use std::str::FromStr;
use std::sync::Arc;

use agent_core::config::AbsolutePath;
use agent_core::history::{AgentIdentity, HistoryEvent, MaxTextBytes, Record, RunStart, RunWhy};
use agent_core::ids::{BranchName, Clock, RunId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::state::Timestamp;
use agent_runtime::history::{DEFAULT_QUEUE, History, HistoryDb, Reader, Setup, TracingReporter};
use time::UtcDateTime;

use super::support;

/// The run the history was never told of: no reminder is recorded for it.
const UNSTARTED: &str = "run-20261124-160210-x6a0";

/// A clock that gives the time of the fixture.
struct FixedClock(UtcDateTime);

impl Clock for FixedClock {
    fn now(&self) -> UtcDateTime {
        self.0
    }
}

#[test]
fn a_reminder_of_a_run_the_history_was_never_told_of_is_not_recorded() {
    let at = support::at(600, 0).as_utc();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
    let started = History::start(
        db,
        Setup {
            reporter: Arc::new(TracingReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    let team = TeamId::from_str("team-20261124-100000-t0t0").unwrap();
    let task = TaskId::from_str("task-20261124-100000-t1t1").unwrap();
    let run = RunId::from_str(UNSTARTED).unwrap();
    started
        .history
        .serena_reminded_with_team(&run, &task, CliKind::Claude, 40, 40, Some(&team), &FixedClock(at));
    started.history.flush();
    let lost = started.history.health().lost;
    started.writer.close().unwrap();

    let read = Reader::open(&path).unwrap().team_records(&team).unwrap();
    let reminders = read
        .iter()
        .filter(|stored| {
            matches!(
                stored.record,
                Record::Event(ref event) if matches!(event.event, HistoryEvent::SerenaReminded { .. })
            )
        })
        .count();

    assert_eq!((reminders, lost), (0, 1));
}

#[test]
fn the_run_of_a_reminder_comes_after_its_own_start_in_the_history_it_is_read_from() {
    let at = support::at(600, 0).as_utc();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
    let started = History::start(
        db,
        Setup {
            reporter: Arc::new(TracingReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    let team = TeamId::from_str("team-20261124-100000-t0t0").unwrap();
    let task = TaskId::from_str("task-20261124-100000-t1t1").unwrap();
    let run = RunId::from_str("run-20261124-100001-r1r1").unwrap();
    started.history.record(Record::Run(Box::new(RunStart {
        run: run.clone(),
        team: team.clone(),
        task: task.clone(),
        at: Timestamp::from_utc(at).unwrap(),
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
        worktree: AbsolutePath::from_str("/work").unwrap(),
        branch: BranchName::from_str("main").unwrap(),
        session: None,
    })));
    started
        .history
        .serena_reminded_with_team(&run, &task, CliKind::Claude, 40, 40, Some(&team), &FixedClock(at));
    started.history.flush();
    started.writer.close().unwrap();
    let reader = Reader::open(&path).unwrap();

    let records = reader.team_records(&team).unwrap();
    let start = records
        .iter()
        .position(|stored| matches!(&stored.record, Record::Run(launch) if launch.run == run));

    assert_eq!(start, Some(0));
    assert_eq!(
        records
            .iter()
            .filter(|stored| matches!(
                stored.record,
                Record::Event(ref event) if matches!(event.event, HistoryEvent::SerenaReminded { .. })
            ))
            .count(),
        1,
        "the reminder is a record of the team after the start of its run"
    );
}
