//! Regressions for swap accounting and runs ending during a halt.

use super::super::fixture::Daemon;
use super::super::takeover::Takeover;
use super::*;
use crate::fixtures::{solo_state, task};

#[tokio::test]
async fn harness_takeover_follows_a_human_swap() {
    let daemon = Daemon::new(&solo_state(), None);
    daemon.configure("[replacement]\nagents = [\"claude-1\"]\nreplacements = 1\n");
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let reply = owner
        .swap_request(
            &Client::Cli,
            SwapRequest {
                task: task(1),
                agent: "claude-2".parse().unwrap(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, Reply::Applied { tasks: vec![task(1)] });
    let result = owner.take_over(&task(1), Role::Solo, Trouble::NoStatus).unwrap();
    assert_ne!(result, Takeover::Nobody);
    let member = owner.parts.state.find_task(&task(1)).unwrap().member;
    assert_eq!(member.agent().name(), "claude-1");
    assert_eq!(
        member
            .past_holders()
            .iter()
            .map(|held| held.agent.name())
            .collect::<Vec<_>>(),
        ["claude-1", "claude-2"]
    );
    assert_eq!(member.harness_takeovers(), 1);
}
/// Leaves a naturally completed run queued for settlement while its task still appears running.
async fn swap_after_run_finished(scenario: &str) -> (super::super::Owner, Result<Reply, Failure>) {
    let daemon = Daemon::new(&solo_state(), None);
    let questions = daemon.home.task_dir(&task(1)).join("questions-1.md");
    daemon.scenario(1, &scenario.replace("QUESTIONS_PATH", &questions.to_string_lossy()));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.start_queued().await.unwrap();
    let (old_id, verdict) = tokio::time::timeout(std::time::Duration::from_secs(30), owner.runs.join_next_with_id())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let task_id = owner.running.remove(&old_id).unwrap();
    let handle = owner.runs.spawn(async move { verdict });
    owner.running.insert(handle.id(), task_id);
    let result = owner
        .swap_request(
            &Client::Cli,
            SwapRequest {
                task: task(1),
                agent: "claude-2".parse().unwrap(),
            },
        )
        .await
        .expect("a run finishing during a swap must not stop the owner");
    (owner, result)
}

#[tokio::test]
async fn swap_refuses_a_run_that_finishes_done_during_halt() {
    let (owner, reply) = swap_after_run_finished("done").await;
    assert_eq!(reply, Err(SwapRefusal::Done { task: task(1) }.into()));
    let member = owner.parts.state.find_task(&task(1)).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Done);
    assert_eq!(member.agent().name(), "claude-1");
    assert_eq!(member.past_holders(), []);
}

#[tokio::test]
async fn swap_preserves_questions_from_a_run_finishing_during_halt() {
    let (owner, reply) = swap_after_run_finished("questions QUESTIONS_PATH").await;
    assert_eq!(reply, Ok(Reply::Applied { tasks: vec![task(1)] }));
    let member = owner.parts.state.find_task(&task(1)).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Questions);
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(member.questions().len(), 1);
    assert!(member.current_run().is_none());
}
