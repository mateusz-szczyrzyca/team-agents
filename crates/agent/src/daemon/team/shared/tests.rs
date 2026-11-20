use agent_core::protocol::SharedAgent;
use agent_core::status::TaskStatus;

use super::*;
use crate::daemon::team::support::{ALL_DONE, CHAIN, PLAIN, crew_of, go, solo, task_id};

#[test]
fn an_agent_whose_task_in_an_open_team_is_not_done_is_shared() {
    let new = solo(PLAIN);
    let mut other = crew_of(CHAIN, 1, 0);
    go(&mut other, 2, &[TaskStatus::Running, TaskStatus::Waiting]);

    let shared = shared_agents(&new, [&other]);

    assert_eq!(
        shared,
        vec![SharedAgent {
            agent: "claude-1".parse().unwrap(),
            team: other.id().clone(),
            task: task_id(2),
        }]
    );
}

#[test]
fn an_agent_whose_task_in_the_other_team_is_done_is_not_shared() {
    let new = solo(PLAIN);
    let mut other = crew_of(CHAIN, 1, 0);
    go(&mut other, 2, &[TaskStatus::Running, TaskStatus::Done]);

    assert_eq!(shared_agents(&new, [&other]), Vec::new());
}

#[test]
fn an_agent_of_a_finished_team_is_not_shared() {
    let new = solo(PLAIN);
    let mut other = crew_of(ALL_DONE, 1, 0);
    go(&mut other, 1, &[TaskStatus::Running, TaskStatus::Waiting]);
    go(&mut other, 2, &[TaskStatus::Running, TaskStatus::Waiting]);

    assert_eq!(shared_agents(&new, [&other]), Vec::new());
}

#[test]
fn an_agent_that_only_the_other_team_has_is_not_shared() {
    let new = solo(PLAIN);
    let other = crew_of(CHAIN, 1, 1);

    let named: Vec<_> = shared_agents(&new, [&other])
        .iter()
        .map(|found| found.agent.as_str().to_owned())
        .collect();

    assert_eq!(named, ["claude-1"]);
}

#[test]
fn every_agent_in_every_other_open_team_is_named_in_the_order_of_the_new_team() {
    let new = crew_of(PLAIN, 1, 0);
    let first = crew_of(CHAIN, 1, 0);
    let second = solo(CHAIN);

    let shared = shared_agents(&new, [&second, &first]);

    let named: Vec<_> = shared
        .iter()
        .map(|found| (found.agent.as_str(), found.task.clone()))
        .collect();
    assert_eq!(
        named,
        vec![
            ("inv-opus", task_id(1)),
            ("claude-1", task_id(2)),
            ("claude-1", task_id(1))
        ]
    );
}
