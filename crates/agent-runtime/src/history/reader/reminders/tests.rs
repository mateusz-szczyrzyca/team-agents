//! The rows of the reminders are read from the log of a history with the model of their runs.

use std::collections::BTreeMap;
use std::path::Path;

use agent_core::decide::reminders::{ReminderKind, ReminderRow};
use agent_core::history::{AgentIdentity, Cut, MaxTextBytes, Record, RunStart, RunWhy};
use agent_core::ids::{RunId, TaskId, TeamId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::log::{LogEvent, LogRecord, LogTaskScope};
use agent_core::state::Timestamp;
use serde_json::json;

use crate::history::schema::MIGRATIONS;
use crate::history::{HistoryDb, ReadError, Reader, Store};

/// The run of the tests that has a model.
const RUN: &str = "run-20261125-100000-aaaa";
/// The run of the tests whose record has no model.
const BARE_RUN: &str = "run-20261125-100001-bbbb";
/// A run that no record of the history has.
const UNKNOWN_RUN: &str = "run-20261125-100002-cccc";
/// The task of every run of the tests.
const TASK: &str = "task-20261125-100000-tttt";

fn at(text: &str) -> Timestamp {
    text.parse().expect("a time of the tests")
}

/// The start of `run` by agent `claude-1` with `model`.
fn run_start(run: &str, model: Option<&str>) -> Record {
    Record::Run(Box::new(RunStart {
        run: run.parse().expect("a run"),
        team: "team-20261125-100000-mmmm".parse::<TeamId>().expect("a team"),
        task: TASK.parse::<TaskId>().expect("a task"),
        at: at("2026-11-25T10:00:00Z"),
        agent: AgentIdentity {
            name: "claude-1".to_owned(),
            tier: Tier::Normal,
            cli: CliKind::Claude,
            model: model.map(|model| model.parse().expect("a model")),
        },
        role: Role::Solo,
        rounds: Vec::new(),
        why: RunWhy::FirstRun,
        prompt: "prompt".to_owned(),
        worktree: "/work/tree".parse().expect("a path"),
        branch: "agents/claude-1/1".parse().expect("a branch"),
        session: None,
    }))
}

/// A log row of `event` at `when` for `run` with `fields`.
fn log(event: LogEvent, when: &str, run: &str, fields: serde_json::Value) -> LogRecord {
    let serde_json::Value::Object(map) = fields else {
        panic!("the fields of a log row are an object");
    };
    let fields: BTreeMap<String, serde_json::Value> = map.into_iter().collect();
    LogRecord {
        at: at(when),
        module: "agent::daemon".to_owned(),
        event,
        team: None,
        task: Some(LogTaskScope::new(
            TASK.parse().expect("a task"),
            Some(run.parse::<RunId>().expect("a run")),
        )),
        message: "a reminder".to_owned(),
        fields,
        cut: Cut::default(),
    }
}

/// A history of the runs `RUN` (with a model) and `BARE_RUN` (without).
fn open(path: &Path) -> HistoryDb {
    let mut db = HistoryDb::open(path, MaxTextBytes::default()).expect("a history");
    db.write(&[run_start(RUN, Some("sonnet-5-5")), run_start(BARE_RUN, None)])
        .expect("the runs are written");
    db
}

#[test]
fn the_four_events_are_read_with_their_agent_and_the_model_of_their_run() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("history.db");
    let mut db = open(&path);
    db.write_logs(&[
        log(
            LogEvent::CommitReminded,
            "2026-11-25T10:10:00Z",
            RUN,
            json!({"agent": "claude-1", "lines": 400, "minutes": 31}),
        ),
        log(
            LogEvent::ContextReminded,
            "2026-11-25T10:11:00Z",
            RUN,
            json!({"agent": "claude-1", "context": 160_000, "bound": 200_000}),
        ),
        log(
            LogEvent::CommitFollowed,
            "2026-11-25T10:20:00Z",
            RUN,
            json!({"agent": "claude-1", "minutes": 10}),
        ),
        log(
            LogEvent::CommitCut,
            "2026-11-25T10:40:00Z",
            RUN,
            json!({"agent": "claude-1", "lines": 400, "minutes": 20}),
        ),
    ])
    .expect("the log is written");

    let rows = Reader::open(&path)
        .expect("the history opens")
        .reminder_rows()
        .expect("the rows are read");

    let agent = "claude-1".parse().expect("an agent");
    let model = Some("sonnet-5-5".to_owned());
    assert_eq!(
        rows,
        [
            ReminderRow {
                at: at("2026-11-25T10:10:00Z"),
                kind: ReminderKind::Commit,
                agent: "claude-1".parse().expect("an agent"),
                model: model.clone(),
                minutes: None,
            },
            ReminderRow {
                at: at("2026-11-25T10:11:00Z"),
                kind: ReminderKind::Context,
                agent: "claude-1".parse().expect("an agent"),
                model: model.clone(),
                minutes: None,
            },
            ReminderRow {
                at: at("2026-11-25T10:20:00Z"),
                kind: ReminderKind::Followed,
                agent: "claude-1".parse().expect("an agent"),
                model: model.clone(),
                minutes: Some(10),
            },
            ReminderRow {
                at: at("2026-11-25T10:40:00Z"),
                kind: ReminderKind::Cut,
                agent,
                model,
                minutes: None,
            },
        ]
    );
}

#[test]
fn a_run_without_a_model_or_without_a_record_has_no_model() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("history.db");
    let mut db = open(&path);
    db.write_logs(&[
        log(
            LogEvent::CommitReminded,
            "2026-11-25T10:10:00Z",
            BARE_RUN,
            json!({"agent": "claude-1", "lines": 400, "minutes": 31}),
        ),
        log(
            LogEvent::CommitCut,
            "2026-11-25T10:40:00Z",
            UNKNOWN_RUN,
            json!({"agent": "codex-1", "lines": 400, "minutes": 20}),
        ),
    ])
    .expect("the log is written");

    let rows = Reader::open(&path)
        .expect("the history opens")
        .reminder_rows()
        .expect("the rows are read");

    let models: Vec<Option<String>> = rows.into_iter().map(|row| row.model).collect();
    assert_eq!(models, [None, None]);
}

#[test]
fn the_other_events_of_the_log_are_not_rows() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("history.db");
    let mut db = open(&path);
    db.write_logs(&[log(
        LogEvent::RunStarted,
        "2026-11-25T10:00:00Z",
        RUN,
        json!({"agent": "claude-1"}),
    )])
    .expect("the log is written");

    let rows = Reader::open(&path)
        .expect("the history opens")
        .reminder_rows()
        .expect("the rows are read");

    assert_eq!(rows, Vec::new());
}

#[test]
fn a_reminder_row_without_an_agent_is_a_value_error() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("history.db");
    let mut db = open(&path);
    db.write_logs(&[log(
        LogEvent::CommitReminded,
        "2026-11-25T10:10:00Z",
        RUN,
        json!({"lines": 400, "minutes": 31}),
    )])
    .expect("the log is written");

    let error = Reader::open(&path)
        .expect("the history opens")
        .reminder_rows()
        .expect_err("the row has no agent");

    assert!(matches!(
        error,
        ReadError::Value {
            table: "daemon_logs",
            column: "fields",
            ..
        }
    ));
}

#[test]
fn a_followed_row_without_whole_minutes_is_a_value_error() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("history.db");
    let mut db = open(&path);
    db.write_logs(&[log(
        LogEvent::CommitFollowed,
        "2026-11-25T10:20:00Z",
        RUN,
        json!({"agent": "claude-1", "minutes": "ten"}),
    )])
    .expect("the log is written");

    let error = Reader::open(&path)
        .expect("the history opens")
        .reminder_rows()
        .expect_err("the minutes are not a number");

    assert!(matches!(
        error,
        ReadError::Value {
            table: "daemon_logs",
            column: "fields",
            ..
        }
    ));
}

#[test]
fn a_history_before_the_daemon_log_has_no_rows() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("history.db");
    drop(HistoryDb::open_with(&path, &MIGRATIONS[..3], MaxTextBytes::default()).expect("an old history"));

    let rows = Reader::open(&path)
        .expect("the history opens")
        .reminder_rows()
        .expect("the rows are read");

    assert_eq!(rows, Vec::new());
}
