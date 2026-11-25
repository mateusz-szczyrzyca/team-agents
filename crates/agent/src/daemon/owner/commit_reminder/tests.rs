//! The reminder to commit through a daemon in a temporary home: a stand-in for the CLI that leaves 400 uncommitted
//! lines, the real `agentbus`, and a clock the test moves.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command as Process;
use std::time::{Duration, Instant};

use agent_core::log::LogEvent;
use agent_core::protocol::{Command, StatusRequest};
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;

use super::super::fixture::{Daemon, PATIENCE, agentbus_binary};
use crate::fixtures::{solo_state, task};

/// The text of the reminder for 400 lines and 2 minutes, as the catalog has it.
const REMINDER: &str = "You have 400 changed lines uncommitted and no commit for 2 minutes. Commit what is finished now, in small commits. A new test that fails goes in as its `RED:` commit with its failing output. Never commit debug output (`eprintln!`, `dbg!`, probes). Do not commit only to silence this message. Then go on with your work.";

/// A repository in `worktree` whose only commit is older than any time of the tests, with the files the stand-in writes
/// for itself left out of what is uncommitted.
fn repository(worktree: &Path) {
    git(worktree, &["init", "-q", "-b", "main"]);
    std::fs::write(worktree.join(".git/info/exclude"), ".fake-*\n").unwrap();
    git(worktree, &["commit", "-q", "--allow-empty", "-m", "Start"]);
}

/// Runs `git` in `dir` as a fixture author, with every commit dated long before the clock of the tests.
fn git(dir: &Path, args: &[&str]) -> String {
    let done = Process::new("git")
        .current_dir(dir)
        .env("GIT_AUTHOR_DATE", "2026-11-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-11-01T00:00:00Z")
        .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
    String::from_utf8(done.stdout).unwrap().trim().to_owned()
}

/// A daemon with the solo task 1 whose stand-in does `scenario`, in a repository, under `[commit_reminder]` of `section`.
fn daemon_for(scenario: &str, section: &str) -> (Daemon, std::path::PathBuf) {
    let daemon = Daemon::new(&solo_state(), Some(agentbus_binary()));
    daemon.configure(section);
    let worktree = daemon.scenario(1, scenario);
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

/// Waits until the daemon has gone through a few turns of its loop since the clock last moved: the answer to a request
/// comes between two turns, and the look at the teams is at the start of one.
async fn looked(daemon: &Daemon) {
    for _ in 0..3 {
        daemon
            .ask(Command::Status(StatusRequest { target: None }))
            .await
            .unwrap();
    }
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
async fn a_run_with_uncommitted_work_past_both_bounds_is_reminded_once() {
    let (mut daemon, worktree) = daemon_for("dirty-hold", "[commit_reminder]\nlines = 300\nminutes = 1\n");
    daemon.start().await;
    until("the stand-in to leave its work", || worktree.join("dirty.txt").exists()).await;

    daemon.clock.advance(time::Duration::minutes(2));
    until("the reminder on the bus", || !chats(&daemon).is_empty()).await;
    for _ in 0..2 {
        daemon.clock.advance(time::Duration::minutes(2));
        looked(&daemon).await;
    }
    std::fs::write(worktree.join(".fake-continue"), "").unwrap();
    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    assert_eq!(chats(&daemon), vec![REMINDER.to_owned()]);
    let reminded = logged(&daemon, LogEvent::CommitReminded);
    assert_eq!(reminded.len(), 1);
    assert_eq!(reminded[0].fields()["lines"], 400);
    assert_eq!(reminded[0].fields()["minutes"], 2);
    assert_eq!(reminded[0].task(), Some(&task(1)));
    assert!(reminded[0].run().is_some());
    assert_eq!(reminded[0].fields()["agent"], "claude-1");
}

#[tokio::test]
async fn a_bound_at_zero_turns_the_reminder_off() {
    let (mut daemon, worktree) = daemon_for("dirty-hold", "[commit_reminder]\nlines = 0\nminutes = 1\n");
    daemon.start().await;
    until("the stand-in to leave its work", || worktree.join("dirty.txt").exists()).await;

    daemon.clock.advance(time::Duration::minutes(5));
    looked(&daemon).await;
    daemon.clock.advance(time::Duration::minutes(5));
    looked(&daemon).await;
    std::fs::write(worktree.join(".fake-continue"), "").unwrap();
    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    assert_eq!(chats(&daemon), Vec::<String>::new());
    assert_eq!(logged(&daemon, LogEvent::CommitReminded).len(), 0);
}

#[tokio::test]
async fn a_run_that_committed_its_work_is_not_reminded() {
    let (mut daemon, worktree) = daemon_for("dirty-commit-hold", "[commit_reminder]\nlines = 300\nminutes = 1\n");
    daemon.start().await;
    until("the stand-in to commit its work", || {
        git(&worktree, &["rev-list", "--count", "HEAD"]) == "2"
    })
    .await;

    daemon.clock.advance(time::Duration::minutes(5));
    looked(&daemon).await;
    daemon.clock.advance(time::Duration::minutes(5));
    looked(&daemon).await;
    std::fs::write(worktree.join(".fake-continue"), "").unwrap();
    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    assert_eq!(chats(&daemon), Vec::<String>::new());
}

#[tokio::test]
async fn a_commit_re_arms_the_reminder_and_the_follow_is_logged() {
    let (mut daemon, worktree) = daemon_for("dirty-twice", "[commit_reminder]\nlines = 300\nminutes = 1\n");
    daemon.start().await;
    until("the stand-in to leave its work", || worktree.join("dirty.txt").exists()).await;
    daemon.clock.advance(time::Duration::minutes(2));
    until("the first reminder", || chats(&daemon).len() == 1).await;

    std::fs::write(worktree.join(".fake-commit"), "").unwrap();
    until("the stand-in to commit", || {
        git(&worktree, &["rev-list", "--count", "HEAD"]) == "2" && worktree.join("second.txt").exists()
    })
    .await;
    daemon.clock.advance(time::Duration::minutes(2));
    until("the second reminder", || chats(&daemon).len() == 2).await;
    std::fs::write(worktree.join(".fake-continue"), "").unwrap();
    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    assert_eq!(logged(&daemon, LogEvent::CommitReminded).len(), 2);
    let followed = logged(&daemon, LogEvent::CommitFollowed);
    assert_eq!(followed.len(), 1);
    assert_eq!(followed[0].task(), Some(&task(1)));
    assert!(followed[0].run().is_some());
    assert_eq!(followed[0].fields()["agent"], "claude-1");
}

/// Runs a daemon whose run is reminded to commit and then commits, and records the events the daemon logged for it, for
/// the test that every log event has a production cause.
pub(in crate::daemon::owner) async fn record_commit_events(observed: &mut BTreeSet<LogEvent>) {
    let (mut daemon, worktree) = daemon_for("dirty-twice", "[commit_reminder]\nlines = 300\nminutes = 1\n");
    daemon.start().await;
    until("the stand-in to leave its work", || worktree.join("dirty.txt").exists()).await;
    daemon.clock.advance(time::Duration::minutes(2));
    until("the reminder", || chats(&daemon).len() == 1).await;
    std::fs::write(worktree.join(".fake-commit"), "").unwrap();
    until("the stand-in to commit", || worktree.join("second.txt").exists()).await;
    daemon.clock.advance(time::Duration::minutes(2));
    until("the second reminder", || chats(&daemon).len() == 2).await;
    std::fs::write(worktree.join(".fake-continue"), "").unwrap();
    daemon.status_is(1, TaskStatus::Waiting).await;
    daemon.stop().await.unwrap();

    for event in [LogEvent::CommitReminded, LogEvent::CommitFollowed] {
        assert!(!logged(&daemon, event).is_empty(), "{event:?} was not logged");
        observed.insert(event);
    }
}
