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
    assert_eq!(state.teams(), std::slice::from_ref(&old));
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

#[test]
fn recovery_rejects_reused_team_and_task_identities_without_reserving() {
    use crate::state::fixtures::task;
    for (proposed, expected) in [
        (destination(1, 10), InvalidState::DuplicateTeam { team: team_id(1) }),
        (destination(4, 10), InvalidState::DuplicateTeam { team: team_id(4) }),
        (
            destination(2, 1),
            InvalidState::DuplicateTask {
                task: task(1),
                first: team_id(1),
                second: team_id(2),
            },
        ),
        (
            destination(2, 30),
            InvalidState::DuplicateTask {
                task: task(30),
                first: team_id(4),
                second: team_id(2),
            },
        ),
    ] {
        let mut state = State::new();
        state.insert_team(team(1)).unwrap();
        state.insert_team(team_of(4, 30)).unwrap();
        let before = state.clone();
        assert_eq!(
            state.prepare_recovery(&team_id(1), proposed, 1),
            Err(RecoveryError::Insert { source: expected })
        );
        assert_eq!(state, before);
    }
}

#[test]
fn recovery_reservations_protect_identities_after_source_removal() {
    let mut state = State::new();
    state.insert_team(team(1)).unwrap();
    state.insert_team(team_of(4, 30)).unwrap();
    state.prepare_recovery(&team_id(1), destination(2, 10), 1).unwrap();
    state.remove_team(&team_id(1)).unwrap();
    let before = reload(&state);
    for n in [1, 2] {
        assert_eq!(
            state.prepare_recovery(&team_id(4), destination(n, 20), 1),
            Err(RecoveryError::Insert {
                source: InvalidState::DuplicateTeam { team: team_id(n) }
            })
        );
        assert_eq!(state, before);
    }
    assert_eq!(
        state.prepare_recovery(&team_id(4), destination(3, 10), 1),
        Err(RecoveryError::Insert {
            source: InvalidState::DuplicateTask {
                task: crate::state::fixtures::task(10),
                first: team_id(2),
                second: team_id(3)
            }
        })
    );
    assert_eq!(state, before);
}

#[test]
fn loading_refuses_forged_recovery_plans_and_restart_lineages() {
    let mut state = State::new();
    state.insert_team(team(1)).unwrap();
    state.prepare_recovery(&team_id(1), destination(2, 10), 2).unwrap();
    state.remove_team(&team_id(1)).unwrap();
    state.install_recovery(&team_id(1)).unwrap();
    state.prepare_recovery(&team_id(2), destination(3, 20), 2).unwrap();
    let valid = serde_json::to_value(&state).unwrap();
    for case in 0..9 {
        let mut value = valid.clone();
        match case {
            0 => value["recoveries"][0]["ordinal"] = 2.into(),
            1 => value["recoveries"][1]["ordinal"] = 1.into(),
            2 => value["recoveries"][0]["phase"] = "pending".into(),
            3 => value["recoveries"][0]["replacement"]["repo"] = "different-repo".into(),
            4 => value["recoveries"][0]["replacement"]["id"] = serde_json::to_value(team_id(1)).unwrap(),
            5 => value["recoveries"][1]["replacement"]["id"] = serde_json::to_value(team_id(1)).unwrap(),
            6 => value["recoveries"][1]["replacement"]["crew"] = value["recoveries"][0]["source"]["crew"].clone(),
            7 => {
                let duplicate = value["recoveries"][0].clone();
                value["recoveries"].as_array_mut().unwrap().push(duplicate);
            }
            8 => value["recoveries"][0]["replacement"]["crew"] = value["recoveries"][0]["source"]["crew"].clone(),
            _ => panic!("unknown mutation case {case}"),
        }
        let result = State::from_json(&serde_json::to_string(&value).unwrap());
        assert!(
            matches!(result, Err(super::super::StateError::Malformed { .. })),
            "case {case}: {result:?}"
        );
    }
    assert_eq!(
        State::from_json(&serde_json::to_string(&valid).unwrap()).unwrap(),
        state
    );
}

#[test]
fn legacy_state_without_recovery_records_starts_with_an_unused_budget() {
    // The stopped documentation team is copied from the pre-recovery schema-11 fixture.
    let mut state = State::from_json(include_str!("legacy.json")).unwrap();
    let old = state.teams().first().unwrap().clone();
    assert_eq!(state.recovery(old.id()), None);
    assert!(!state.to_json().unwrap().contains("\"recoveries\""));
    assert_eq!(
        state.prepare_recovery(old.id(), destination(2, 10), 1),
        Ok(RecoveryPreparation::Reserved(team_id(2)))
    );
    let state = reload(&state);
    let plan = state.recovery(old.id()).unwrap();
    assert_eq!(plan.ordinal().get(), 1);
    assert_eq!(plan.source(), &old);
    assert_eq!(plan.replacement().repo(), old.repo());
    assert_eq!(plan.replacement().task_list(), old.task_list());
    assert_eq!(plan.replacement().rounds(), []);
}

#[test]
fn recovery_unknown_sources_and_installation_conflicts_leave_state_unchanged() {
    let mut state = State::new();
    assert_eq!(
        state.prepare_recovery(&team_id(1), destination(2, 10), 1),
        Err(RecoveryError::Unknown { team: team_id(1) })
    );
    assert_eq!(
        state.install_recovery(&team_id(1)),
        Err(RecoveryError::Unknown { team: team_id(1) })
    );
    assert_eq!(state, State::new());
    state.insert_team(team(1)).unwrap();
    state.prepare_recovery(&team_id(1), destination(2, 10), 1).unwrap();
    state.remove_team(&team_id(1)).unwrap();
    state.insert_team(team_of(4, 10)).unwrap();
    let before = state.clone();
    assert_eq!(
        state.install_recovery(&team_id(1)),
        Err(RecoveryError::Insert {
            source: InvalidState::DuplicateTask {
                task: crate::state::fixtures::task(10),
                first: team_id(4),
                second: team_id(2),
            }
        })
    );
    assert_eq!(state, before);
    state.remove_team(&team_id(4)).unwrap();
    assert_eq!(state.install_recovery(&team_id(1)), Ok(RecoveryInstallation::Installed));
}

#[test]
fn recovery_rejects_uninstalled_or_changed_predecessors_without_mutation() {
    for case in 0..3 {
        let mut state = State::new();
        state.insert_team(team(1)).unwrap();
        state.prepare_recovery(&team_id(1), destination(2, 10), 2).unwrap();
        state.remove_team(&team_id(1)).unwrap();
        if case != 0 {
            state.install_recovery(&team_id(1)).unwrap();
            state.remove_team(&team_id(2)).unwrap();
        }
        let mut value = serde_json::to_value(state.recovery(&team_id(1)).unwrap().replacement()).unwrap();
        match case {
            0 => {}
            1 => value["repo"] = "different-repo".into(),
            2 => {
                let list = value["task_list"].as_str().unwrap().to_owned();
                value["task_list"] = format!("{list}\n").into();
            }
            _ => panic!("unknown case {case}"),
        }
        state.insert_team(serde_json::from_value(value).unwrap()).unwrap();
        let before = reload(&state);
        assert_eq!(
            state.prepare_recovery(&team_id(2), destination(3, 20), 2),
            Err(RecoveryError::InvalidPredecessor { team: team_id(2) }),
            "case {case}"
        );
        assert_eq!(state, before);
    }
}

#[test]
fn recovery_refuses_a_new_source_reusing_another_lineages_task() {
    let mut state = State::new();
    state.insert_team(team(1)).unwrap();
    state.prepare_recovery(&team_id(1), destination(2, 10), 1).unwrap();
    state.remove_team(&team_id(1)).unwrap();
    state.insert_team(team_of(4, 1)).unwrap();
    let before = reload(&state);
    assert_eq!(
        state.prepare_recovery(&team_id(4), destination(3, 20), 1),
        Err(RecoveryError::Insert {
            source: InvalidState::DuplicateTask {
                task: crate::state::fixtures::task(1),
                first: team_id(1),
                second: team_id(4),
            }
        })
    );
    assert_eq!(state, before);
}
