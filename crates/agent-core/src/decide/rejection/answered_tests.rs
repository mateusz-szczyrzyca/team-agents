//! A lead that repeats its `RETURNED:` line before the member ran again sends back the same work once: the second line
//! is no new rejection.

use super::*;
use crate::status::RunOutcome;

/// The facts of a hand-over after the third run, which is still the member's last.
fn handed_over() -> Facts {
    let third = NonZeroU32::new(3).expect("3 is not zero");
    Facts::new(
        Held::ByTheMember,
        LastRun::Reported(RunOutcome::Done),
        MovedOn::No,
        third,
        third,
    )
}

/// What a `RETURNED: 1.1` counts with `facts`.
fn counted(facts: Facts) -> Option<Rejection> {
    classify(&ReturnLine::returned(&"1.1".parse().expect("1.1 is an item")), facts)
}

#[test]
fn a_return_of_work_that_was_sent_back_already_counts_once() {
    assert_eq!(counted(handed_over()), Some(Rejection::Counted));
    assert_eq!(
        counted(handed_over().with_answered(Answered::Yes)),
        Some(Rejection::AlreadyCounted)
    );
}

#[test]
fn a_return_with_no_work_handed_over_is_not_taken_for_a_repeat() {
    let facts = handed_over()
        .with_last_run(LastRun::Reported(RunOutcome::Questions))
        .with_answered(Answered::Yes);

    assert_eq!(counted(facts), Some(Rejection::NotHandedOver));
}
