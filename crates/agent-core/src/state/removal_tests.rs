//! Removing a team from the state: the team and the wake-up records its tasks keep go with it; the other teams stay,
//! and removing a team that is not there changes nothing.

use super::*;
use super::fixtures::{task, team_id, team_of};

/// A state with two teams: team 1 with tasks 1 to 3 and team 2 with tasks 4 to 6.
fn two_teams() -> State {
    let mut state = State::new();
    state.insert_team(team_of(1, 1)).unwrap();
    state.insert_team(team_of(2, 4)).unwrap();
    state
}

#[test]
fn removing_a_team_takes_it_and_its_tasks_out_and_keeps_the_other_team() {
    let mut state = two_teams();

    let removed = state.remove_team(&team_id(1));

    assert_eq!(removed.map(|team| team.id().clone()), Some(team_id(1)));
    assert!(state.team(&team_id(1)).is_none());
    assert!(state.find_task(&task(1)).is_none());
    assert!(state.find_task(&task(3)).is_none());
    assert!(state.team(&team_id(2)).is_some());
    assert!(state.find_task(&task(4)).is_some());
}

#[test]
fn removing_a_team_forgets_the_wake_records_of_its_tasks_only() {
    let mut state = two_teams();
    let pending = WakeDelivery::Pending {
        message: None,
        count: 1,
    };
    state.record_wake_delivery(task(2), Some(pending));
    state.record_wake_delivery(task(5), Some(pending));

    state.remove_team(&team_id(1));

    assert_eq!(state.wake_delivery(&task(2)), None);
    assert_eq!(state.wake_delivery(&task(5)), Some(pending));
}

#[test]
fn removing_an_unknown_team_changes_nothing() {
    let mut state = two_teams();
    let before = state.clone();

    let removed = state.remove_team(&team_id(9));

    assert!(removed.is_none());
    assert_eq!(state, before);
}
