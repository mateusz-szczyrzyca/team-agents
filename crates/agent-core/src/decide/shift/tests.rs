//! Shift boundaries, role exclusions, candidate availability and stable rotation.

use super::*;
use crate::kinds::Role;
use crate::state::fixtures::*;
use crate::state::{Crew, ItemPhase, MergeMode, NewRun, RunEnd, TeamMembers};
use crate::status::TaskStatus;

/// A working lead on a round whose PR is already open.
fn setup(solo: bool, pull_request: bool) -> State {
    let crew = if solo {
        Crew::solo(member(1, normal("codex-1"))).unwrap()
    } else {
        Crew::team(TeamMembers {
            coordinator: member(1, normal("codex-1")),
            members: vec![member(2, normal("claude-1")), member(3, normal("gemini-1"))],
            workers: vec![member(4, worker("worker-1"))],
        })
        .unwrap()
    };
    let mut team = Team::new(team_id(1), repo(), at(0), MergeMode::Autopilot, list(), crew);
    team.open_round(round_id(1), Some(item("1.2")), at(0)).unwrap();
    team.advance_item(
        item("1.2"),
        ItemPhase::InProgress {
            assignee: agent("codex-1"),
            branch: branch("agents/codex-1/item-1-2"),
        },
    )
    .unwrap();
    if pull_request {
        team.deliver_round_with_pr(&round_id(1), pr(7)).unwrap();
    }
    team.begin_run(
        &task(1),
        NewRun {
            id: run(1),
            at: at(0),
            session: None,
            rounds: vec![round_id(1)],
        },
    )
    .unwrap();
    team.find_mut(&task(1))
        .unwrap()
        .1
        .transition(TaskStatus::Running)
        .unwrap();
    let mut state = State::new();
    state.insert_team(team).unwrap();
    state
}

/// Evaluate one task at a chosen clock value.
fn check(state: &State, number: u32, minutes: u32, seconds: u32) -> ShiftCheck {
    let replacements = [agent("codex-1"), agent("claude-1"), agent("gemini-1")];
    shift_target(
        &task(number),
        state.team(&team_id(1)).unwrap(),
        state,
        &ShiftLimits {
            lead_shift_minutes: minutes,
            now: at(seconds),
            replacement_agents: &replacements,
        },
    )
}

/// Occupy a candidate without changing the lead's run.
fn busy(state: &mut State, number: u32) {
    state
        .find_task_mut(&task(number))
        .unwrap()
        .1
        .begin_run(run(number), at(0), None)
        .unwrap();
    state
        .find_task_mut(&task(number))
        .unwrap()
        .1
        .transition(TaskStatus::Running)
        .unwrap();
}

/// Give an excluded role enough running work on a PR round to otherwise be due.
fn work_on_round(state: &mut State, number: u32) {
    state
        .team_mut(&team_id(1))
        .unwrap()
        .begin_run(
            &task(number),
            NewRun {
                id: run(number),
                at: at(0),
                session: None,
                rounds: vec![round_id(1)],
            },
        )
        .unwrap();
}

#[test]
fn not_due_at_119_due_at_120() {
    let state = setup(false, true);
    assert_eq!(check(&state, 1, 120, 119 * 60), ShiftCheck::NotDue);
    assert_eq!(check(&state, 1, 120, 120 * 60), ShiftCheck::To(agent("claude-1")));
}

#[test]
fn not_due_when_zero() {
    assert_eq!(check(&setup(false, true), 1, 0, 7200), ShiftCheck::NotDue);
}
#[test]
fn not_due_for_member_task() {
    let mut state = setup(false, true);
    work_on_round(&mut state, 2);
    assert_eq!(check(&state, 2, 120, 7200), ShiftCheck::NotDue);
}
#[test]
fn not_due_for_worker_task() {
    let mut state = setup(false, true);
    work_on_round(&mut state, 4);
    assert_eq!(check(&state, 4, 120, 7200), ShiftCheck::NotDue);
}
#[test]
fn not_due_for_reviewer_task() {
    let mut state = setup(false, true);
    state.add_reviewer(&team_id(1), member(5, inv("inv-codex"))).unwrap();
    work_on_round(&mut state, 5);
    assert_eq!(state.find_task(&task(5)).unwrap().role, Role::Reviewer);
    assert_eq!(check(&state, 5, 120, 7200), ShiftCheck::NotDue);
}

/// Stop a lead in a status that must never be interrupted by a shift.
fn inactive(status: TaskStatus) -> State {
    let mut state = setup(false, true);
    let mut lead = state.find_task_mut(&task(1)).unwrap().1;
    lead.end_run(at(7200), RunEnd::Stopped).unwrap();
    lead.transition(status).unwrap();
    state
}
#[test]
fn not_due_when_paused() {
    assert_eq!(check(&inactive(TaskStatus::Paused), 1, 120, 7200), ShiftCheck::NotDue);
}
#[test]
fn not_due_when_done() {
    assert_eq!(check(&inactive(TaskStatus::Done), 1, 120, 7200), ShiftCheck::NotDue);
}
#[test]
fn not_due_without_pull_request() {
    assert_eq!(check(&setup(false, false), 1, 120, 7200), ShiftCheck::NotDue);
}

#[test]
fn b_busy_goes_to_c() {
    let mut state = setup(false, true);
    busy(&mut state, 2);
    assert_eq!(check(&state, 1, 120, 7200), ShiftCheck::To(agent("gemini-1")));
}
#[test]
fn all_busy_is_none_free() {
    let mut state = setup(false, true);
    busy(&mut state, 2);
    busy(&mut state, 3);
    assert_eq!(check(&state, 1, 120, 7200), ShiftCheck::NoneFree);
}
#[test]
fn team_worker_is_skipped() {
    let mut state = setup(false, true);
    busy(&mut state, 2);
    busy(&mut state, 3);
    assert_eq!(check(&state, 1, 120, 7200), ShiftCheck::NoneFree);
}
#[test]
fn candidate_waiting_for_answers_is_skipped() {
    let mut state = setup(false, true);
    busy(&mut state, 2);
    let mut candidate = state.find_task_mut(&task(2)).unwrap().1;
    candidate.end_run(at(1), RunEnd::Stopped).unwrap();
    candidate.transition(TaskStatus::Questions).unwrap();
    assert_eq!(check(&state, 1, 120, 7200), ShiftCheck::To(agent("gemini-1")));
}
#[test]
fn solo_goes_to_next_free_replacement_agent() {
    let state = setup(true, true);
    assert_eq!(check(&state, 1, 120, 7200), ShiftCheck::To(agent("claude-1")));
}
#[test]
fn solo_skips_agent_of_another_open_team() {
    let mut state = setup(true, true);
    let mut other = team_of(2, 10);
    other
        .find_mut(&task(11))
        .unwrap()
        .1
        .held_task(normal("claude-1"), at(0));
    state.insert_team(other).unwrap();
    assert_eq!(check(&state, 1, 120, 7200), ShiftCheck::To(agent("gemini-1")));
}
#[test]
fn solo_skips_provider_refused_agent() {
    let mut state = setup(true, true);
    state.keep_refused_until(agent("claude-1"), at(8000));
    assert_eq!(check(&state, 1, 120, 7200), ShiftCheck::To(agent("gemini-1")));
}
#[test]
fn accumulated_and_running_work_are_added() {
    let mut state = setup(false, true);
    let mut lead = state.find_task_mut(&task(1)).unwrap().1;
    lead.end_run(at(3600), RunEnd::Stopped).unwrap();
    state
        .team_mut(&team_id(1))
        .unwrap()
        .begin_run(
            &task(1),
            NewRun {
                id: run(6),
                at: at(4000),
                session: None,
                rounds: vec![round_id(1)],
            },
        )
        .unwrap();
    assert_eq!(check(&state, 1, 120, 7599), ShiftCheck::NotDue);
    assert_eq!(check(&state, 1, 120, 7600), ShiftCheck::To(agent("claude-1")));
}
#[test]
fn circle_a_b_c_a() {
    let mut state = setup(false, true);
    for (n, (next, target_task)) in [("claude-1", 2), ("gemini-1", 3), ("codex-1", 2)]
        .into_iter()
        .enumerate()
    {
        let seconds = u32::try_from(n + 1).unwrap() * 7200;
        assert_eq!(check(&state, 1, 120, seconds), ShiftCheck::To(agent(next)));
        let previous = state.find_task(&task(1)).unwrap().member.agent().clone();
        state
            .find_task_mut(&task(1))
            .unwrap()
            .1
            .end_run(at(seconds), RunEnd::Stopped)
            .unwrap();
        state
            .find_task_mut(&task(1))
            .unwrap()
            .1
            .held_task(normal(next), at(seconds));
        state
            .find_task_mut(&task(target_task))
            .unwrap()
            .1
            .held_task(previous, at(seconds));
        state
            .team_mut(&team_id(1))
            .unwrap()
            .begin_run(
                &task(1),
                NewRun {
                    id: run(u32::try_from(n + 10).unwrap()),
                    at: at(seconds),
                    session: None,
                    rounds: vec![round_id(1)],
                },
            )
            .unwrap();
    }
}

#[test]
fn solo_skips_agent_holding_another_open_solo_task() {
    let mut state = setup(true, true);
    let other = Team::new(
        team_id(2),
        repo(),
        at(0),
        MergeMode::Autopilot,
        list(),
        Crew::solo(member(10, normal("claude-1"))).unwrap(),
    );
    assert_eq!(other.find(&task(10)).unwrap().1.status(), TaskStatus::Queued);
    state.insert_team(other).unwrap();
    assert_eq!(check(&state, 1, 120, 7200), ShiftCheck::To(agent("gemini-1")));
}

#[test]
fn ended_run_pr_does_not_authorize_a_new_assigned_round_without_pr() {
    let mut state = setup(true, true);
    state
        .find_task_mut(&task(1))
        .unwrap()
        .1
        .end_run(at(7200), RunEnd::Stopped)
        .unwrap();
    let mut team = state.team_mut(&team_id(1)).unwrap();
    team.open_round(round_id(2), Some(item("2.1")), at(7200)).unwrap();
    team.advance_item(
        item("2.1"),
        ItemPhase::InProgress {
            assignee: agent("codex-1"),
            branch: branch("agents/codex-1/item-2-1"),
        },
    )
    .unwrap();
    assert_eq!(check(&state, 1, 120, 7200), ShiftCheck::NotDue);
}

#[test]
fn format_span_formats_hours_and_minutes_correctly() {
    use super::format_span;

    assert_eq!(format_span(0), "0m");
    assert_eq!(format_span(45), "45m");
    assert_eq!(format_span(60), "1h");
    assert_eq!(format_span(100), "1h40m");
    assert_eq!(format_span(120), "2h");
    assert_eq!(format_span(150), "2h30m");
}
