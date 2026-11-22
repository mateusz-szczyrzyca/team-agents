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

fn time(text: &str) -> crate::state::Timestamp {
    text.parse().unwrap()
}

// Rule: a response is judged only when it was given in a run that started at or after the verdict: never a message that
// was given before the review, and never a run that merely ended after it.
#[test]
fn a_lead_run_is_judged_only_when_it_started_after_the_verdict() {
    let verdict = Some(time("2026-11-22T01:00:00Z"));
    let run = |started: &str, _ended: &str| Some(LeadRun::started_at(time(started)));
    let cases = [
        (None, verdict, Response::None, "no completed run"),
        (
            run("2026-11-22T00:10:00Z", "2026-11-22T00:50:00Z"),
            verdict,
            Response::BeforeVerdict,
            "a message given before the verdict",
        ),
        (
            run("2026-11-22T00:50:00Z", "2026-11-22T01:00:01Z"),
            verdict,
            Response::BeforeVerdict,
            "a run that started before the verdict and ended after it",
        ),
        (
            run("2026-11-22T01:00:00Z", "2026-11-22T01:20:00Z"),
            verdict,
            Response::AfterVerdict,
            "a run that started at the verdict",
        ),
        (
            run("2026-11-22T01:05:00Z", "2026-11-22T01:20:00Z"),
            verdict,
            Response::AfterVerdict,
            "a run after the verdict",
        ),
        (
            run("2026-11-22T01:05:00Z", "2026-11-22T01:20:00Z"),
            None,
            Response::AfterVerdict,
            "a pass that was never asked for has no verdict to come after",
        ),
    ];

    for (lead, verdict_at, want, name) in cases {
        assert_eq!(response_of(lead, verdict_at), want, "{name}");
    }
}
