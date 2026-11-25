//! A daemon that is stopping starts no run: the runs its stop ended free their agents, and a queued task must not take
//! one of them in the turns of the owner's loop that are left before it sees the stop.

use agent_core::status::TaskStatus;

use super::fixture::Daemon;
use crate::fixtures::{solo_state, task};

#[tokio::test]
async fn a_queued_task_starts_while_the_daemon_runs() {
    let daemon = Daemon::new(&solo_state(), None);
    daemon.scenario(1, "done");
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.start_queued().await.unwrap();

    assert_eq!(owner.runs.len(), 1, "the queued task is started");
    assert_eq!(owner.parts.state.find_task(&task(1)).unwrap().member.runs().len(), 1);
}

#[tokio::test]
async fn a_queued_task_does_not_start_once_the_daemon_is_stopping() {
    let daemon = Daemon::new(&solo_state(), None);
    daemon.scenario(1, "done");
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.shutdown.cancel();

    owner.start_queued().await.unwrap();

    assert_eq!(owner.runs.len(), 0, "no run is started after the stop");
    let found = owner.parts.state.find_task(&task(1)).unwrap();
    assert!(
        found.member.runs().is_empty(),
        "no run is recorded: {:?}",
        found.member.runs()
    );
    assert_eq!(
        found.member.status(),
        TaskStatus::Queued,
        "the task waits, queued, for the next start"
    );
}
