//! The line of a record: the event in words, its task, its agent, its run and its fields.

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use crate::config::AbsolutePath;
use crate::history::{AgentIdentity, RunStart, RunWhy};
use crate::ids::{RoundId, RunId, TaskId, TeamId};
use crate::kinds::{CliKind, Role, Tier};
use crate::log::render::record_line;
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

/// The subject of every row: the task, the agent, the team, the round and the run.
fn subject(agent: &str) -> String {
    format!("{} ({agent}) team {} rounds {} run {}", task(), team(), round(), run())
}

#[test]
fn record_line_gives_the_common_events_words_and_every_other_its_fields() {
    let long = "x".repeat(200);
    let cut = format!("{}…", "x".repeat(120));
    let rows: Vec<(LogEntry, String)> = vec![
        (
            entry(
                LogEvent::TaskStatusChanged,
                "task status changed",
                &[("from", "waiting"), ("to", "queued")],
            ),
            format!("{}: queued ← waiting", subject("inv-claude-sonnet")),
        ),
        (
            entry(LogEvent::RunStarted, "run started", &[("agent", "inv-claude-sonnet")]),
            format!("{}: run started by inv-claude-sonnet", subject("inv-claude-sonnet")),
        ),
        (
            entry(
                LogEvent::RunStarted,
                "run started: the bus woke the agent for a message",
                &[
                    ("agent", "inv-claude-sonnet"),
                    ("why", "wake_up"),
                    ("message_id", "1510"),
                    ("from", "codex-1"),
                    ("wake_ups", "2"),
                ],
            ),
            format!(
                "{}: the bus woke the agent for a message from codex-1 message_id=1510 wake_ups=2",
                subject("inv-claude-sonnet")
            ),
        ),
        (
            entry(
                LogEvent::RunEnded,
                "run ended",
                &[("end", "Reported { outcome: Done }")],
            ),
            format!("{}: ended reported DONE", subject("inv-claude-sonnet")),
        ),
        (
            entry(
                LogEvent::RunEnded,
                "run ended",
                &[("end", "NoStatus { exit_code: Some(1) }")],
            ),
            format!("{}: ended no STATUS line (exit code 1)", subject("inv-claude-sonnet")),
        ),
        (
            entry(
                LogEvent::RunEnded,
                "run ended",
                &[("end", "QuotaExhausted { resets_at: None }")],
            ),
            format!("{}: ended quota exhausted", subject("inv-claude-sonnet")),
        ),
        (
            entry(
                LogEvent::ItemPhaseChanged,
                "an item changed phase after a review verdict",
                &[
                    ("item", "1.1"),
                    ("phase", "in review"),
                    ("pr", "168"),
                    ("from", "in progress"),
                ],
            ),
            format!(
                "{}: item 1.1 in review ← in progress after a review verdict, pr=168",
                subject("inv-claude-sonnet")
            ),
        ),
        (
            entry(
                LogEvent::NotificationSent,
                "a notification is delivered",
                &[("file", "outbox/1.md")],
            ),
            format!(
                "{}: a notification is delivered file=outbox/1.md",
                subject("inv-claude-sonnet")
            ),
        ),
        (
            entry(LogEvent::BusDown, "agentbus is down", &[("detail", &long)]),
            format!("{}: agentbus is down detail={cut}", subject("inv-claude-sonnet")),
        ),
    ];
    for (row, rest) in rows {
        let expected = format!("{} {} {} {rest}", row.at(), row.level(), row.event());
        assert_eq!(
            record_line(&row, Some(&start("inv-claude-sonnet"))),
            expected,
            "{:?}",
            row.event()
        );
    }
}

#[test]
fn record_line_tells_every_end_of_a_run_and_the_events_of_questions_and_of_a_refused_publication() {
    let ended = |end: &str| entry(LogEvent::RunEnded, "run ended", &[("end", end)]);
    let rows: Vec<(LogEntry, String)> = vec![
        (
            ended("RateLimited { retry_after: None }"),
            format!("{}: ended rate limited", subject("inv-claude-sonnet")),
        ),
        (
            ended("Hung"),
            format!("{}: ended hung: stopped by the watchdog", subject("inv-claude-sonnet")),
        ),
        (
            ended("TurnLimit"),
            format!("{}: ended turn limit reached", subject("inv-claude-sonnet")),
        ),
        (
            ended("Stopped"),
            format!("{}: ended stopped by the human", subject("inv-claude-sonnet")),
        ),
        (
            ended("NoStatus { exit_code: None }"),
            format!(
                "{}: ended no STATUS line (ended by a signal)",
                subject("inv-claude-sonnet")
            ),
        ),
        (
            ended("QuotaExhausted { resets_at: Some(2026-11-21T17:00:00Z) }"),
            format!(
                "{}: ended quota exhausted (back at 2026-11-21T17:00:00Z)",
                subject("inv-claude-sonnet")
            ),
        ),
        (
            entry(
                LogEvent::QuestionsFileOpened,
                "a questions file is opened",
                &[("number", "2")],
            ),
            format!("{}: a questions file is opened number=2", subject("inv-claude-sonnet")),
        ),
        (
            entry(
                LogEvent::QuestionsAnswered,
                "the questions are answered",
                &[("via", "cli")],
            ),
            format!("{}: the questions are answered via=cli", subject("inv-claude-sonnet")),
        ),
        (
            entry(
                LogEvent::GithubStepWaiting,
                "a step that calls GitHub waits",
                &[("operation", "merge"), ("outlook", "will_not_pass"), ("pr", "167")],
            ),
            format!(
                "{}: a step that calls GitHub waits operation=merge outlook=will_not_pass pr=167",
                subject("inv-claude-sonnet")
            ),
        ),
    ];
    for (row, expected) in rows {
        let line = record_line(&row, Some(&start("inv-claude-sonnet")));
        assert_eq!(line, format!("2026-11-21T12:50:32Z {} {expected}", row_prefix(&row)));
    }
}

/// The level and the event of `row`, as its line starts after the time.
fn row_prefix(row: &LogEntry) -> String {
    format!("{} {}", row.level(), row.event())
}
