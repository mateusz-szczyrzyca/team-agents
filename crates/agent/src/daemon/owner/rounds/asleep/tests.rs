//! A verified delivery owns its wait even when no member can work.

use super::*;
use crate::daemon::owner::fixture::Daemon;
use crate::fixtures::{at, pr, task, team_id, team_state};

fn waiting_delivery() -> agent_core::state::State {
    let mut state = team_state();
    let mut team = state.team_mut(&team_id(0)).unwrap();
    for (id, suffix) in [("1.1", "wait"), ("1.2", "ownr")] {
        team.open_round(
            format!("round-20261119-090000-{suffix}").parse().unwrap(),
            Some(id.parse().unwrap()),
            at(1),
        )
        .unwrap();
        team.advance_item(
            id.parse().unwrap(),
            ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: "agents/claude-1/item".parse().unwrap(),
            },
        )
        .unwrap();
    }
    team.advance_item(
        "1.2".parse().unwrap(),
        ItemPhase::InReview {
            pr: pr(7),
            reviews: Vec::new(),
        },
    )
    .unwrap();
    team.deliver_round(&"round-20261119-090000-wait".parse().unwrap(), pr(7).number)
        .unwrap();
    drop(team);
    state
}

#[tokio::test]
async fn waiting_delivery_is_not_unfinished_work_for_the_asleep_watch() {
    let daemon = Daemon::new(&waiting_delivery(), None);
    let (owner, _sinks) = daemon.bare_owner().await;
    assert_eq!(
        owner.look_at_sleep(&team_id(0)).unwrap().unfinished,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn waiting_delivery_is_not_blocked_work_when_its_member_failed() {
    let mut state = waiting_delivery();
    {
        let mut team = state.team_mut(&team_id(0)).unwrap();
        let (_, mut member) = team.find_mut(&task(2)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.transition(TaskStatus::Failed).unwrap();
    }
    let daemon = Daemon::new(&state, None);
    let (owner, _sinks) = daemon.bare_owner().await;
    assert_eq!(
        owner.look_at_sleep(&team_id(0)).unwrap().standings,
        Vec::<Standing>::new()
    );
}
