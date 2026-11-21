//! Human answers must preserve the capability requirement of the next fix pass.

use super::*;
use crate::daemon::owner::fixture::Daemon;
use crate::fixtures::{at, investigator, member, normal, pr, repo, review, task, team_id};
use agent_core::state::{Crew, State, TaskListText, Team, TeamMembers};
use agent_core::status::ReviewVerdict;

#[tokio::test]
async fn a_human_answer_gives_the_renewed_fix_to_a_qualified_member() {
    let id = "1.1".parse().unwrap();
    let mut team = Team::new(
        team_id(1),
        repo(),
        at(0),
        agent_core::state::MergeMode::Autopilot,
        TaskListText::new("Brief.\n\n1. Feature\n[ ] 1.1 Item\nTier: investigator\n".into()).unwrap(),
        Crew::team(TeamMembers {
            coordinator: member(1, normal("lead")),
            members: vec![member(2, investigator("inv-member"))],
            workers: vec![],
        })
        .unwrap(),
    );
    team.open_round("round-20261111-153001-test".parse().unwrap(), Some(id), at(1))
        .unwrap();
    team.advance_item(
        id,
        ItemPhase::InProgress {
            assignee: "inv-member".parse().unwrap(),
            branch: pr(1).branch,
        },
    )
    .unwrap();
    team.advance_item(
        id,
        ItemPhase::InReview {
            pr: pr(1),
            reviews: vec![review("inv-r1", 3)],
        },
    )
    .unwrap();
    team.item_mut(id)
        .unwrap()
        .submit_review(&task(3), ReviewVerdict::RequestChanges, at(30))
        .unwrap();
    let mut state = State::new();
    state.insert_team(team).unwrap();
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let lead = owner.lead_of(&team_id(1)).unwrap();
    let waits = Exhausted {
        pr: pr(1),
        items: vec![id],
        passes: 0,
        requested_at: at(30),
    };
    owner
        .start_pass_after_answer(&team_id(1), &lead, &waits, &mut Look::default())
        .unwrap();
    let team = owner.parts.state.team(&team_id(1)).unwrap();
    assert!(
        matches!(team.item(id).unwrap().phase(), ItemPhase::Fixing {assignee, pr, ..}
        if assignee.as_str() == "inv-member" && pr.number.get().get() == 1)
    );
    assert_eq!(team.rounds().len(), 1);
}
