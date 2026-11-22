//! The facts of an item as its member's runs show them: how the last run ended, which run is the last, and which run the
//! hand-over was made after.

use super::*;
use crate::state::RunEnd;
use crate::state::fixtures::{at, member, normal, run};
use crate::status::RunOutcome;

/// How a run of these tests ends, or that it is still going.
#[derive(Clone)]
enum Run {
    Ended(RunEnd),
    Going,
}

/// The facts of an item held by the member of these tests, whose runs are `runs`, in order.
fn facts_of(runs: &[Run]) -> Facts {
    let mut task = member(1, normal("claude-1"));
    for (index, how) in runs.iter().enumerate() {
        let number = u32::try_from(index).unwrap() * 2 + 1;
        task.begin_run(run(number), at(number), None).unwrap();
        if let Run::Ended(end) = how {
            task.end_run(at(number + 1), end.clone()).unwrap();
        }
    }
    Facts::from_runs(Held::ByTheMember, Answered::No, task.runs())
}

/// What the facts of an item whose runs are `runs` are expected to be.
fn expected(last_run: LastRun, handed_over_after: u32, last_run_number: u32) -> Facts {
    Facts::new(
        Held::ByTheMember,
        last_run,
        MovedOn::No,
        NonZeroU32::new(handed_over_after).unwrap(),
        NonZeroU32::new(last_run_number).unwrap(),
    )
}

/// A run that ended with `outcome`.
fn reported(outcome: RunOutcome) -> Run {
    Run::Ended(RunEnd::Reported { outcome })
}

#[test]
fn the_facts_follow_the_end_and_the_number_of_the_members_runs() {
    let table = [
        ("no run yet", vec![], expected(LastRun::Never, 1, 1)),
        (
            "one run that ended DONE",
            vec![reported(RunOutcome::Done)],
            expected(LastRun::Reported(RunOutcome::Done), 1, 1),
        ),
        (
            "two runs that ended DONE: the hand-over is after the second",
            vec![reported(RunOutcome::Done), reported(RunOutcome::Done)],
            expected(LastRun::Reported(RunOutcome::Done), 2, 2),
        ),
        (
            "a run that asked after one that ended DONE",
            vec![reported(RunOutcome::Done), reported(RunOutcome::Questions)],
            expected(LastRun::Reported(RunOutcome::Questions), 1, 2),
        ),
        (
            "a run without a status",
            vec![
                reported(RunOutcome::Done),
                Run::Ended(RunEnd::NoStatus { exit_code: Some(0) }),
            ],
            expected(LastRun::NoStatus, 1, 2),
        ),
        (
            "a run the human stopped",
            vec![reported(RunOutcome::Done), Run::Ended(RunEnd::Stopped)],
            expected(LastRun::Stopped, 1, 2),
        ),
        (
            "a run that hung",
            vec![reported(RunOutcome::Done), Run::Ended(RunEnd::Hung)],
            expected(LastRun::NoStatus, 1, 2),
        ),
        (
            "a run that is going",
            vec![reported(RunOutcome::Done), Run::Going],
            expected(LastRun::Running, 1, 2),
        ),
    ];
    for (what, runs, facts) in table {
        assert_eq!(facts_of(&runs), facts, "{what}");
    }
}

#[test]
fn the_facts_keep_who_holds_the_item_and_whether_it_was_sent_back_already() {
    let task = member(1, normal("claude-1"));

    let facts = Facts::from_runs(Held::Nobody, Answered::Yes, task.runs());

    assert_eq!(
        facts,
        expected(LastRun::Never, 1, 1)
            .with_held(Held::Nobody)
            .with_answered(Answered::Yes)
    );
}
