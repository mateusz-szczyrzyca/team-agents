//! A task the human starts anew with `agent renew`: what the state keeps of it, and what it refuses.

use super::*;
use crate::decide::ladder::Trouble;

/// The failed lead of the state of `main` before renewals: it has seven runs and a session.
fn task() -> TaskId {
    "task-20261120-144101-r7q2".parse().unwrap()
}

/// A worker of that state, which has no run.
fn worker() -> TaskId {
    "task-20261111-193003-k3x9".parse().unwrap()
}

/// A reviewer of that state, which has no run.
fn reviewer() -> TaskId {
    "task-20261111-193007-k3x9".parse().unwrap()
}

/// A state written by `main` before renewals existed.
fn old_state() -> State {
    State::from_json(include_str!("../../tests/fixtures/state-before-renewals.json")).expect("a state of main loads")
}

/// When the human renews, after every run of the fixture.
fn late() -> Timestamp {
    "2026-11-21T00:00:00Z".parse().unwrap()
}

/// How many new sessions of `task` are counted.
fn used(state: &State, task: &TaskId) -> u32 {
    state
        .find_task(task)
        .expect("the task")
        .member
        .renewals()
        .map_or(0, Renewals::used)
}

#[test]
fn a_lead_with_runs_awaits_a_new_session_and_no_new_session_is_counted() {
    let mut state = old_state();
    let before = used(&state, &task());

    let begun = state.begin_human_session(&task(), late());

    assert!(matches!(begun, Ok(true)), "{begun:?}");
    let member = state.find_task(&task()).expect("the task").member;
    assert!(member.awaits_new_session());
    assert_eq!(member.last_session(), None, "no session before it is resumed");
    assert_eq!(member.renewals().map(Renewals::reason), Some(Trouble::Human));
    assert_eq!(used(&state, &task()), before, "a renewal of the human is not counted");
}

#[test]
fn a_task_of_a_worker_or_a_reviewer_is_refused_with_its_role() {
    for (task, expected) in [(worker(), Role::Worker), (reviewer(), Role::Reviewer)] {
        let mut state = old_state();

        let begun = state.begin_human_session(&task, late());

        assert!(
            matches!(&begun, Err(RenewTaskError::Role { task: refused, role }) if *refused == task && *role == expected),
            "{begun:?}"
        );
        let member = state.find_task(&task).expect("the task").member;
        assert!(!member.awaits_new_session(), "{task}: nothing was recorded");
    }
}

#[test]
fn a_task_that_is_not_in_the_state_is_refused() {
    let mut state = old_state();
    let missing: TaskId = "task-20261111-193059-zzzz".parse().unwrap();

    let begun = state.begin_human_session(&missing, late());

    assert!(
        matches!(&begun, Err(RenewTaskError::NoTask { task }) if *task == missing),
        "{begun:?}"
    );
}

#[test]
fn a_task_with_no_run_has_no_session_to_begin() {
    let mut state = old_state();
    let fresh: TaskId = "task-20261121-100000-frsh".parse().unwrap();
    let team = Team::new(
        "team-20261121-100000-frsh".parse().unwrap(),
        "team-agents".parse().unwrap(),
        "2026-11-21T10:00:00Z".parse().unwrap(),
        MergeMode::NoMerge,
        TaskListText::new("Solo task.\n".to_owned()).unwrap(),
        Crew::solo(Member::new(
            fresh.clone(),
            Participant::Normal("claude-1".parse().unwrap()),
        ))
        .unwrap(),
    );
    state.insert_team(team).unwrap();

    let begun = state.begin_human_session(&fresh, late());

    assert!(matches!(begun, Ok(false)), "{begun:?}");
    assert_eq!(state.find_task(&fresh).expect("the task").member.renewals(), None);
}

#[test]
fn the_human_reason_is_written_as_human_and_survives_a_restart() {
    let mut state = old_state();
    state.begin_human_session(&task(), late()).expect("the lead ran");

    let member = state.find_task(&task()).expect("the task").member;
    let renewals = serde_json::to_value(member.renewals().expect("the renewal")).expect("it serializes");
    assert_eq!(renewals["reason"], serde_json::json!("human"));
    assert_eq!(
        serde_json::to_value(Trouble::Human).unwrap(),
        serde_json::json!("human")
    );
    assert_eq!(serde_json::from_str::<Trouble>("\"human\"").unwrap(), Trouble::Human);
    let loaded = State::from_json(&state.to_json().expect("the state saves")).expect("a restart reads it");
    assert_eq!(loaded, state);
}

#[test]
fn the_states_written_before_the_human_reason_load_as_they_did() {
    let before_renewals = old_state();
    let found = before_renewals.find_task(&task()).expect("the task");
    assert_eq!(found.member.renewals(), None);
    assert_eq!(found.member.runs().len(), 7);

    let before_no_tool_call = State::from_json(include_str!("../../tests/fixtures/state-before-no-tool-call.json"))
        .expect("a state before the no-tool retry loads");
    let found = before_no_tool_call.find_task(&task()).expect("the task");
    assert_eq!(found.member.renewals().map(Renewals::reason), Some(Trouble::Hang));
    assert_eq!(found.member.runs().len(), 3);
}
