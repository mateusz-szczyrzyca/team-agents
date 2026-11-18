//! The fixture's own behaviour: what it does when the home is still locked by something that is going away.

use std::time::Duration;

use agent_runtime::store::StateStore;

use super::Daemon;
use crate::fixtures::solo_state;

#[tokio::test]
async fn a_bare_owner_waits_for_a_home_whose_lock_is_released_a_moment_later() {
    let daemon = Daemon::new(&solo_state(), None);
    let holder = StateStore::open(daemon.home.clone()).await.unwrap();
    // The lock of a daemon that ended is kept for a few milliseconds by a child another test forked: here, a store that
    // is dropped a moment after the owner started to open the home.
    let release = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(holder);
    });

    let (_owner, _sinks) = daemon.bare_owner().await;

    release.await.unwrap();
}

#[tokio::test]
async fn shutdown_does_not_queue_a_fourth_no_status_run() {
    use crate::fixtures::{at, run, task};
    use agent_core::state::RunEnd;
    use agent_core::status::TaskStatus;

    let mut state = solo_state();
    let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
    for index in 0..3 {
        member.transition(TaskStatus::Running).unwrap();
        member
            .begin_run(run(index + 1), at(0), Some("session".parse().unwrap()))
            .unwrap();
        if index < 2 {
            member.end_run(at(0), RunEnd::NoStatus { exit_code: Some(0) }).unwrap();
            member.transition(TaskStatus::Queued).unwrap();
        }
    }
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.interrupted(&task(1), None).await.unwrap();
    let member = owner.parts.state.find_task(&task(1)).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Failed);
    assert_eq!(member.runs().len(), 3);
    owner.recover().await.unwrap();
    assert_eq!(
        owner.parts.state.find_task(&task(1)).unwrap().member.status(),
        TaskStatus::Failed
    );
}
