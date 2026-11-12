use agent_core::state::ItemPhase;
use agent_core::status::{RunOutcome, TaskStatus};

use super::*;
use crate::daemon::team::support::{ALL_DONE, CHAIN, PARALLEL, PLAIN, crew_of, go, ran, reported, solo, task_id};
use crate::daemon::team::{assignable, start_item};

fn held(team: &Team, n: u32) -> bool {
    let (role, member) = team.find(&task_id(n)).unwrap();
    is_held(team, role, member)
}

/// Gives every item an assignee, as the daemon does.
fn assign(team: &mut Team) {
    for given in assignable(team).unwrap() {
        let mut progress = team.item_mut(given.item).unwrap();
        start_item(&mut progress, &given).unwrap();
    }
}

// ---- held members ------------------------------------------------------------------------------------------------

#[test]
fn a_member_without_an_item_that_never_ran_is_held() {
    let team = crew_of(PLAIN, 1, 0);

    assert!(held(&team, 2));
}

#[test]
fn a_member_with_an_item_is_not_held() {
    let mut team = crew_of(CHAIN, 1, 0);
    assign(&mut team);

    assert!(!held(&team, 2));
}

#[test]
fn a_worker_that_never_ran_is_held() {
    let team = crew_of(CHAIN, 1, 1);

    assert!(held(&team, 3));
}

#[test]
fn the_coordinator_and_a_solo_agent_are_never_held() {
    let team = crew_of(PLAIN, 1, 0);
    assert!(!held(&team, 1));

    let alone = solo(PLAIN);
    assert!(!held(&alone, 1));
}

#[test]
fn a_member_that_has_run_or_is_not_queued_is_not_held() {
    let mut team = crew_of(PLAIN, 3, 0);
    ran(
        &mut team,
        2,
        1,
        reported(RunOutcome::Done),
        &[TaskStatus::Waiting, TaskStatus::Queued],
    );
    go(&mut team, 3, &[TaskStatus::Running]);

    assert!(!held(&team, 2), "it has run");
    assert!(!held(&team, 3), "it is running");
    assert!(held(&team, 4));
}

// ---- finished teams ----------------------------------------------------------------------------------------------

#[test]
fn a_team_whose_items_are_pending_is_not_finished() {
    let mut team = crew_of(CHAIN, 1, 0);
    go(&mut team, 1, &[TaskStatus::Running, TaskStatus::Waiting]);
    go(&mut team, 2, &[TaskStatus::Running, TaskStatus::Waiting]);

    assert!(!is_finished(&team));
}

#[test]
fn a_team_whose_items_are_all_done_and_whose_tasks_wait_is_finished() {
    let mut team = crew_of(ALL_DONE, 1, 1);
    go(&mut team, 1, &[TaskStatus::Running, TaskStatus::Waiting]);
    go(&mut team, 2, &[TaskStatus::Running, TaskStatus::Waiting]);

    assert!(is_finished(&team));
}

#[test]
fn a_team_with_a_task_that_is_going_asking_paused_or_about_to_start_is_not_finished() {
    for statuses in [
        vec![TaskStatus::Running],
        vec![TaskStatus::Running, TaskStatus::Questions],
        vec![TaskStatus::Paused],
    ] {
        let mut team = crew_of(ALL_DONE, 1, 0);
        go(&mut team, 1, &[TaskStatus::Running, TaskStatus::Waiting]);
        go(&mut team, 2, &statuses);

        assert!(!is_finished(&team), "{statuses:?}");
    }
    let mut team = crew_of(ALL_DONE, 1, 0);
    go(&mut team, 1, &[TaskStatus::Running, TaskStatus::Waiting]);
    ran(
        &mut team,
        2,
        1,
        reported(RunOutcome::Done),
        &[TaskStatus::Waiting, TaskStatus::Queued],
    );
    assert!(!is_finished(&team), "a task that is queued to run again");
}

#[test]
fn a_held_task_and_a_failed_or_stopped_one_do_not_keep_a_team_open() {
    let mut team = crew_of(ALL_DONE, 3, 1);
    go(&mut team, 1, &[TaskStatus::Running, TaskStatus::Waiting]);
    go(&mut team, 2, &[TaskStatus::Running, TaskStatus::Failed]);
    go(&mut team, 3, &[TaskStatus::Stopped]);
    // Task 4 is a member that never ran and task 5 a worker: both held.

    assert!(is_finished(&team));
}

#[test]
fn a_list_without_items_is_finished_when_its_lead_said_done_and_not_before() {
    let mut team = crew_of(PLAIN, 1, 0);
    go(&mut team, 2, &[TaskStatus::Running, TaskStatus::Waiting]);
    ran(&mut team, 1, 1, reported(RunOutcome::Sleeping), &[TaskStatus::Waiting]);
    assert!(!is_finished(&team), "the lead only slept");

    go(&mut team, 1, &[TaskStatus::Queued]);
    ran(&mut team, 1, 3, reported(RunOutcome::Done), &[TaskStatus::Waiting]);
    assert!(is_finished(&team));
}

#[test]
fn a_solo_task_without_items_is_finished_when_it_is_done() {
    let mut team = solo(PLAIN);
    assert!(!is_finished(&team));
    ran(&mut team, 1, 1, reported(RunOutcome::Done), &[TaskStatus::Done]);

    assert!(is_finished(&team));
}

#[test]
fn a_list_with_pending_items_is_not_finished_however_its_lead_ended() {
    let mut team = solo(PARALLEL);
    ran(&mut team, 1, 1, reported(RunOutcome::Done), &[TaskStatus::Waiting]);

    assert!(!is_finished(&team));
}

#[test]
fn a_team_with_an_item_in_progress_is_not_finished() {
    let mut team = crew_of(PARALLEL, 1, 0);
    assign(&mut team);
    assert!(matches!(
        team.items().first().unwrap().phase(),
        ItemPhase::InProgress { .. }
    ));

    assert!(!is_finished(&team));
}

// ---- closing -----------------------------------------------------------------------------------------------------

#[test]
fn closing_ends_the_waiting_tasks_and_stops_the_held_ones() {
    let mut team = crew_of(ALL_DONE, 2, 1);
    go(&mut team, 1, &[TaskStatus::Running, TaskStatus::Waiting]);
    go(&mut team, 2, &[TaskStatus::Running, TaskStatus::Waiting]);
    go(&mut team, 3, &[TaskStatus::Running, TaskStatus::Done]);
    // Task 4 is the worker, never run: held.

    assert_eq!(
        closing_transitions(&team),
        vec![
            (task_id(1), TaskStatus::Done),
            (task_id(2), TaskStatus::Done),
            (task_id(4), TaskStatus::Stopped)
        ]
    );
}

#[test]
fn closing_a_closed_team_changes_nothing() {
    let mut team = crew_of(ALL_DONE, 1, 0);
    go(&mut team, 1, &[TaskStatus::Running, TaskStatus::Done]);
    go(&mut team, 2, &[TaskStatus::Stopped]);

    assert_eq!(closing_transitions(&team), vec![]);
}

#[test]
fn a_team_that_is_not_finished_has_nothing_to_close() {
    let team = crew_of(CHAIN, 1, 0);

    assert_eq!(closing_transitions(&team), vec![]);
}
