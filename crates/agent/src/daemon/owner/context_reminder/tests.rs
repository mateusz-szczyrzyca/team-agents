//! The reminder of a full context through a daemon in a temporary home: a stand-in for the CLI that reports a context of
//! 130 000 tokens, the real `agentbus`, and the bound of the session.

use std::collections::BTreeSet;
use std::str::FromStr;
use std::time::{Duration, Instant};

use agent_core::cli::SessionId;
use agent_core::ids::RunId;
use agent_core::log::LogEvent;
use agent_core::protocol::{Command, StatusRequest};
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;
use agent_runtime::run::context::ContextRecord;

use super::super::fixture::{Daemon, PATIENCE, agentbus_binary};
use super::{Told, context_to_tell};
use crate::fixtures::{solo_state, task};

/// The text of the reminder for a context of 130 000 tokens at 80% of a bound of 150 000, as the catalog has it.
const REMINDER: &str = "Your session holds 130000 tokens of context, 80% of its bound of 150000. Commit what is finished now: a failing new test as its `RED:` commit, no debug output. Then write in your report to the bus where you are and what is next: the next run may start a new session that sees only your commits, the list of uncommitted files and what you wrote.";

/// A repository in `worktree` with one commit, with the files the stand-in writes for itself left out of what is
/// uncommitted.
fn repository(worktree: &std::path::Path) {
    git(worktree, &["init", "-q", "-b", "main"]);
    std::fs::write(worktree.join(".git/info/exclude"), ".fake-*\n").unwrap();
    git(worktree, &["commit", "-q", "--allow-empty", "-m", "Start"]);
}

/// Runs `git` in `dir` as a fixture author.
fn git(dir: &std::path::Path, args: &[&str]) {
    let done = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
}

/// A daemon with the solo task 1 whose stand-in reports 130 000 tokens of context and holds, under `section`.
fn daemon_for(section: &str) -> (Daemon, std::path::PathBuf) {
    let daemon = Daemon::new(&solo_state(), Some(agentbus_binary()));
    daemon.configure(section);
    let worktree = daemon.scenario(1, "big-context");
    repository(&worktree);
    (daemon, worktree)
}

/// The texts of the chats the harness put on the bus, oldest first.
fn chats(daemon: &Daemon) -> Vec<String> {
    let text = std::fs::read_to_string(daemon.home.root().join("bus/messages.jsonl")).unwrap_or_default();
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|message| message["from"] == "agentbus")
        .map(|message| message["text"].as_str().unwrap().to_owned())
        .collect()
}

/// Waits until `ready` holds, polling; it never sleeps for a fixed time.
async fn until(what: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Waits until the daemon has gone through a few turns of its loop: the look at the teams is at the start of one.
async fn looked(daemon: &Daemon) {
    for _ in 0..3 {
        daemon
            .ask(Command::Status(StatusRequest { target: None }))
            .await
            .unwrap();
    }
}

/// Whether the engine has written the context file of task 1.
fn context_written(daemon: &Daemon) -> bool {
    daemon.home.task_dir(&task(1)).join("context.json").exists()
}

/// The records of the daemon's log for `event`, after the daemon has stopped and its history is complete.
fn logged(daemon: &Daemon, event: LogEvent) -> Vec<agent_core::log::LogEntry> {
    Reader::open(&daemon.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|entry| entry.event() == event)
        .collect()
}

#[tokio::test]
async fn a_session_at_the_percent_of_its_bound_is_reminded_once() {
    let (mut daemon, worktree) = daemon_for("[session]\nmax_context = 150000\n");
    daemon.start().await;

    until("the reminder on the bus", || !chats(&daemon).is_empty()).await;
    for _ in 0..2 {
        looked(&daemon).await;
    }
    std::fs::write(worktree.join(".fake-continue"), "").unwrap();
    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    assert_eq!(chats(&daemon), vec![REMINDER.to_owned()]);
    let reminded = logged(&daemon, LogEvent::ContextReminded);
    assert_eq!(reminded.len(), 1);
    assert_eq!(reminded[0].fields()["context"], 130_000);
    assert_eq!(reminded[0].fields()["bound"], 150_000);
    assert_eq!(reminded[0].fields()["agent"], "claude-1");
    assert_eq!(reminded[0].task(), Some(&task(1)));
    assert!(reminded[0].run().is_some());
}

#[tokio::test]
async fn a_percent_of_zero_turns_the_reminder_off() {
    let (mut daemon, worktree) =
        daemon_for("[session]\nmax_context = 150000\n[commit_reminder]\ncontext_percent = 0\n");
    daemon.start().await;
    until("the engine to write the context", || context_written(&daemon)).await;

    for _ in 0..3 {
        looked(&daemon).await;
    }
    std::fs::write(worktree.join(".fake-continue"), "").unwrap();
    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    assert_eq!(chats(&daemon), Vec::<String>::new());
    assert_eq!(logged(&daemon, LogEvent::ContextReminded).len(), 0);
}

#[tokio::test]
async fn a_session_below_the_percent_of_its_bound_is_not_reminded() {
    let (mut daemon, worktree) = daemon_for("[session]\nmax_context = 200000\n");
    daemon.start().await;
    until("the engine to write the context", || context_written(&daemon)).await;

    for _ in 0..3 {
        looked(&daemon).await;
    }
    std::fs::write(worktree.join(".fake-continue"), "").unwrap();
    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    assert_eq!(chats(&daemon), Vec::<String>::new());
}

/// Runs a daemon whose session is reminded of its full context, and records the event the daemon logged for it, for the
/// test that every log event has a production cause.
pub(in crate::daemon::owner) async fn record_context_reminded(observed: &mut BTreeSet<LogEvent>) {
    let (mut daemon, worktree) = daemon_for("[session]\nmax_context = 150000\n");
    daemon.start().await;
    until("the reminder on the bus", || !chats(&daemon).is_empty()).await;
    std::fs::write(worktree.join(".fake-continue"), "").unwrap();
    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    assert!(
        !logged(&daemon, LogEvent::ContextReminded).is_empty(),
        "context_reminded was not logged"
    );
    observed.insert(LogEvent::ContextReminded);
}

/// A record of the run `run` in `session` with `context` tokens.
fn record(run: &str, session: Option<&str>, context: u64) -> ContextRecord {
    ContextRecord {
        run: RunId::from_str(run).unwrap(),
        session: session.map(|id| SessionId::from_str(id).unwrap()),
        context,
    }
}

const RUN: &str = "run-20261125-100000-a1b2";
const OTHER_RUN: &str = "run-20261125-100001-c3d4";
const SESSION: &str = "0c4e9a52-7a11-4d0b-8f3e-5b2c6d1e9a07";
const OTHER_SESSION: &str = "9d8c7b6a-5f4e-4d3c-8b2a-1f0e9d8c7b6a";

#[test]
fn the_context_to_tell_follows_the_bound_the_percent_and_what_was_told() {
    let run = RunId::from_str(RUN).unwrap();
    let session = Told::Session(SessionId::from_str(SESSION).unwrap());
    let other_session = Told::Session(SessionId::from_str(OTHER_SESSION).unwrap());
    let by_run = Told::Run(RunId::from_str(RUN).unwrap());
    // (record, bound, percent, what the session was told by, the context to tell).
    let table = [
        (
            record(RUN, Some(SESSION), 120_000),
            Some(150_000),
            80,
            None,
            Some(120_000),
        ),
        (record(RUN, Some(SESSION), 119_999), Some(150_000), 80, None, None),
        (record(RUN, Some(SESSION), 130_000), None, 80, None, None),
        (record(RUN, Some(SESSION), 130_000), Some(150_000), 0, None, None),
        (record(OTHER_RUN, Some(SESSION), 130_000), Some(150_000), 80, None, None),
        (
            record(RUN, Some(SESSION), 130_000),
            Some(150_000),
            80,
            Some(&session),
            None,
        ),
        (
            record(RUN, Some(SESSION), 130_000),
            Some(150_000),
            80,
            Some(&other_session),
            Some(130_000),
        ),
        (record(RUN, None, 130_000), Some(150_000), 80, None, Some(130_000)),
        (record(RUN, None, 130_000), Some(150_000), 80, Some(&by_run), None),
        (
            record(RUN, None, 130_000),
            Some(150_000),
            80,
            Some(&session),
            Some(130_000),
        ),
    ];
    for (index, (record, bound, percent, told, expected)) in table.into_iter().enumerate() {
        assert_eq!(
            context_to_tell(&record, &run, bound, percent, told),
            expected,
            "row {index}: {record:?}, bound {bound:?}, percent {percent}, told {told:?}"
        );
    }
}
