//! A history on disk for the tests of `agent log --reminders`: one run of `claude-1` with the model `sonnet-5-5`, and
//! the log events of its reminders at times counted back from a moment the test names.

use std::collections::BTreeMap;
use std::path::Path;

use agent_core::history::{AgentIdentity, Cut, MaxTextBytes, Record, RunStart, RunWhy};
use agent_core::ids::{RunId, TaskId};
use agent_core::kinds::{CliKind, Role, Tier};
use agent_core::log::{LogEvent, LogRecord, LogTaskScope};
use agent_core::state::Timestamp;
use agent_runtime::history::schema::MIGRATIONS;
use agent_runtime::history::{HistoryDb, Store};
use serde_json::json;

/// The run of the history.
const RUN: &str = "run-20261125-100000-aaaa";
/// The task of the run.
const TASK: &str = "task-20261125-100000-tttt";

/// `hours` before `now`.
fn before(now: Timestamp, hours: i64) -> Timestamp {
    Timestamp::from_utc(now.as_utc() - time::Duration::hours(hours)).unwrap()
}

/// A log row of `event` at `at`, with the fields of `fields` (an object) and the agent `claude-1`.
fn log(event: LogEvent, at: Timestamp, fields: serde_json::Value) -> LogRecord {
    let serde_json::Value::Object(map) = fields else {
        panic!("the fields of a log row are an object");
    };
    let mut fields: BTreeMap<String, serde_json::Value> = map.into_iter().collect();
    fields.insert("agent".to_owned(), json!("claude-1"));
    LogRecord {
        at,
        module: "agent::daemon".to_owned(),
        event,
        team: None,
        task: Some(LogTaskScope::new(
            TASK.parse::<TaskId>().unwrap(),
            Some(RUN.parse::<RunId>().unwrap()),
        )),
        message: "a reminder".to_owned(),
        fields,
        cut: Cut::default(),
    }
}

/// The run and the reminders of `claude-1`: three commit reminders, a context reminder, two followed after 9 and 12
/// minutes and a cut, all in the three days before `now`.
fn records(now: Timestamp) -> (Vec<Record>, Vec<LogRecord>) {
    let run = Record::Run(Box::new(RunStart {
        run: RUN.parse().unwrap(),
        team: "team-20261125-100000-mmmm".parse().unwrap(),
        task: TASK.parse().unwrap(),
        at: before(now, 72),
        agent: AgentIdentity {
            name: "claude-1".to_owned(),
            tier: Tier::Normal,
            cli: CliKind::Claude,
            model: Some("sonnet-5-5".parse().unwrap()),
        },
        role: Role::Solo,
        rounds: Vec::new(),
        why: RunWhy::FirstRun,
        prompt: "prompt".to_owned(),
        worktree: "/work/tree".parse().unwrap(),
        branch: "agents/claude-1/1".parse().unwrap(),
        session: None,
    }));
    let logs = vec![
        log(
            LogEvent::CommitReminded,
            before(now, 60),
            json!({"lines": 400, "minutes": 31}),
        ),
        log(LogEvent::CommitFollowed, before(now, 59), json!({"minutes": 9})),
        log(
            LogEvent::CommitReminded,
            before(now, 40),
            json!({"lines": 350, "minutes": 35}),
        ),
        log(LogEvent::CommitFollowed, before(now, 39), json!({"minutes": 12})),
        log(
            LogEvent::CommitReminded,
            before(now, 20),
            json!({"lines": 500, "minutes": 40}),
        ),
        log(
            LogEvent::ContextReminded,
            before(now, 10),
            json!({"context": 160_000, "bound": 200_000}),
        ),
        log(
            LogEvent::CommitCut,
            before(now, 1),
            json!({"lines": 500, "minutes": 20}),
        ),
    ];
    (vec![run], logs)
}

/// Writes the history of the reminders of the three days before `now` to `path`.
pub(crate) fn write_history(path: &Path, now: Timestamp) {
    let (records, logs) = records(now);
    let mut db = HistoryDb::open(path, MaxTextBytes::default()).unwrap();
    db.write(&records).unwrap();
    db.write_logs(&logs).unwrap();
}

/// Writes a history of the version before the daemon log (version 3: no log rows, so none of the reminders).
pub(crate) fn write_history_without_log(path: &Path) {
    drop(HistoryDb::open_with(path, &MIGRATIONS[..3], MaxTextBytes::default()).unwrap());
}
