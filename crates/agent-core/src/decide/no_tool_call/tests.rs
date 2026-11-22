use crate::state::RunEnd;

use super::{AfterFailedWithoutTool, after_failed_without_tool};

/// The end of a run that reported `STATUS: FAILED`.
fn failed() -> RunEnd {
    RunEnd::Reported {
        outcome: crate::status::RunOutcome::Failed,
    }
}

/// The decision table of a run that ended `STATUS: FAILED`: every combination of used-a-tool, first-of-session and
/// already-retried, and what the run becomes.
#[test]
fn the_decision_table_of_a_failed_run() {
    let cases = [
        // used_a_tool, first_of_session, already_retried, expected
        (false, true, false, AfterFailedWithoutTool::Retry),
        (false, true, true, AfterFailedWithoutTool::Fail),
        (false, false, false, AfterFailedWithoutTool::Fail),
        (false, false, true, AfterFailedWithoutTool::Fail),
        (true, true, false, AfterFailedWithoutTool::Fail),
        (true, true, true, AfterFailedWithoutTool::Fail),
        (true, false, false, AfterFailedWithoutTool::Fail),
        (true, false, true, AfterFailedWithoutTool::Fail),
    ];
    for (used_a_tool, first_of_session, already_retried, expected) in cases {
        assert_eq!(
            after_failed_without_tool(&failed(), used_a_tool, first_of_session, already_retried),
            expected,
            "used_a_tool {used_a_tool}, first_of_session {first_of_session}, already_retried {already_retried}"
        );
    }
}

/// An end that is not `STATUS: FAILED` is never retried, whatever the three flags say.
#[test]
fn an_end_that_did_not_report_failed_is_never_retried() {
    for end in [
        RunEnd::Reported {
            outcome: crate::status::RunOutcome::Done,
        },
        RunEnd::Reported {
            outcome: crate::status::RunOutcome::Sleeping,
        },
        RunEnd::NoStatus { exit_code: Some(1) },
        RunEnd::Hung,
        RunEnd::TurnLimit,
        RunEnd::Stopped,
        RunEnd::QuotaExhausted { resets_at: None },
        RunEnd::RateLimited,
    ] {
        for used_a_tool in [false, true] {
            for first_of_session in [false, true] {
                for already_retried in [false, true] {
                    assert_eq!(
                        after_failed_without_tool(&end, used_a_tool, first_of_session, already_retried),
                        AfterFailedWithoutTool::Fail,
                        "end {end:?}, used_a_tool {used_a_tool}, first_of_session {first_of_session}, already_retried {already_retried}"
                    );
                }
            }
        }
    }
}