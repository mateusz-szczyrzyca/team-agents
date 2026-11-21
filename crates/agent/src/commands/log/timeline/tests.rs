//! Rows and steps as one text, in the order things happened, and the newest lines of both.

use std::collections::BTreeMap;
use std::path::Path;

use agent_core::history::{MaxTextBytes, Record, RoundOpening};
use agent_core::ids::{AgentName, RoundId, TaskId, TeamId};
use agent_core::log::LogEntry;
use agent_core::log::{LogEvent, LogRecord};
use agent_runtime::history::LogFilter;
use agent_runtime::history::{HistoryDb, LogWindow, Reader, Store};

use crate::commands::log::steps::{Place, TimedStep};
use crate::commands::log::timeline::Timeline;

/// When `second` is, after 12:50:00 UTC.
fn at(second: u32) -> agent_core::state::Timestamp {
    format!("2026-11-21T12:50:{second:02}Z")
        .parse()
        .expect("a literal time is valid")
}

/// A history with two rounds and three status rows: at 5 s, at 15 s and at 25 s.
fn history(path: &Path) -> (TeamId, RoundId) {
    let mut db = HistoryDb::open(path, MaxTextBytes::default()).expect("a temporary history opens");
    let team: TeamId = "team-20261121-132909-4qem".parse().expect("a literal id is valid");
    let first: RoundId = "round-20261121-132909-q1w2".parse().expect("a literal id is valid");
    let second: RoundId = "round-20261121-132909-q3w4".parse().expect("a literal id is valid");
    db.write(&[
        Record::Round(Box::new(RoundOpening {
            round: first.clone(),
            team: team.clone(),
            at: at(0),
            subject: agent_core::history::RoundSubject::Brief("The first".to_owned()),
        })),
        Record::Round(Box::new(RoundOpening {
            round: second.clone(),
            team: team.clone(),
            at: at(20),
            subject: agent_core::history::RoundSubject::Brief("The second".to_owned()),
        })),
    ])
    .expect("the rounds are written");
    for second in [5, 15, 25] {
        let round = if second < 20 {
            first.clone()
        } else {
            second_round(&second)
        };
        db.write_logs(&[LogRecord {
            at: at(second),
            module: "test".to_owned(),
            event: LogEvent::TaskStatusChanged,
            team: Some(agent_core::log::LogTeamScope::new(team.clone(), [round])),
            task: None,
            message: "task status changed".to_owned(),
            fields: [
                ("from".to_owned(), serde_json::json!("waiting")),
                ("to".to_owned(), serde_json::json!("queued")),
            ]
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
            cut: agent_core::history::Cut::default(),
        }])
        .expect("the rows are written");
    }
    (team, first)
}

/// The round of the third row.
fn second_round(_second: &u32) -> RoundId {
    "round-20261121-132909-q3w4".parse().expect("a literal id is valid")
}

/// The rows of `round`, oldest first.
fn rows(reader: &Reader, round: &RoundId) -> Vec<LogEntry> {
    reader
        .log_rows(
            &LogFilter {
                round: Some(round.clone()),
                ..LogFilter::default()
            },
            LogWindow::Newest(10),
        )
        .expect("the rows are read")
}

/// A step of `task` at `second`, its first.
fn step(task: &str, second: u32) -> TimedStep {
    let task: TaskId = format!("task-20261121-132909-{task}")
        .parse()
        .expect("a literal id is valid");
    TimedStep {
        step: agent_core::history::Step {
            run: "run-20261121-132909-0001".parse().expect("a literal id is valid"),
            number: std::num::NonZeroU32::MIN,
            at: at(second),
            event: agent_core::cli::ActivityEvent::Command("cargo test".to_owned()),
        },
        run: agent_core::history::RunStart {
            run: "run-20261121-132909-0001".parse().expect("a literal id is valid"),
            team: "team-20261121-132909-4qem".parse().expect("a literal id is valid"),
            task,
            at: at(0),
            agent: agent_core::history::AgentIdentity {
                name: "inv-claude-sonnet".to_owned(),
                tier: agent_core::kinds::Tier::Normal,
                cli: agent_core::kinds::CliKind::Claude,
                model: None,
            },
            role: agent_core::kinds::Role::Solo,
            rounds: Vec::new(),
            why: agent_core::history::RunWhy::FirstRun,
            prompt: String::new(),
            worktree: "/home/example/work".parse().expect("a literal path is valid"),
            branch: "agents/example/task".parse().expect("a literal branch is valid"),
            session: None,
        },
        at: at(second),
    }
}

#[test]
fn newest_bounds_the_rows() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let (_team, round) = history(&path);
    let reader = Reader::open(&path).expect("the history opens");
    let found = rows(&reader, &round);
    let timeline = Timeline::new(found.iter().collect(), Vec::new(), Vec::new());
    assert_eq!(timeline.newest(2).lines().count(), 2);
    assert_eq!(timeline.newest(0), String::new());
}

#[test]
fn places_are_against_the_rows() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    let (_team, round) = history(&path);
    let reader = Reader::open(&path).expect("the history opens");
    let found = rows(&reader, &round);
    let steps = vec![step("0001", 12)];
    assert_eq!(
        crate::commands::log::steps::places(&found, &steps),
        vec![Place::After(found.get(0).expect("a row").id())]
    );
    let name: AgentName = "inv-claude-sonnet".parse().expect("a literal name is valid");
    let _ = name;
}
