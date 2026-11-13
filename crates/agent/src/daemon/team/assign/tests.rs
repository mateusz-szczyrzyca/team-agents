use agent_core::ids::{AgentName, ItemId, TaskId};
use agent_core::state::{ItemPhase, Team};
use agent_core::status::TaskStatus;

use super::*;
use crate::daemon::team::support::{CHAIN, PARALLEL, crew_of, go, solo, task_id};

fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

fn item(text: &str) -> ItemId {
    text.parse().unwrap()
}

fn assigned(team: &Team) -> Vec<(String, String)> {
    assignable(team)
        .unwrap()
        .into_iter()
        .map(|a| (a.item.to_string(), a.agent.to_string()))
        .collect()
}

/// Applies the assignments of `team`, as the daemon does.
fn apply(team: &mut Team) {
    for given in assignable(team).unwrap() {
        let mut progress = team.item_mut(given.item).unwrap();
        start_item(&mut progress, &given).unwrap();
    }
}

#[test]
fn the_first_item_goes_to_the_first_member_with_its_own_branch() {
    let team = crew_of(CHAIN, 1, 0);

    let given = assignable(&team).unwrap();

    assert_eq!(
        given,
        vec![Assigned {
            item: item("1.1"),
            task: task_id(2),
            agent: agent("claude-1"),
            branch: "agents/claude-1/task-20261111-153002-k3x9-1.1".parse().unwrap(),
        }]
    );
}

#[test]
fn a_solo_agent_takes_the_items_itself() {
    let team = solo(CHAIN);

    assert_eq!(assigned(&team), [("1.1".to_owned(), "claude-1".to_owned())]);
}

#[test]
fn items_of_features_that_do_not_wait_go_to_the_members_in_order() {
    let team = crew_of(PARALLEL, 2, 0);

    assert_eq!(
        assigned(&team),
        [
            ("1.1".to_owned(), "claude-1".to_owned()),
            ("2.1".to_owned(), "claude-2".to_owned())
        ]
    );
}

#[test]
fn a_member_that_has_an_item_is_not_given_another() {
    let mut team = crew_of(PARALLEL, 1, 0);
    apply(&mut team);

    assert_eq!(assigned(&team), []);
}

#[test]
fn the_next_item_of_a_feature_waits_for_the_one_before() {
    let mut team = crew_of(CHAIN, 2, 0);
    apply(&mut team);

    assert_eq!(assigned(&team), [], "1.2 starts when 1.1 is merged");
}

#[test]
fn a_waiting_member_is_given_the_next_item() {
    let mut team = crew_of(PARALLEL, 2, 0);
    go(&mut team, 2, &[TaskStatus::Running, TaskStatus::Waiting]);
    go(&mut team, 3, &[TaskStatus::Running]);

    assert_eq!(assigned(&team), [("1.1".to_owned(), "claude-1".to_owned())]);
}

#[test]
fn a_member_that_is_running_is_not_given_an_item() {
    let mut team = crew_of(PARALLEL, 2, 0);
    go(&mut team, 2, &[TaskStatus::Running]);

    assert_eq!(assigned(&team), [("1.1".to_owned(), "claude-2".to_owned())]);
}

#[test]
fn members_that_cannot_work_are_not_given_items() {
    let mut team = crew_of(PARALLEL, 3, 0);
    go(&mut team, 2, &[TaskStatus::Paused]);
    go(&mut team, 3, &[TaskStatus::Stopped]);
    go(&mut team, 4, &[TaskStatus::Running, TaskStatus::Failed]);

    assert_eq!(assigned(&team), []);
}

#[test]
fn the_coordinator_and_the_workers_take_no_items() {
    let mut team = crew_of(PARALLEL, 1, 2);
    go(&mut team, 2, &[TaskStatus::Running]);

    assert_eq!(assigned(&team), []);
}

#[test]
fn a_list_without_items_assigns_nothing() {
    let team = crew_of(crate::daemon::team::support::PLAIN, 2, 0);

    assert_eq!(assigned(&team), []);
}

#[test]
fn an_item_branch_and_a_task_branch_are_named_by_agent_and_task() {
    let task: TaskId = "task-20261112-101500-ab12".parse().unwrap();

    assert_eq!(
        item_branch(&agent("codex-1"), &task, item("2.3")).unwrap().as_str(),
        "agents/codex-1/task-20261112-101500-ab12-2.3"
    );
    assert_eq!(
        task_branch(&agent("codex-1"), &task).unwrap().as_str(),
        "agents/codex-1/task-20261112-101500-ab12"
    );
}

#[test]
fn starting_an_item_records_the_member_and_the_branch() {
    let mut team = crew_of(CHAIN, 1, 0);
    let given = assignable(&team).unwrap().remove(0);

    let mut progress = team.item_mut(given.item).unwrap();
    start_item(&mut progress, &given).unwrap();

    let phase = team.item(item("1.1")).unwrap().phase();
    assert_eq!(
        phase,
        &ItemPhase::InProgress {
            assignee: agent("claude-1"),
            branch: given.branch.clone(),
        }
    );
}

#[test]
fn an_item_that_is_not_pending_cannot_be_started_again() {
    let mut team = crew_of(CHAIN, 1, 0);
    let given = assignable(&team).unwrap().remove(0);
    let mut progress = team.item_mut(given.item).unwrap();
    start_item(&mut progress, &given).unwrap();

    let mut progress = team.item_mut(given.item).unwrap();
    let again = start_item(&mut progress, &given);

    assert!(matches!(again, Err(AdvanceError::Transition(_))), "{again:?}");
}
