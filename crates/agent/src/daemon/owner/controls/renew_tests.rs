//! `agent renew` through the daemon: what it refuses, and what a refusal leaves alone.

use agent_core::protocol::{Command, ControlRequest, FailureKind, Reply, Target};
use agent_core::state::{Renewals, State};
use agent_core::status::TaskStatus;

use super::super::fixture::{Daemon, reviewed_pair_state};
use crate::client::ClientError;
use crate::fixtures::{solo_state, task, team_id};

/// The request to renew task `n`.
fn renew(n: u32) -> Command {
    Command::Renew(ControlRequest {
        target: Target::Task(task(n)),
    })
}

/// The refusal in `result`: its kind and its message.
fn refusal(result: Result<Reply, ClientError>) -> (FailureKind, String) {
    match result {
        Err(ClientError::Refused(failure)) => (failure.kind, failure.message),
        other => panic!("a refusal was expected: {other:?}"),
    }
}

/// How many runs and which renewal task `n` has in `state`.
fn history_of(state: &State, n: u32) -> (usize, Option<Renewals>) {
    let member = state.find_task(&task(n)).unwrap().member;
    (member.runs().len(), member.renewals().copied())
}

#[tokio::test]
async fn a_task_with_a_run_going_is_not_renewed_and_the_refusal_says_how_to_stop_it() {
    let mut daemon = Daemon::new(&solo_state(), None);
    daemon.scenario(1, "busy");
    daemon.start().await;
    let state = daemon.status_is(1, TaskStatus::Running).await;

    let (kind, message) = refusal(daemon.ask(renew(1)).await);

    assert_eq!(kind, FailureKind::Conflict);
    assert_eq!(
        message,
        format!("{} is running: stop it first with `agent stop {}`", task(1), task(1))
    );
    assert_eq!(history_of(&daemon.state(), 1).1, history_of(&state, 1).1);
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_task_that_is_done_is_not_renewed() {
    let mut daemon = Daemon::new(&solo_state(), None);
    daemon.scenario(1, "done");
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Done).await;

    let (kind, message) = refusal(daemon.ask(renew(1)).await);

    assert_eq!(kind, FailureKind::Conflict);
    assert_eq!(message, format!("{} is done: nothing is left to do for it", task(1)));
    assert_eq!(history_of(&daemon.state(), 1).1, None);
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_task_that_never_ran_is_not_renewed() {
    let mut state = solo_state();
    state
        .find_task_mut(&task(1))
        .unwrap()
        .1
        .transition(TaskStatus::Stopped)
        .unwrap();
    let mut daemon = Daemon::new(&state, None);
    daemon.start().await;

    let (kind, message) = refusal(daemon.ask(renew(1)).await);

    assert_eq!(kind, FailureKind::Conflict);
    assert_eq!(
        message,
        format!("{} has no run yet: there is no session to renew", task(1))
    );
    assert_eq!(history_of(&daemon.state(), 1), (0, None));
    assert_eq!(
        daemon.state().find_task(&task(1)).unwrap().member.status(),
        TaskStatus::Stopped
    );
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_reviewer_is_not_renewed() {
    let mut state = reviewed_pair_state();
    state
        .find_task_mut(&task(3))
        .unwrap()
        .1
        .transition(TaskStatus::Stopped)
        .unwrap();
    let mut daemon = Daemon::new(&state, None);
    daemon.scenario(1, "busy");
    daemon.scenario(2, "busy");
    daemon.start().await;

    let (kind, message) = refusal(daemon.ask(renew(3)).await);

    assert_eq!(kind, FailureKind::Conflict);
    assert_eq!(
        message,
        format!("{} is a reviewer: only a lead or a member can be renewed", task(3))
    );
    assert_eq!(history_of(&daemon.state(), 3), (0, None));
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_team_is_not_renewed_as_a_whole() {
    let mut daemon = Daemon::new(&solo_state(), None);
    daemon.scenario(1, "busy");
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Running).await;

    let (kind, message) = refusal(
        daemon
            .ask(Command::Renew(ControlRequest {
                target: Target::Team(team_id(0)),
            }))
            .await,
    );

    assert_eq!(kind, FailureKind::Invalid);
    assert_eq!(
        message,
        format!("{} is a team: `agent renew` takes one task (`task-…`)", team_id(0))
    );
    daemon.stop().await.unwrap();
}
