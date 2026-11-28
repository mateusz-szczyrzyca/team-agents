//! Holder changes reset accumulated work; legacy states have no accumulated work.

use super::*;
use crate::decide::ladder::Trouble;

/// The legacy fixture with work recorded on the task before a holder change.
fn worked_state() -> State {
    fn record(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                if object.get("task").and_then(serde_json::Value::as_str) == Some(task().as_str()) {
                    object.insert("lead_worked_secs".into(), 7200.into());
                }
                for child in object.values_mut() {
                    record(child);
                }
            }
            serde_json::Value::Array(values) => values.iter_mut().for_each(record),
            _ => {}
        }
    }
    let mut json: serde_json::Value = serde_json::from_str(&old_state().to_json().unwrap()).unwrap();
    record(&mut json);
    State::from_json(&json.to_string()).unwrap()
}

#[test]
fn worked_secs_zero_after_takeover() {
    let mut state = worked_state();
    assert_eq!(state.find_task(&task()).unwrap().member.lead_worked_secs(), 7200);
    state.hand_over_task(&task(), normal("claude-9"), at()).unwrap();
    assert_eq!(state.find_task(&task()).unwrap().member.lead_worked_secs(), 0);
}

#[test]
fn worked_secs_zero_after_swap() {
    let mut state = worked_state();
    state
        .hand_over_task_as(&task(), normal("claude-9"), Choice::Human, at())
        .unwrap();
    assert_eq!(state.find_task(&task()).unwrap().member.lead_worked_secs(), 0);
}

#[test]
fn worked_secs_zero_after_shift() {
    let mut state = worked_state();
    state
        .hand_over_task_as(&task(), normal("claude-9"), Choice::Human, at())
        .unwrap();
    state.begin_holder_session(&task(), Trouble::Shift, at()).unwrap();
    assert_eq!(state.find_task(&task()).unwrap().member.lead_worked_secs(), 0);
    assert_eq!(
        state.find_task(&task()).unwrap().member.renewals().unwrap().reason(),
        Trouble::Shift
    );
}

#[test]
fn state_without_worked_secs_reads_as_zero() {
    let state = old_state();
    assert_eq!(state.find_task(&task()).unwrap().member.lead_worked_secs(), 0);
}

#[test]
fn team_tasks_trade_holders_and_record_human_handover() {
    let mut state = worked_state();
    let team_id = state.teams()[0].id().clone();
    let coord_task = state.teams()[0].crew().coordinator().unwrap().task().clone();
    let member_task = task();
    let coord_before = state.find_task(&coord_task).unwrap().member.agent().clone();
    let member_before = state.find_task(&member_task).unwrap().member.agent().clone();

    state
        .trade_team_tasks(&team_id, &coord_task, &member_task, at())
        .unwrap();

    let coord_after = state.find_task(&coord_task).unwrap().member;
    let member_after = state.find_task(&member_task).unwrap().member;

    assert_eq!(coord_after.agent(), &member_before);
    assert_eq!(member_after.agent(), &coord_before);
    assert_eq!(coord_after.lead_worked_secs(), 0);
    assert_eq!(member_after.lead_worked_secs(), 0);
    assert_eq!(coord_after.past_holders().last().unwrap().agent, coord_before);
    assert_eq!(member_after.past_holders().last().unwrap().agent, member_before);
}
