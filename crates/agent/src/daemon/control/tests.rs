use agent_core::status::StatusMachine;
use agent_core::status::TaskStatus::{self, Done, Failed, Paused, Questions, Queued, Running, Stopped, Waiting};

use super::Halt::{Pause as HaltPause, Stop as HaltStop};
use super::Skipped::{Already, Going, Inactive, NotPaused, Over, Paused as IsPaused, Questions as Asking};
use super::Step::{Halt, Move, Skip};
use super::*;

/// Every status, so that a table that misses one is seen.
const ALL: [TaskStatus; 8] = [Queued, Running, Waiting, Questions, Paused, Stopped, Failed, Done];

fn table(steer: Steer) -> Vec<(TaskStatus, Step)> {
    ALL.iter().map(|status| (*status, decide(steer, *status))).collect()
}

#[test]
fn stop_ends_every_task_that_is_not_over_and_stops_the_run_that_is_going() {
    assert_eq!(
        table(Steer::Stop),
        [
            (Queued, Move(Stopped)),
            (Running, Halt(HaltStop)),
            (Waiting, Move(Stopped)),
            (Questions, Move(Stopped)),
            (Paused, Move(Stopped)),
            (Stopped, Skip(Already)),
            (Failed, Move(Stopped)),
            (Done, Skip(Over)),
        ]
    );
}

#[test]
fn pause_holds_a_task_that_is_going_or_could_go_and_leaves_the_others() {
    assert_eq!(
        table(Steer::Pause),
        [
            (Queued, Move(Paused)),
            (Running, Halt(HaltPause)),
            (Waiting, Move(Paused)),
            (Questions, Skip(Asking)),
            (Paused, Skip(Already)),
            (Stopped, Skip(Inactive)),
            (Failed, Skip(Inactive)),
            (Done, Skip(Over)),
        ]
    );
}

#[test]
fn unpause_queues_a_paused_task_and_nothing_else() {
    assert_eq!(
        table(Steer::Unpause),
        [
            (Queued, Skip(NotPaused)),
            (Running, Skip(NotPaused)),
            (Waiting, Skip(NotPaused)),
            (Questions, Skip(NotPaused)),
            (Paused, Move(Queued)),
            (Stopped, Skip(NotPaused)),
            (Failed, Skip(NotPaused)),
            (Done, Skip(Over)),
        ]
    );
}

#[test]
fn resume_queues_a_stopped_failed_or_waiting_task_and_points_the_others_to_their_own_command() {
    assert_eq!(
        table(Steer::Resume),
        [
            (Queued, Skip(Going)),
            (Running, Skip(Going)),
            (Waiting, Move(Queued)),
            (Questions, Skip(Asking)),
            (Paused, Skip(IsPaused)),
            (Stopped, Move(Queued)),
            (Failed, Move(Queued)),
            (Done, Skip(Over)),
        ]
    );
}

#[test]
fn every_move_is_a_change_the_status_table_allows() {
    for steer in [Steer::Stop, Steer::Pause, Steer::Unpause, Steer::Resume] {
        for (from, step) in table(steer) {
            if let Move(to) = step {
                assert!(from.successors().contains(&to), "{steer:?}: {from} to {to}");
            }
        }
    }
}

#[test]
fn a_control_is_the_action_of_its_name() {
    let actions: Vec<ControlAction> = [Steer::Stop, Steer::Pause, Steer::Unpause, Steer::Resume]
        .into_iter()
        .map(ControlAction::from)
        .collect();

    assert_eq!(
        actions,
        [
            ControlAction::Stop,
            ControlAction::Pause,
            ControlAction::Unpause,
            ControlAction::Resume
        ]
    );
}

#[test]
fn a_run_that_ended_by_itself_while_a_stop_was_asked_leaves_the_task_stopped_where_the_table_allows() {
    let after: Vec<_> = ALL.iter().map(|status| after_halt(HaltStop, *status)).collect();

    // A task that is over, failed or asking keeps what the run made of it.
    assert_eq!(
        after,
        [Stopped, Running, Stopped, Questions, Stopped, Stopped, Failed, Done]
    );
}

#[test]
fn a_run_that_ended_by_itself_while_a_pause_was_asked_leaves_the_task_paused_where_the_table_allows() {
    let after: Vec<_> = ALL.iter().map(|status| after_halt(HaltPause, *status)).collect();

    assert_eq!(
        after,
        [Paused, Running, Paused, Questions, Paused, Stopped, Failed, Done]
    );
}
