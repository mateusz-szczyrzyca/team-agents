//! Exhaustive recovery policy table, including no-result and wrong-result boundaries.

use super::*;

#[test]
fn recovery_table_separates_missing_results_from_wrong_and_stale_results() {
    for (cause, preference) in [
        (Cause::Died, Preference::FirstFree),
        (Cause::Refused, Preference::OtherProvider),
        (Cause::Quota, Preference::OtherProvider),
        (Cause::NoStatus, Preference::FirstFree),
        (Cause::RestartQueued, Preference::FirstFree),
        (Cause::RestartRunning, Preference::FirstFree),
    ] {
        for returns in 0..=MAX_NO_RESULT_RETURNS {
            for elsewhere in [Elsewhere::Free, Elsewhere::SameProvider, Elsewhere::None] {
                let expected = if returns == MAX_NO_RESULT_RETURNS {
                    Decision::Return
                } else {
                    match elsewhere {
                        Elsewhere::Free | Elsewhere::SameProvider => Decision::Serve(preference),
                        Elsewhere::None => Decision::Wait(preference),
                    }
                };
                assert_eq!(
                    decide(Event::NoResult(cause), returns, elsewhere),
                    expected,
                    "cause={cause:?}, returns={returns}, elsewhere={elsewhere:?}"
                );
            }
        }
    }
    for returns in [0, MAX_NO_RESULT_RETURNS, u8::MAX] {
        for elsewhere in [Elsewhere::Free, Elsewhere::SameProvider, Elsewhere::None] {
            assert_eq!(decide(Event::Result, returns, elsewhere), Decision::Settle);
            assert_eq!(decide(Event::StaleAnswer, returns, elsewhere), Decision::Drop);
        }
        assert_eq!(
            decide(Event::NoResult(Cause::Died), u8::MAX, Elsewhere::Free),
            Decision::Return
        );
    }
}

#[test]
fn cause_notice_names_match_the_serialized_journal_names() {
    for cause in [
        Cause::Died,
        Cause::Refused,
        Cause::Quota,
        Cause::NoStatus,
        Cause::RestartQueued,
        Cause::RestartRunning,
    ] {
        assert_eq!(serde_json::to_value(cause).unwrap(), serde_json::json!(cause.as_str()));
    }
}
