use agent_core::ids::{ItemId, RoundId};
use agent_core::state::RoundEnd;

use super::{OwnerError, open_rounds_of};
use crate::fixtures::{at, solo_state, team_id};

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
