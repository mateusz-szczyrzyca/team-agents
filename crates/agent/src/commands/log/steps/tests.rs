//! The steps of a scope, their runs and their places among the daemon's rows.

use std::num::NonZeroU32;

use agent_core::cli::ActivityEvent;
use agent_core::history::{
    AgentIdentity, MaxTextBytes, Record, RoundOpening, RoundSubject, RunStart, RunWhy, Step, TeamStart,
};
use agent_core::ids::{RoundId, RunId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::log::{LogEvent, LogRecord};
use agent_core::state::{MergeMode, Timestamp};
use agent_runtime::history::{HistoryDb, LogFilter, Reader, Store};

use super::{covered_runs, run_steps};

/// The team of the history.
fn team() -> TeamId {
    "team-20261121-132909-4qem".parse().expect("a literal id is valid")
}

/// The round of the history.
fn round() -> RoundId {
    "round-20261121-132909-q1w2".parse().expect("a literal id is valid")
}

/// When `second` is, after 12:50:00 UTC.
fn at(second: u32) -> Timestamp {
    format!("2026-11-21T12:50:{second:02}Z")
        .parse()
        .expect("a literal time is valid")
}

/// The task numbered `name`.
fn task(name: &str) -> TaskId {
    format!("task-20261121-132909-{name:0>4}")
        .parse()
        .expect("a literal id is valid")
}

/// The start of the run of `agent`, on `cli`, which works on `task` in `round`.
fn start(agent: &str, cli: CliKind, number: &str, round: Option<&RoundId>) -> RunStart {
    RunStart {
        run: format!("run-20261121-132909-{number:0>4}")
            .parse::<RunId>()
            .expect("a literal id is valid"),
        team: team(),
        task: task(number),
        at: at(0),
        agent: AgentIdentity {
            name: agent.to_owned(),
            tier: Tier::Normal,
            cli,
            model: None,
        },
        role: Role::Solo,
        rounds: round.into_iter().cloned().collect(),
        why: RunWhy::FirstRun,
        prompt: String::new(),
        worktree: "/home/example/worktrees/example/task"
            .parse()
            .expect("a literal path is valid"),
        branch: "agents/example/task".parse().expect("a literal branch is valid"),
        session: None,
    }
}

/// The `number`th step of `run`, at `second`.
fn step(run: &RunStart, number: u32, second: u32) -> Step {
    Step {
        run: run.run.clone(),
        number: NonZeroU32::new(number).expect("a step number starts at one"),
        at: at(second),
        event: ActivityEvent::Text(format!("step {number}")),
    }
}

/// A status row at `second`.
fn row(second: u32) -> LogRecord {
    LogRecord {
        at: at(second),
        module: "test".to_owned(),
        event: LogEvent::TaskStatusChanged,
        team: None,
        task: None,
        message: "task status changed".to_owned(),
        fields: std::collections::BTreeMap::new(),
        cut: agent_core::history::Cut::default(),
    }
}

/// A history of one team and one round, with two runs of two agents and two log rows.
fn history(path: &std::path::Path) {
    let mut db = HistoryDb::open(path, MaxTextBytes::default()).expect("a temporary history opens");
    db.write(&[
        Record::Team(Box::new(TeamStart {
            team: team(),
            at: at(0),
            repo: "example".to_owned(),
            base_branch: "main".parse().expect("a literal branch is valid"),
            merge: MergeMode::NoMerge,
            task_list: String::new(),
            plan: Vec::new(),
        })),
        Record::Round(Box::new(RoundOpening {
            round: round(),
            team: team(),
            at: at(0),
            subject: RoundSubject::Brief("A brief".to_owned()),
        })),
    ])
    .expect("the team and its round are written");
    let first = start("inv-claude-sonnet", CliKind::Claude, "7", Some(&round()));
    let second = start("codex-1", CliKind::Codex, "8", Some(&round()));
    for (run, steps) in [(&first, vec![(1, 1), (2, 2)]), (&second, vec![(1, 12)])] {
        db.write(&[Record::Run(Box::new(run.clone()))])
            .expect("the run is written");
        for (number, second) in steps {
            db.write(&[Record::Step(Box::new(step(run, number, second)))])
                .expect("the step is written");
        }
    }
    db.write_logs(&[row(5), row(15)]).expect("the rows are written");
}

/// The IDs of the runs `filter` covers, oldest first.
fn covered(path: &std::path::Path, filter: &LogFilter) -> Vec<String> {
    let reader = Reader::open(path).expect("the history opens");
    covered_runs(&reader, filter)
        .expect("the runs are read")
        .into_iter()
        .map(|row| row.start.value.agent.name)
        .collect()
}

#[test]
fn a_task_covers_its_runs_and_their_steps_come_oldest_first() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    history(&path);
    let filter = LogFilter {
        task: Some(task("7")),
        ..LogFilter::default()
    };
    assert_eq!(covered(&path, &filter), ["inv-claude-sonnet"]);
    let reader = Reader::open(&path).expect("the history opens");
    let runs = covered_runs(&reader, &filter).expect("the runs are read");
    let steps = run_steps(&reader, &runs[0].start.value).expect("the steps are read");
    assert_eq!(
        steps
            .iter()
            .map(|timed| (timed.step.number.get(), timed.at))
            .collect::<Vec<_>>(),
        vec![(1, at(1)), (2, at(2))]
    );
}

#[test]
fn a_round_and_a_team_cover_the_runs_of_every_agent_in_them() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    history(&path);
    let round = LogFilter {
        round: Some(self::round()),
        ..LogFilter::default()
    };
    let team = LogFilter {
        team: Some(self::team()),
        ..LogFilter::default()
    };
    assert_eq!(covered(&path, &round), ["inv-claude-sonnet", "codex-1"]);
    assert_eq!(covered(&path, &team), ["inv-claude-sonnet", "codex-1"]);
}

#[test]
fn a_scope_that_names_two_things_covers_the_runs_that_match_both() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    history(&path);
    let both = LogFilter {
        task: Some(task("7")),
        agent: Some("codex-1".parse().expect("a literal name is valid")),
        ..LogFilter::default()
    };
    assert_eq!(covered(&path, &both), Vec::<String>::new());
}

#[test]
fn a_scope_with_no_run_task_agent_round_or_team_covers_no_run() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    history(&path);
    assert_eq!(covered(&path, &LogFilter::default()), Vec::<String>::new());
}

#[test]
fn run_steps_are_the_steps_of_that_run() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    history(&path);
    let reader = Reader::open(&path).expect("the history opens");
    let run = reader
        .runs_of_task(&task("8"))
        .expect("the runs are read")
        .remove(0)
        .start
        .value;
    let steps = run_steps(&reader, &run).expect("the steps are read");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].step.number.get(), 1);
    assert_eq!(steps[0].run.agent.name, "codex-1");
}

#[test]
fn an_agent_covers_every_run_of_its_own() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    history(&path);
    let filter = LogFilter {
        agent: Some("codex-1".parse().expect("a literal name is valid")),
        ..LogFilter::default()
    };
    assert_eq!(covered(&path, &filter), ["codex-1"]);
}
