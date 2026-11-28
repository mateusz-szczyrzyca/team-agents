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
