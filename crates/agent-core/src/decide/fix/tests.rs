use super::*;

#[test]
fn completed_fix_passes_require_commits_only_for_claimed_or_missing_fixes() {
    let cases = [
        (Progress::Committed, Budget::Available, Step::Verify),
        (Progress::Committed, Budget::Exhausted, Step::Verify),
        (Progress::Gating, Budget::Available, Step::WaitGate),
        (Progress::Gating, Budget::Exhausted, Step::WaitGate),
        (Progress::Unchanged(Answers::Accounted), Budget::Available, Step::Verify),
        (Progress::Unchanged(Answers::Accounted), Budget::Exhausted, Step::Verify),
        (
            Progress::Unchanged(Answers::Fixed),
            Budget::Available,
            Step::Correct(Hold::NoCommit),
        ),
        (
            Progress::Unchanged(Answers::Missing),
            Budget::Available,
            Step::Correct(Hold::NoCommit),
        ),
        (
            Progress::Unchanged(Answers::Fixed),
            Budget::Exhausted,
            Step::AskHuman(Hold::NoCommit),
        ),
        (
            Progress::Unchanged(Answers::Missing),
            Budget::Exhausted,
            Step::AskHuman(Hold::NoCommit),
        ),
        (Progress::StaleGate, Budget::Available, Step::Correct(Hold::StaleGate)),
        (Progress::StaleGate, Budget::Exhausted, Step::AskHuman(Hold::StaleGate)),
    ];
    for (progress, budget, expected) in cases {
        assert_eq!(fix_step(progress, budget), expected, "{progress:?}, {budget:?}");
    }
}
