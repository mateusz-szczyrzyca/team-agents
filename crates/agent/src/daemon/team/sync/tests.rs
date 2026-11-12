use agent_core::state::Member;
use agent_core::status::{RunOutcome, TaskStatus};
use serde_json::json;

use super::*;
use crate::daemon::team::support::{CHAIN, PLAIN, crew_of, go, ran, reported, task_id};
use crate::daemon::team::{assignable, start_item};

fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

fn desired(team: &Team, n: u32) -> MemberState {
    let (_, member) = team.find(&task_id(n)).unwrap();
    desired_bus_state(member)
}

/// The bus team of a crew of the coordinator `inv-opus`, a member `claude-1` and a worker `glm-1`, with the states
/// given for the three.
fn bus(coordinator: &str, member: &str, worker: &str) -> BusTeam {
    serde_json::from_value(json!({
        "id": "team-20261111-153000-t5m8",
        "repo": "example",
        "lead": "inv-opus",
        "done": false,
        "members": {
            "inv-opus": {"task": task_id(1).as_str(), "branch": "agents/a", "worktree": "/w/1", "state": coordinator, "tier": "normal"},
            "claude-1": {"task": task_id(2).as_str(), "branch": "agents/b", "worktree": "/w/2", "state": member, "tier": "normal"},
            "glm-1": {"task": task_id(3).as_str(), "branch": "agents/c", "worktree": "/w/3", "state": worker, "tier": "worker"},
        },
        "board": null,
    }))
    .unwrap()
}

fn assign(team: &mut Team) {
    for given in assignable(team).unwrap() {
        let mut progress = team.item_mut(given.item).unwrap();
        start_item(&mut progress, &given).unwrap();
    }
}

fn status_of(member: &Member) -> TaskStatus {
    member.status()
}

// ---- the state each status asks for ------------------------------------------------------------------------------

#[test]
fn a_task_that_runs_or_is_about_to_is_active_on_the_bus() {
    let mut team = crew_of(CHAIN, 1, 0);
    assign(&mut team);
    assert_eq!(desired(&team, 1), MemberState::Active, "the coordinator, queued");
    assert_eq!(desired(&team, 2), MemberState::Active, "a member with an item, queued");
    go(&mut team, 2, &[TaskStatus::Running]);

    assert_eq!(desired(&team, 2), MemberState::Active);
}

#[test]
fn a_task_that_waits_sleeps_on_the_bus() {
    let mut team = crew_of(PLAIN, 2, 1);
    go(&mut team, 2, &[TaskStatus::Running, TaskStatus::Waiting]);
    go(&mut team, 4, &[TaskStatus::Running, TaskStatus::Waiting]);

    assert_eq!(desired(&team, 2), MemberState::Sleeping, "a member");
    assert_eq!(desired(&team, 4), MemberState::Sleeping, "a worker");
}

#[test]
fn a_task_that_asks_or_is_paused_is_paused_on_the_bus_so_that_messages_do_not_wake_it() {
    let mut team = crew_of(PLAIN, 2, 0);
    go(&mut team, 2, &[TaskStatus::Running, TaskStatus::Questions]);
    go(&mut team, 3, &[TaskStatus::Paused]);

    assert_eq!(desired(&team, 2), MemberState::Paused);
    assert_eq!(desired(&team, 3), MemberState::Paused);
}

#[test]
fn a_stopped_task_is_stopped_and_a_done_or_failed_one_is_done() {
    let mut team = crew_of(PLAIN, 3, 0);
    go(&mut team, 2, &[TaskStatus::Stopped]);
    go(&mut team, 3, &[TaskStatus::Running, TaskStatus::Done]);
    go(&mut team, 4, &[TaskStatus::Running, TaskStatus::Failed]);

    assert_eq!(desired(&team, 2), MemberState::Stopped);
    assert_eq!(desired(&team, 3), MemberState::Done);
    assert_eq!(desired(&team, 4), MemberState::Done);
}

#[test]
fn a_member_that_was_excluded_is_excluded_on_the_bus() {
    let mut team = crew_of(PLAIN, 1, 0);
    ran(&mut team, 2, 1, reported(RunOutcome::Excluded), &[TaskStatus::Done]);

    assert_eq!(desired(&team, 2), MemberState::Excluded);
    assert_eq!(status_of(team.find(&task_id(2)).unwrap().1), TaskStatus::Done);
}

// ---- what differs ------------------------------------------------------------------------------------------------

#[test]
fn a_bus_that_agrees_needs_nothing() {
    let mut team = crew_of(PLAIN, 1, 1);
    go(&mut team, 1, &[TaskStatus::Running]);
    go(&mut team, 2, &[TaskStatus::Running, TaskStatus::Waiting]);
    go(&mut team, 3, &[TaskStatus::Running, TaskStatus::Waiting]);

    let plan = reconcile(&team, &bus("active", "sleeping", "sleeping")).unwrap();

    assert_eq!(plan, Reconciliation::default());
}

#[test]
fn a_member_that_waits_while_the_bus_says_active_is_put_to_sleep() {
    let mut team = crew_of(PLAIN, 1, 0);
    go(&mut team, 1, &[TaskStatus::Running]);
    go(&mut team, 2, &[TaskStatus::Running, TaskStatus::Waiting]);

    let plan = reconcile(&team, &bus("active", "active", "sleeping")).unwrap();

    assert_eq!(
        plan,
        Reconciliation {
            set: vec![(agent("claude-1"), MemberState::Sleeping)],
            wake: vec![],
        }
    );
}

#[test]
fn a_member_that_runs_while_the_bus_says_sleeping_is_made_active() {
    let mut team = crew_of(PLAIN, 1, 0);
    go(&mut team, 1, &[TaskStatus::Running]);
    go(&mut team, 2, &[TaskStatus::Running]);

    let plan = reconcile(&team, &bus("active", "sleeping", "sleeping")).unwrap();

    assert_eq!(plan.set, vec![(agent("claude-1"), MemberState::Active)]);
}

#[test]
fn a_waiting_member_the_bus_woke_is_woken_and_made_active() {
    let mut team = crew_of(PLAIN, 1, 0);
    go(&mut team, 1, &[TaskStatus::Running]);
    go(&mut team, 2, &[TaskStatus::Running, TaskStatus::Waiting]);

    let plan = reconcile(&team, &bus("active", "waking", "sleeping")).unwrap();

    assert_eq!(
        plan,
        Reconciliation {
            set: vec![(agent("claude-1"), MemberState::Active)],
            wake: vec![task_id(2)],
        }
    );
}

#[test]
fn a_member_that_is_running_while_the_bus_says_waking_is_not_woken_twice() {
    let mut team = crew_of(PLAIN, 1, 0);
    go(&mut team, 1, &[TaskStatus::Running]);
    go(&mut team, 2, &[TaskStatus::Running]);

    let plan = reconcile(&team, &bus("active", "waking", "sleeping")).unwrap();

    assert_eq!(plan.wake, vec![]);
    assert_eq!(plan.set, vec![(agent("claude-1"), MemberState::Active)]);
}

#[test]
fn a_member_the_bus_excluded_stays_as_the_bus_has_it() {
    let mut team = crew_of(PLAIN, 1, 0);
    go(&mut team, 1, &[TaskStatus::Running]);
    go(&mut team, 2, &[TaskStatus::Running, TaskStatus::Waiting]);

    let plan = reconcile(&team, &bus("active", "excluded", "sleeping")).unwrap();

    assert_eq!(plan, Reconciliation::default());
}

#[test]
fn a_member_the_bus_does_not_know_is_left_out() {
    let mut team = crew_of(PLAIN, 1, 0);
    go(&mut team, 1, &[TaskStatus::Running]);
    let bus: BusTeam = serde_json::from_value(json!({
        "id": "team-20261111-153000-t5m8", "repo": "example", "lead": "inv-opus", "done": false,
        "members": {"inv-opus": {"task": task_id(1).as_str(), "branch": "agents/a", "worktree": "/w/1", "state": "active", "tier": "normal"}},
    }))
    .unwrap();

    let plan = reconcile(&team, &bus).unwrap();

    assert_eq!(plan, Reconciliation::default());
}
