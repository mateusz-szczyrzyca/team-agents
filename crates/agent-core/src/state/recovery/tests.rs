use super::*;
use crate::state::PhaseKind;
use crate::state::fixtures::{at, drive, item, round_id, team, team_id, team_of};

/// Fresh identities with no external effects.
fn destination(n: u32, first_task: u32) -> RecoveryDestination {
    RecoveryDestination {
        id: team_id(n),
        created_at: at(100),
        crew: team_of(n, first_task).crew().clone(),
    }
}

/// Simulates saving and loading state at a crash boundary.
fn reload(state: &State) -> State {
    State::from_json(&state.to_json().unwrap()).unwrap()
}

#[test]
fn recovery_reservation_survives_removal_and_installs_exactly_once() {
    let mut state = State::new();
    let old = team(1);
    state.insert_team(old.clone()).unwrap();
    assert_eq!(
        state.prepare_recovery(&team_id(1), destination(2, 10), 1),
        Ok(RecoveryPreparation::Reserved(team_id(2)))
    );
    let mut state = reload(&state);
    let plan = state.recovery(&team_id(1)).unwrap().clone();
    assert_eq!(plan.source(), &old);
    assert_eq!(plan.ordinal().get(), 1);
    assert_eq!(plan.phase(), RecoveryPhase::Pending);
    assert_eq!(state.teams(), &[old.clone()]);
    assert_eq!(
        state.prepare_recovery(&team_id(1), destination(3, 20), 0),
        Ok(RecoveryPreparation::Reserved(team_id(2)))
    );
    assert_eq!(state.recovery(&team_id(1)), Some(&plan));
    assert_eq!(
        state.install_recovery(&team_id(1)),
        Err(RecoveryError::SourcePresent { team: team_id(1) })
    );
    assert_eq!(state.remove_team(&team_id(1)), Some(old));
    let mut state = reload(&state);
    assert_eq!(state.recovery(&team_id(1)), Some(&plan));
    assert_eq!(state.install_recovery(&team_id(1)), Ok(RecoveryInstallation::Installed));
    assert_eq!(state.teams(), &[plan.replacement().clone()]);
    state
        .team_mut(&team_id(2))
        .unwrap()
        .open_round(round_id(2), Some(item("1.2")), at(110))
        .unwrap();
    let before = reload(&state);
    let mut state = before.clone();
    assert_eq!(
        state.install_recovery(&team_id(1)),
        Ok(RecoveryInstallation::AlreadyInstalled)
    );
    assert_eq!(state, before);
    assert_eq!(state.recovery(&team_id(1)).unwrap().phase(), RecoveryPhase::Installed);
    assert!(state.remove_team(&team_id(2)).is_some());
    let mut state = reload(&state);
    assert_eq!(
        state.install_recovery(&team_id(1)),
        Ok(RecoveryInstallation::AlreadyInstalled)
    );
    assert_eq!(state.teams(), []);
}

#[test]
fn recovery_budget_is_durable_across_replacement_identities() {
    let mut state = State::new();
    state.insert_team(team(1)).unwrap();
    assert_eq!(
        state.prepare_recovery(&team_id(1), destination(2, 10), 0),
        Ok(RecoveryPreparation::Exhausted)
    );
    assert_eq!(state.recovery(&team_id(1)), None);
    assert_eq!(
        state.prepare_recovery(&team_id(1), destination(2, 10), 1),
        Ok(RecoveryPreparation::Reserved(team_id(2)))
    );
    assert!(state.remove_team(&team_id(1)).is_some());
    state.install_recovery(&team_id(1)).unwrap();
    let mut state = reload(&state);
    let before = state.clone();
    assert_eq!(
        state.prepare_recovery(&team_id(2), destination(3, 20), 1),
        Ok(RecoveryPreparation::Exhausted)
    );
    assert_eq!(state, before);
    assert_eq!(
        state.prepare_recovery(&team_id(2), destination(3, 20), 2),
        Ok(RecoveryPreparation::Reserved(team_id(3)))
    );
    assert_eq!(reload(&state).recovery(&team_id(2)).unwrap().ordinal().get(), 2);
}

#[test]
fn recovery_keeps_progress_and_other_teams_across_persistence() {
    let mut old = team(1);
    old.open_round(round_id(1), Some(item("1.2")), at(1)).unwrap();
    drive(&mut old, "1.2", PhaseKind::Done);
    old.close_round(&round_id(1), at(60), super::super::RoundEnd::Merged)
        .unwrap();
    let other = team_of(4, 30);
    let mut state = State::new();
    state.insert_team(old.clone()).unwrap();
    state.insert_team(other.clone()).unwrap();
    assert_eq!(
        state.prepare_recovery(&team_id(1), destination(2, 10), 1),
        Ok(RecoveryPreparation::Reserved(team_id(2)))
    );
    assert!(state.remove_team(&team_id(1)).is_some());
    let mut state = reload(&state);
    state.install_recovery(&team_id(1)).unwrap();
    let state = reload(&state);
    assert_eq!(state.team(&team_id(4)), Some(&other));
    assert_eq!(
        state.team(&team_id(2)).unwrap().item(item("1.2")),
        old.item(item("1.2"))
    );
    assert_eq!(state.teams().len(), 2);
}
