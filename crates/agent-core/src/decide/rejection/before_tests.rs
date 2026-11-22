//! A lead sends the work back by telling the member on the bus, which starts the member again while the lead's run is
//! still going. The facts of the item are those of the runs that started before the lead's run, and the runs after it
//! are what shows the member was started again.

use super::*;
use crate::state::RunEnd;
use crate::state::fixtures::{at, member, normal, run};
use crate::status::RunOutcome;

/// The runs of a member whose run `n` starts at second `10 * n` and ends `DONE` a second later, except the last of the
/// `count`, which is still going when `going` says so.
fn runs(count: u32, going: bool) -> Vec<crate::state::Run> {
    let mut task = member(1, normal("claude-1"));
    for n in 1..=count {
        task.begin_run(run(n), at(10 * n), None).unwrap();
        if !(going && n == count) {
            task.end_run(
                at(10 * n + 1),
                RunEnd::Reported {
                    outcome: RunOutcome::Done,
                },
            )
            .unwrap();
        }
    }
    task.runs().to_vec()
}

/// The facts the member's first `ended` runs give, the last of them its last, the hand-over after it.
fn after(ended: u32) -> Facts {
    let number = NonZeroU32::new(ended).unwrap();
    Facts::new(
        Held::ByTheMember,
        LastRun::Reported(RunOutcome::Done),
        MovedOn::No,
        number,
        number,
    )
}

#[test]
fn the_facts_leave_out_the_runs_that_started_after_the_lead_began() {
    let table = [
        // The lead began at second 15, between the first run and the second one that its message started.
        (runs(2, true), 15, after(1), "the run the return started is left out"),
        // The lead began after both runs: the second one is the work it read.
        (
            runs(2, false),
            25,
            after(2),
            "a run before the lead is the work it read",
        ),
        // The lead began at the second the run started: that run is not before it.
        (
            runs(2, false),
            20,
            after(1),
            "a run that starts with the lead is not before it",
        ),
    ];
    for (runs, lead_started, expected, what) in table {
        assert_eq!(
            Facts::from_runs_before(Held::ByTheMember, Answered::No, &runs, at(lead_started)),
            expected,
            "{what}"
        );
    }
}

#[test]
fn the_member_was_started_again_when_a_run_began_at_or_after_the_lead() {
    let table = [
        (runs(2, true), 15, Some(2), "the run the return started"),
        (runs(3, false), 15, Some(3), "the last of the runs after the lead began"),
        (runs(2, false), 25, None, "both runs are before the lead"),
        (
            runs(2, false),
            20,
            Some(2),
            "a run that starts with the lead is since it",
        ),
        (runs(0, false), 5, None, "no run at all"),
    ];
    for (runs, lead_started, expected, what) in table {
        assert_eq!(
            run_started_since(&runs, at(lead_started)),
            expected.and_then(NonZeroU32::new),
            "{what}"
        );
    }
}
