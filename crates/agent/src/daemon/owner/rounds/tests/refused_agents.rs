//! An agent its provider refused takes no task over before its return time, and takes one again once that time has
//! passed; the state written by the refusal is the only thing that changes between the two.

use agent_core::ids::{AgentName, TaskId};
use agent_core::status::TaskStatus;

use crate::daemon::owner::fixture::Daemon;

/// The name of the outside candidate that the refusal concerns.
fn candidate() -> AgentName {
    "claude-2".parse().expect("a valid agent name")
}

/// A waiting ordinary member in a team with a candidate-free investigator available as a replacement.
fn waiting_team_state() -> (agent_core::state::State, agent_core::ids::TeamId, TaskId) {
    let mut state = crate::fixtures::team_state();
    let task = crate::fixtures::task(2);
    state
        .start_waiting(&task, crate::fixtures::at(0))
        .expect("the task is there");
    (state, crate::fixtures::team_id(0), task)
}

/// Enables a replacement candidate by name; the sample investigator inv-codex needs its config entry too.
fn configure_candidate(daemon: &super::super::fixture::Daemon, name: &str) {
    let extra = if name == "inv-codex" {
        "[investigators.inv-codex]\ncli = \"claude\"\n"
    } else {
        ""
    };
    daemon.configure(&format!(
        "\n{extra}[replacement]\nagents = [\"{name}\"]\nbusy_after = 1\n"
    ));
}

/// Puts `claude-2` in a solo team of its own, outside the waiting member's team and with the requested status.
fn candidate_outside_with_status(state: &mut agent_core::state::State, status: TaskStatus) {
    use agent_core::state::{Crew, Member, MergeMode, TaskListText, Team};

    let task = crate::fixtures::task(5);
    let team = crate::fixtures::team_id(1);
    state
        .insert_team(Team::new(
            team.clone(),
            crate::fixtures::repo(),
            crate::fixtures::at(0),
            MergeMode::NoMerge,
            TaskListText::new("Solo task.\n".to_owned()).unwrap(),
            Crew::solo(Member::new(task.clone(), crate::fixtures::normal("claude-2"))).unwrap(),
        ))
        .unwrap();
    let mut solo = state.team_mut(&team).unwrap();
    let (_, mut member) = solo.find_mut(&task).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    if status != TaskStatus::Running {
        member.transition(status).unwrap();
    }
}

#[tokio::test]
async fn a_refused_agent_takes_no_waiting_task_over_before_its_return() {
    let (mut state, _team, task) = waiting_team_state();
    candidate_outside_with_status(&mut state, TaskStatus::Done);
    state.record_refused_until(candidate(), Some(crate::fixtures::at(10_000)));
    let daemon = Daemon::new(&state, None);
    configure_candidate(&daemon, "claude-2");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner
        .hand_over_busy_wait(&task)
        .await
        .expect("the hand-over is decided");

    assert!(!changed, "a refused candidate is not free before its return");
    assert_eq!(
        owner
            .parts
            .state
            .find_task(&task)
            .expect("the task is there")
            .member
            .agent()
            .name(),
        "claude-1"
    );
}

#[tokio::test]
async fn a_refused_agent_takes_the_waiting_task_again_after_its_return() {
    let (mut state, _team, task) = waiting_team_state();
    candidate_outside_with_status(&mut state, TaskStatus::Done);
    state.record_refused_until(candidate(), Some(crate::fixtures::at(1)));
    let daemon = Daemon::new(&state, None);
    configure_candidate(&daemon, "claude-2");
    daemon.clock.advance(time::Duration::seconds(2));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let changed = owner
        .hand_over_busy_wait(&task)
        .await
        .expect("the hand-over is decided");

    assert!(changed, "a candidate whose return has passed is free again");
}
