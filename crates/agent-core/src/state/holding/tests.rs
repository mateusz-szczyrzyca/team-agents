//! A hand-over through the state: only to an agent outside the team, of a task that is not in a run, and so that the
//! state it leaves loads again.

use crate::ids::{AgentName, InvestigatorName, TaskId};
use crate::kinds::Role;
use crate::state::{HandOverError, Holding, Participant, State, Timestamp};
use crate::status::TaskStatus;

/// The failed member's task of the 1.1 fixture: `codex-1`, a member of a team led by `inv-opus`.
fn task() -> TaskId {
    "task-20261120-144101-r7q2".parse().unwrap()
}

/// The state of 1.1, written by main.
fn old_state() -> State {
    State::from_json(include_str!("../../../tests/fixtures/state-before-renewals.json")).expect("a state of main loads")
}

/// When the hand-overs of these tests happen.
fn at() -> Timestamp {
    "2026-11-21T00:00:00Z".parse().unwrap()
}

fn normal(name: &str) -> Participant {
    Participant::Normal(name.parse::<AgentName>().unwrap())
}

fn investigator(name: &str) -> Participant {
    Participant::Investigator(name.parse::<InvestigatorName>().unwrap())
}

#[test]
fn a_task_handed_over_to_an_agent_outside_the_team_is_held_by_it_and_the_state_loads_again() {
    let mut state = old_state();

    state.hand_over_task(&task(), normal("claude-9"), at()).unwrap();

    let loaded = State::from_json(&state.to_json().unwrap()).expect("the state loads again");
    let member = loaded.find_task(&task()).unwrap().member;
    assert_eq!(member.agent(), &normal("claude-9"));
    assert_eq!(
        member.past_holders(),
        vec![Holding {
            agent: normal("codex-1"),
            until: at(),
        }]
    );
}

#[test]
fn a_task_is_not_handed_over_to_the_agent_that_holds_it() {
    let mut state = old_state();

    let refused = state.hand_over_task(&task(), normal("codex-1"), at());

    assert_eq!(refused, Err(HandOverError::SameHolder { task: task() }));
}

#[test]
fn a_task_is_not_handed_over_to_an_agent_of_its_own_team() {
    for agent in ["inv-opus", "inv-codex"] {
        let mut state = old_state();

        let refused = state.hand_over_task(&task(), investigator(agent), at());

        assert_eq!(
            refused,
            Err(HandOverError::InTeam {
                task: task(),
                agent: agent.to_owned(),
            }),
            "{agent} leads or reviews the team"
        );
    }
}

#[test]
fn a_task_in_the_middle_of_a_run_is_not_handed_over() {
    let mut state = old_state();
    {
        let (_, mut member) = state.find_task_mut(&task()).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member
            .begin_run("run-20261121-000000-a1a1".parse().unwrap(), at(), None)
            .unwrap();
    }

    let refused = state.hand_over_task(&task(), normal("claude-9"), at());

    assert_eq!(refused, Err(HandOverError::InRun { task: task() }));
}

#[test]
fn a_task_that_is_not_in_the_state_or_is_a_workers_or_a_reviewers_is_not_handed_over() {
    let mut state = old_state();
    let unknown: TaskId = "task-20261121-000000-zzzz".parse().unwrap();
    let worker: TaskId = "task-20261111-193003-k3x9".parse().unwrap();
    let reviewer: TaskId = "task-20261111-193007-k3x9".parse().unwrap();

    assert_eq!(
        state.hand_over_task(&unknown, normal("claude-9"), at()),
        Err(HandOverError::NoTask { task: unknown })
    );
    assert_eq!(
        state.hand_over_task(&worker, normal("claude-9"), at()),
        Err(HandOverError::Role {
            task: worker,
            role: Role::Worker,
        })
    );
    assert_eq!(
        state.hand_over_task(&reviewer, investigator("inv-kimi"), at()),
        Err(HandOverError::Role {
            task: reviewer,
            role: Role::Reviewer,
        })
    );
}
