//! The decision of what a lead's `RETURNED:` line counts: a table over the lead's end, who holds the item, the end of
//! the member's last run, and whether the item moved on since the run it was handed over after.

use super::*;
use crate::status::RunOutcome;

/// The member's last run ended `DONE`, and the item was handed over after the third run, which is still its last: the
/// facts of a return of work the lead read.
fn handed_over() -> Facts {
    Facts::new(
        Held::ByTheMember,
        LastRun::Reported(RunOutcome::Done),
        MovedOn::No,
        NonZeroU32::new(3).expect("3 is not zero"),
        NonZeroU32::new(3).expect("3 is not zero"),
    )
}

/// What a `RETURNED: 1.1` counts with `facts`.
fn counted(facts: Facts) -> Option<Rejection> {
    classify(&ReturnLine::returned(&"1.1".parse().expect("1.1 is an item")), facts)
}

#[test]
fn a_return_of_handed_over_work_is_a_rejection() {
    assert_eq!(
        counted(handed_over().with_lead(LeadEnd::Settled(RunOutcome::Sleeping))),
        Some(Rejection::Counted)
    );
    // `DONE` beside the line is the lead taking the work and returning only the other items it read.
    assert_eq!(
        counted(handed_over().with_lead(LeadEnd::Settled(RunOutcome::Done))),
        Some(Rejection::Counted)
    );
}

#[test]
fn a_return_with_no_handed_over_work_behind_it_is_no_rejection() {
    let table = [
        // The item is nobody's to hand over: pending, under review, approved, done.
        (handed_over().with_held(Held::Nobody), "the item left its phase"),
        // The member has not run on it yet.
        (handed_over().with_last_run(LastRun::Never), "the member never ran"),
        // The member's last run ended with something but `DONE`.
        (
            handed_over().with_last_run(LastRun::Reported(RunOutcome::Failed)),
            "the member failed",
        ),
        (
            handed_over().with_last_run(LastRun::Reported(RunOutcome::Questions)),
            "the member asked",
        ),
        (
            handed_over().with_last_run(LastRun::NoStatus),
            "the member said nothing",
        ),
        (handed_over().with_last_run(LastRun::Stopped), "the human stopped it"),
    ];
    assert_eq!(
        counted(handed_over()),
        Some(Rejection::Counted),
        "the facts of a hand-over first"
    );
    for (facts, reason) in table {
        assert_eq!(counted(facts), Some(Rejection::NotHandedOver), "{reason}");
    }
}

#[test]
fn a_return_of_work_the_item_moved_on_from_is_no_rejection() {
    let table = [
        // The item left its phase after the run it was handed over after.
        (handed_over().moved_on(), "the item moved on"),
        // The member was started again before the lead's return.
        (
            handed_over().with_last_run_of(NonZeroU32::new(4).expect("4 is not zero")),
            "the member was started again",
        ),
    ];
    for (facts, reason) in table {
        assert_eq!(counted(facts), Some(Rejection::NotHandedOver), "{reason}");
    }
}

#[test]
fn a_lead_that_ended_with_questions_failed_or_unsettled_counts_no_rejection() {
    let table = [
        LeadEnd::Settled(RunOutcome::Questions),
        LeadEnd::Settled(RunOutcome::Failed),
        LeadEnd::Settled(RunOutcome::Excluded),
        LeadEnd::Unsettled,
    ];
    for end in table {
        assert_eq!(counted(handed_over().with_lead(end)), None, "{end:?}");
    }
}

#[test]
fn a_message_that_names_the_item_only_in_prose_counts_nothing() {
    let prose = "The work of 1.1 is not accepted.\n\nSTATUS: SLEEPING\n";
    let facts = handed_over().with_lead(LeadEnd::Settled(RunOutcome::Sleeping));

    assert_eq!(returned_items(prose), Vec::new());
    assert!(
        returned_items(prose)
            .into_iter()
            .all(|item| classify(&ReturnLine::returned(&item), facts).is_none())
    );
}
