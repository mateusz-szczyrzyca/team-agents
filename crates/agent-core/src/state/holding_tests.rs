//! Who held a task: a task given to another agent keeps the record of the agent it was taken from, so that `agent status`
//! and the history can say who has it and who had it.

use super::*;
use crate::ids::AgentName;
use crate::status::TaskStatus;

/// The task of these tests: the failed member of the 1.1 fixture.
fn task() -> TaskId {
    "task-20261120-144101-r7q2".parse().unwrap()
}

/// The state of 1.1, written by main: a member that ended seven runs without a status, before anyone held it.
fn old_state() -> State {
    State::from_json(include_str!("../../tests/fixtures/state-before-renewals.json")).expect("a state of main loads")
}

/// When a hand-over of these tests happens.
fn handed_at() -> Timestamp {
    "2026-11-21T00:00:00Z".parse().unwrap()
}

#[test]
fn a_state_written_before_handovers_loads_with_no_record() {
    let state = old_state();

    let found = state.find_task(&task()).expect("the task is there");
    assert_eq!(found.member.status(), TaskStatus::Failed);
    assert_eq!(found.member.past_holders(), Vec::<Holding>::new());
}

#[test]
fn a_task_given_to_another_agent_keeps_who_held_it_before() {
    let mut state = old_state();
    let holder: AgentName = "claude-2".parse().expect("an agent name");
    let (_, mut member) = state.find_task_mut(&task()).expect("the task");
    member.held_task(Participant::Normal(holder.clone()), handed_at());

    let found = state.find_task(&task()).expect("the task");
    assert_eq!(
        found.member.past_holders(),
        &[Holding {
            agent: Participant::Normal("codex-1".parse().expect("the agent of the fixture")),
            until: handed_at(),
        }]
    );
    assert_eq!(found.member.agent(), &Participant::Normal(holder));
}

#[test]
fn a_state_with_two_holders_is_written_and_read_back() {
    let mut state = old_state();
    {
        let (_, mut member) = state.find_task_mut(&task()).expect("the task");
        member.held_task(
            Participant::Normal("claude-2".parse().expect("an agent name")),
            handed_at(),
        );
        member.held_task(
            Participant::Investigator("inv-kimi".parse().expect("an investigator name")),
            handed_at(),
        );
    }
    let saved = state.to_json().expect("the state saves");

    let loaded = State::from_json(&saved).expect("it loads again");

    let found = loaded.find_task(&task()).expect("the task");
    assert_eq!(
        found.member.agent(),
        &Participant::Investigator("inv-kimi".parse().expect("an investigator name"))
    );
    assert_eq!(found.member.past_holders().len(), 2);
    assert_eq!(loaded, state);
}

#[test]
fn the_record_is_not_written_for_a_task_nobody_else_held() {
    let state = old_state();
    let written: serde_json::Value = serde_json::from_str(&state.to_json().expect("the state saves")).expect("json");
    let member = &written["teams"][0]["crew"]["members"][0];

    assert!(member.get("past_holders").is_none(), "{member}");
}
