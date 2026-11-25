//! The backlog of the reminders of Serena: what is tried again and what is told once.

use std::str::FromStr;

use agent_core::ids::{RunId, TaskId};
use agent_core::kinds::CliKind;
use agent_runtime::run::serena::DueReminder;

use super::{Backlog, Failed, MAX_ATTEMPTS};

/// The reminder of the run `run` of the fixture's task.
fn due(run: &str) -> DueReminder {
    DueReminder {
        task: TaskId::from_str("task-20261124-100000-t1t1").expect("the task"),
        run: RunId::from_str(run).expect("the run"),
        cli: CliKind::Claude,
        searches: 40,
    }
}

const FIRST: &str = "run-20261124-100000-r1r1";
const SECOND: &str = "run-20261124-100100-r2r2";

#[test]
fn a_reminder_offered_is_tried_once_and_leaves_the_backlog() {
    let mut backlog = Backlog::default();
    backlog.offer(due(FIRST));

    let attempts = backlog.take();

    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts.first().map(|attempt| attempt.due.clone()), Some(due(FIRST)));
    assert_eq!(attempts.first().map(|attempt| attempt.failures), Some(0));
    assert_eq!(backlog.take(), Vec::new());
}

#[test]
fn a_reminder_that_failed_is_tried_again_at_the_next_turn() {
    let mut backlog = Backlog::default();
    backlog.offer(due(FIRST));
    let attempts = backlog.take();
    let attempt = attempts.into_iter().next().expect("one attempt");

    let outcome = backlog.failed(attempt);

    assert_eq!(outcome, Failed::Retry);
    let again = backlog.take();
    assert_eq!(again.len(), 1);
    assert_eq!(again.first().map(|attempt| attempt.failures), Some(1));
}

#[test]
fn a_reminder_that_failed_every_time_is_given_up_after_the_bound() {
    let mut backlog = Backlog::default();
    backlog.offer(due(FIRST));
    let mut outcomes = Vec::new();
    for _ in 0..MAX_ATTEMPTS {
        let attempt = backlog.take().into_iter().next().expect("an attempt");
        outcomes.push(backlog.failed(attempt));
    }

    let mut expected = vec![Failed::Retry; usize::try_from(MAX_ATTEMPTS).expect("a small bound") - 1];
    expected.push(Failed::GaveUp);
    assert_eq!(outcomes, expected);
    assert_eq!(backlog.take(), Vec::new());
}

#[test]
fn a_run_whose_reminder_the_bus_took_is_not_offered_again() {
    let mut backlog = Backlog::default();
    backlog.offer(due(FIRST));
    let attempt = backlog.take().into_iter().next().expect("an attempt");
    backlog.delivered(&attempt);

    backlog.offer(due(FIRST));

    assert_eq!(backlog.take(), Vec::new());
}

#[test]
fn a_run_whose_reminder_was_given_up_is_offered_again_by_the_end_of_the_run() {
    let mut backlog = Backlog::default();
    backlog.offer(due(FIRST));
    for _ in 0..MAX_ATTEMPTS {
        let attempt = backlog.take().into_iter().next().expect("an attempt");
        backlog.failed(attempt);
    }

    backlog.offer(due(FIRST));

    assert_eq!(backlog.take().len(), 1);
}

#[test]
fn a_reminder_that_waits_is_not_added_twice_and_another_run_is() {
    let mut backlog = Backlog::default();
    backlog.offer(due(FIRST));
    backlog.offer(due(FIRST));
    backlog.offer(due(SECOND));

    let runs: Vec<String> = backlog
        .take()
        .into_iter()
        .map(|attempt| attempt.due.run.as_str().to_owned())
        .collect();

    assert_eq!(runs, [FIRST, SECOND]);
}
