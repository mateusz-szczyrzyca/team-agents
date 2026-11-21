//! The line of a record: the event in words, its task, its agent, its run and its fields.

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use super::record_line;
use crate::config::AbsolutePath;
use crate::history::{AgentIdentity, RunStart, RunWhy};
use crate::ids::{RoundId, RunId, TaskId, TeamId};
use crate::kinds::{CliKind, Role, Tier};
use crate::log::{LogEntry, LogEntryParts, LogEvent};

/// The task every row of the table is about.
fn task() -> TaskId {
    "task-20261121-132909-ummt".parse().expect("a literal id is valid")
}

/// The team of the table.
fn team() -> TeamId {
    "team-20261121-132909-4qem".parse().expect("a literal id is valid")
}

/// The round of the table.
fn round() -> RoundId {
    "round-20261121-132909-q1w2".parse().expect("a literal id is valid")
}

/// The run of the table.
fn run() -> RunId {
    "run-20261121-132909-2fqp".parse().expect("a literal id is valid")
}

/// The start of the table's run, run by `agent`.
fn start(agent: &str) -> RunStart {
    RunStart {
        run: run(),
        team: team(),
        task: task(),
        at: "2026-11-21T12:50:32Z".parse().expect("a literal time is valid"),
        agent: AgentIdentity {
            name: agent.to_owned(),
            tier: Tier::Normal,
            cli: CliKind::Claude,
            model: None,
        },
        role: Role::Solo,
        rounds: vec![round()],
        why: RunWhy::FirstRun,
        prompt: String::new(),
        worktree: "/home/example/worktrees/example/task"
            .parse::<AbsolutePath>()
            .expect("a literal path is valid"),
        branch: "agents/example/task".parse().expect("a literal branch is valid"),
        session: None,
    }
}

/// A row of the table, in the scope of the table's task, team and run.
fn entry(event: LogEvent, message: &str, fields: &[(&str, &str)]) -> LogEntry {
    let at = "2026-11-21T12:50:32Z".parse().expect("a literal time is valid");
    LogEntry::try_new(LogEntryParts {
        id: 1,
        first_at: at,
        at,
        level: event.level(),
        module: "agent::daemon::owner".to_owned(),
        event,
        team: Some(team()),
        rounds: vec![round()],
        task: Some(task()),
        run: Some(run()),
        message: message.to_owned(),
        fields: fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), serde_json::Value::String((*value).to_owned())))
            .collect::<BTreeMap<_, _>>(),
        count: NonZeroU64::MIN,
        cut: String::new(),
    })
    .expect("the table's rows are valid")
}

/// The subject of every row: the task, the team, the round and the run.
fn subject(agent: &str) -> String {
    format!("{} ({agent}) team {} rounds {} run {}", task(), team(), round(), run())
}

#[test]
fn record_line_says_what_happened_with_its_subject_and_fields() {
    let long = "x".repeat(200);
    let cut = format!("{}…", "x".repeat(120));
    let rows: Vec<(LogEntry, String)> = vec![
        (
            entry(
                LogEvent::TaskStatusChanged,
                "task status changed",
                &[("from", "queued"), ("to", "running")],
            ),
            format!(
                "2026-11-21T12:50:32Z info task_status_changed {}: queued → running",
                subject("inv-claude-sonnet")
            ),
        ),
        (
            entry(
                LogEvent::RunStarted,
                "run started: the bus woke the agent for a message",
                &[("agent", "inv-claude-sonnet"), ("why", "wake_up"), ("from", "codex-1")],
            ),
            format!(
                "2026-11-21T12:50:32Z info run_started {}: the bus woke the agent for a message from codex-1",
                subject("inv-claude-sonnet")
            ),
        ),
        (
            entry(LogEvent::TeamChanged, "a team is started", &[("kind", "started")]),
            format!(
                "2026-11-21T12:50:32Z info team_changed {}: kind=started",
                subject("inv-claude-sonnet")
            ),
        ),
        (
            entry(LogEvent::BusDown, "agentbus is down", &[("detail", &long)]),
            format!(
                "2026-11-21T12:50:32Z warn bus_down {}: detail={cut}",
                subject("inv-claude-sonnet")
            ),
        ),
    ];
    for (row, expected) in rows {
        assert_eq!(record_line(&row, Some(&start("inv-claude-sonnet"))), expected);
    }
}
