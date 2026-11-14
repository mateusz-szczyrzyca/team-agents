use std::num::NonZeroU64;
use std::path::PathBuf;

use agent_core::ids::{ItemId, RoundId};
use agent_core::state::RoundEnd;
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{DelegationRequest, DelegationStatus, RequestId};

use super::{OwnerError, open_rounds_of, rounds_for};
use crate::daemon::team::Why;
use crate::fixtures::{at, solo_state, task, team_id};

/// The ID of the sample round number `n`.
fn round(n: u32) -> RoundId {
    format!("round-20261111-1530{n:02}-q1w2").parse().unwrap()
}

#[test]
fn the_open_rounds_of_a_team_are_its_rounds_that_have_not_ended_oldest_first() {
    let mut state = solo_state();
    let team = team_id(0);
    let first: ItemId = "1.1".parse().unwrap();
    let second: ItemId = "1.2".parse().unwrap();
    state.open_round(&team, round(1), Some(first), at(1)).unwrap();
    state.open_round(&team, round(2), Some(second), at(2)).unwrap();
    state
        .team_mut(&team)
        .unwrap()
        .crew_mut()
        .find_mut(&task(1))
        .unwrap()
        .1
        .transition(TaskStatus::Stopped)
        .unwrap();
    state.close_round(&round(1), at(3), RoundEnd::Abandoned).unwrap();

    let open = open_rounds_of(&state, &team).unwrap();

    assert_eq!(open, vec![(round(2), Some(second))]);
}

#[test]
fn a_team_without_rounds_has_no_open_round() {
    let state = solo_state();

    let open = open_rounds_of(&state, &team_id(0)).unwrap();

    assert_eq!(open, Vec::new());
}

#[test]
fn a_team_that_is_not_in_the_state_is_lost_and_not_a_team_without_rounds() {
    let state = solo_state();

    let err = open_rounds_of(&state, &team_id(7)).unwrap_err();

    assert!(matches!(err, OwnerError::LostTeam { team } if team == team_id(7)));
}

#[test]
fn a_delegation_whose_requester_is_not_in_the_team_is_lost_and_not_a_requester_without_rounds() {
    let state = solo_state();
    let missing = task(7);
    let request = DelegationRequest {
        id: RequestId::new(NonZeroU64::new(7).unwrap()),
        from: "claude-1".parse().unwrap(),
        from_task: missing.clone(),
        repo: "example".parse().unwrap(),
        branch: "agents/claude-1/task-7".parse().unwrap(),
        worktree: PathBuf::from("/srv/agents/worktrees/example/task-7"),
        request: "List the files under src/.".to_owned(),
        context: String::new(),
        status: DelegationStatus::Starting,
        worker: None,
        helper_task: None,
        summary: String::new(),
    };
    let why = Why::Delegated(Box::new(request));

    let err = rounds_for(&state, &task(1), Some(&why)).unwrap_err();

    assert!(matches!(err, OwnerError::LostTask { task } if task == missing));
}
