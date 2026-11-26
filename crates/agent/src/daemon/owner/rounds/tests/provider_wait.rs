//! A round whose reviewer its provider refused says which provider and until when, never that the human owes an answer
//! while no question is open (issue #233), as `agent status` shows it.
use agent_core::protocol::{Client, Command, Reply, ShowRequest, Target};
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;

use super::rig::{ONE, PATIENCE, Rig, investigator, normal};

/// What the daemon tells of `round`: the line `agent status` shows for what it waits for.
async fn waiting_of(rig: &Rig, round: agent_core::ids::RoundId) -> Option<String> {
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Show(ShowRequest {
                target: Target::Round(round),
            }),
        )
        .await
        .unwrap();
    let Reply::Round { round } = reply else {
        panic!("expected the detail of a round: {reply:?}");
    };
    round.waiting
}

/// What the daemon tells of `round` once it tells anything: a round whose reviewer is paused by its provider is idle,
/// and so has a line of what it waits for, only when no other member of it still runs.
async fn waiting_once_set(rig: &Rig, round: agent_core::ids::RoundId) -> String {
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        if let Some(waiting) = waiting_of(rig, round.clone()).await {
            return waiting;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the round {round} to say what it waits for"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

// Rule (#233): a review whose reviewer was refused by its provider and whose other reviewer has answered waits at the
// provider of the refused one, with the time of its return; the human is not asked for anything.
#[tokio::test]
async fn a_round_with_a_reviewer_refused_by_its_provider_waits_at_the_provider() {
    let mut rig = Rig::new(
        &[
            normal("claude-2", "work"),
            investigator("inv-r1", "turn-limit-quota"),
            investigator("inv-r2", "sleep"),
        ],
        5,
        false,
    )
    .await;
    rig.pin_turn_limit_clock();
    rig.start().await;
    let (team, _tasks) = rig.run(&["claude-2"], ONE, MergeMode::Autopilot, None).await;
    let item = "1.1".parse().unwrap();

    let state = rig
        .state_where(
            "a reviewer to be paused by its provider while the item is in review",
            |state| {
                state.team(&team).is_some_and(|found| {
                    found.round_of(item).is_some_and(|round| {
                        found.members_in(round.id()).iter().any(|(_, member)| {
                            member.agent().name() == "inv-r1" && member.status() == TaskStatus::Paused
                        })
                    })
                })
            },
        )
        .await;
    let round = state.team(&team).unwrap().round_of(item).unwrap().id().clone();

    let waiting = waiting_once_set(&rig, round).await;

    assert!(waiting.starts_with("the provider of inv-r1 until "), "{waiting}");
    assert!(!waiting.contains("the human"), "{waiting}");
    rig.stop().await.unwrap();
}
