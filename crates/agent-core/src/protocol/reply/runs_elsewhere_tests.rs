//! The runs a queued task waits for: which members are marked, and how the mark reaches the wire.

use serde_json::json;

use super::{MemberSummary, RunElsewhere, mark_runs_elsewhere};
use crate::config::Limits;
use crate::decide::slot::Going;
use crate::ids::{AgentName, TaskId, TeamId};
use crate::kinds::Role;
use crate::state::Participant;
use crate::status::TaskStatus;

/// The task `name` as a task ID.
fn task(name: &str) -> TaskId {
    name.parse().expect("a valid task ID")
}

/// A member of `name` that has `status` in its task `task`.
fn member(agent: &str, task_name: &str, status: TaskStatus) -> MemberSummary {
    let agent: AgentName = agent.parse().expect("a valid agent name");
    MemberSummary {
        task: task(task_name),
        agent: Participant::Normal(agent),
        role: Role::Member,
        status,
        open_question: None,
        new_sessions: None,
        held_by: None,
        returned: None,
        waits_for: None,
    }
}

/// A run of `agent` for `task_name` in the team `team`.
fn going(agent: &str, team: &str, task_name: &str) -> Going {
    Going {
        agent: agent.parse().expect("a valid agent name"),
        team: team.parse().expect("a valid team ID"),
        task: task(task_name),
    }
}

const TEAM_A: &str = "team-20261122-140915-aaaa";
const TEAM_B: &str = "team-20261122-140915-bbbb";
const TASK_A: &str = "task-20261122-140915-a1a1";
const TASK_B: &str = "task-20261122-140915-b1b1";
const TASK_C: &str = "task-20261122-140915-c1c1";

#[test]
fn a_queued_task_whose_agent_runs_elsewhere_waits_for_that_run() {
    let mut members = vec![member("codex-1", TASK_C, TaskStatus::Queued)];
    let running = [going("codex-1", TEAM_A, TASK_A)];
    mark_runs_elsewhere(&mut members, &running, &Limits::default());
    assert_eq!(
        members[0].waits_for,
        Some(RunElsewhere {
            team: TEAM_A.parse().expect("a valid team ID"),
            task: task(TASK_A),
        })
    );
}

#[test]
fn a_task_that_runs_or_whose_agent_is_free_waits_for_nothing() {
    let mut members = vec![
        member("codex-1", TASK_B, TaskStatus::Running),
        member("gemini-1", TASK_C, TaskStatus::Queued),
    ];
    let running = [going("codex-1", TEAM_A, TASK_A)];
    mark_runs_elsewhere(&mut members, &running, &Limits::default());
    assert_eq!(members[0].waits_for, None);
    assert_eq!(members[1].waits_for, None);
}

#[test]
fn no_limit_means_a_queued_task_never_waits() {
    let mut members = vec![member("codex-1", TASK_C, TaskStatus::Queued)];
    let running = [going("codex-1", TEAM_A, TASK_A)];
    let limits = Limits {
        runs_per_agent: 0,
        ..Limits::default()
    };
    mark_runs_elsewhere(&mut members, &running, &limits);
    assert_eq!(members[0].waits_for, None);
}

#[test]
fn a_limit_of_two_lets_a_queued_task_wait_only_when_two_runs_go() {
    let limits = Limits {
        runs_per_agent: 2,
        ..Limits::default()
    };
    let mut one = vec![member("codex-1", TASK_C, TaskStatus::Queued)];
    mark_runs_elsewhere(&mut one, &[going("codex-1", TEAM_A, TASK_A)], &limits);
    assert_eq!(one[0].waits_for, None);
    let two = [going("codex-1", TEAM_A, TASK_A), going("codex-1", TEAM_B, TASK_B)];
    let mut members = vec![member("codex-1", TASK_C, TaskStatus::Queued)];
    mark_runs_elsewhere(&mut members, &two, &limits);
    assert_eq!(
        members[0].waits_for.as_ref().map(|wait| wait.task.clone()),
        Some(task(TASK_A))
    );
}

#[test]
fn a_member_without_a_mark_has_no_waits_for_on_the_wire() {
    let plain = member("codex-1", TASK_C, TaskStatus::Queued);
    let value = serde_json::to_value(&plain).expect("a member is JSON");
    assert!(value.get("waits_for").is_none(), "{value}");
    let mut marked = plain;
    marked.waits_for = Some(RunElsewhere {
        team: TEAM_A.parse().expect("a valid team ID"),
        task: task(TASK_A),
    });
    let value = serde_json::to_value(&marked).expect("a member is JSON");
    assert_eq!(value["waits_for"], json!({"team": TEAM_A, "task": TASK_A}), "{value}");
}
