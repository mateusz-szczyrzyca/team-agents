//! A GitHub that answers no call, for a while or for good, never ends the owner: each step that fails waits with a
//! growing pause, the round says what it waits for, and the other rounds go on. The evidence of 20.11.2026 is the
//! second opinion of a pull request whose read of GitHub timed out and stopped the whole daemon for the night.
use std::time::Duration;

use agent_core::log::LogEvent;
use agent_core::protocol::{Client, Command, Reply, StatusRequest};
use agent_core::state::{ItemPhase, MergeMode, State};
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, investigator, normal};

/// The daemon's log as the history kept it, or none before the history wrote it.
fn daemon_rows(home: &agent_runtime::store::Home) -> Vec<agent_core::log::LogEntry> {
    Reader::open(&home.history_file())
        .and_then(|reader| reader.daemon_logs())
        .unwrap_or_default()
}

/// Whether the daemon recorded that a task failed: how it ends a round for an outside failure.
fn a_task_failed(home: &agent_runtime::store::Home) -> bool {
    daemon_rows(home)
        .iter()
        .any(|row| row.event() == LogEvent::DaemonTaskFailed)
}

/// Whether the daemon recorded a step that waits for GitHub.
fn a_step_waits(home: &agent_runtime::store::Home) -> bool {
    daemon_rows(home)
        .iter()
        .any(|row| row.event() == LogEvent::GithubStepWaiting)
}

/// Asks the daemon status, as the human does.
async fn status(rig: &Rig) -> Reply {
    crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap()
}

/// Waits until `ready` holds for the daemon's log of `home`.
async fn wait_for_log(home: &agent_runtime::store::Home, what: &str, ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + super::rig::PATIENCE;
    while !ready() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_second_opinion_whose_read_of_github_fails_does_not_stop_the_daemon() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,opinion-ask"),
            investigator("inv-r1", "opinion-probe:CONFIRMED"),
            investigator("inv-r2", "opinion-probe:CONFIRMED"),
            investigator("inv-r3", "opinion-probe:NOT CONFIRMED"),
        ],
        5,
        false,
    )
    .await;
    // GitHub answers no call while the fix pass and its second opinions are under way.
    rig.github.fail_every_call(503);
    rig.start().await;
    let (team, _tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.state_where("the fix pass", |state: &State| {
        state
            .team(&team)
            .and_then(|found| found.items().first())
            .is_some_and(|item| matches!(item.phase(), ItemPhase::Fixing { .. }))
    })
    .await;
    let home = rig.home.clone();
    wait_for_log(&home, "a step that waits for GitHub", || a_step_waits(&home)).await;

    assert!(!a_task_failed(&home), "the network must not end the owner");
    assert!(
        matches!(status(&rig).await, Reply::Overview { .. } | Reply::Teams { .. }),
        "the daemon answers status while GitHub is down"
    );
    rig.stop().await.unwrap();
}
