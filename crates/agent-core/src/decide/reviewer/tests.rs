use super::*;

const PATIENCE: Duration = Duration::from_mins(15);

/// A review whose reviewer is `reviewer` and whose last run ended `quiet_for` ago, in a team that goes on.
fn awaited(reviewer: TaskStatus, quiet_for: Duration) -> Awaited {
    Awaited {
        reviewer,
        lead: TaskStatus::Waiting,
        quiet_for,
        patience: PATIENCE,
    }
}

// Rule: a reviewer whose task failed or was stopped will not review: it is replaced at once.
#[test]
fn a_reviewer_that_failed_or_was_stopped_is_replaced_at_once() {
    for status in [TaskStatus::Failed, TaskStatus::Stopped] {
        assert!(replace_now(&awaited(status, Duration::ZERO)), "{status:?}");
    }
}

// Rule: a paused reviewer is given the time a reviewer may be paused, and replaced when it is over.
#[test]
fn a_paused_reviewer_is_replaced_when_it_was_paused_as_long_as_it_may_be() {
    let almost = PATIENCE.saturating_sub(Duration::from_secs(1));

    assert!(!replace_now(&awaited(TaskStatus::Paused, Duration::ZERO)));
    assert!(!replace_now(&awaited(TaskStatus::Paused, almost)));
    assert!(replace_now(&awaited(TaskStatus::Paused, PATIENCE)));
    assert!(replace_now(&awaited(TaskStatus::Paused, Duration::from_hours(8))));
}

// Rule: a reviewer that works on its review, is about to be given it or has reported is not replaced, however long
// it takes.
#[test]
fn a_reviewer_that_works_waits_or_reported_is_not_replaced() {
    for status in [
        TaskStatus::Queued,
        TaskStatus::Running,
        TaskStatus::Questions,
        TaskStatus::Waiting,
        TaskStatus::Done,
    ] {
        assert!(!replace_now(&awaited(status, Duration::from_hours(8))), "{status:?}");
    }
}

// Rule: a team whose lead was stopped or failed does not go on: its reviewers are not replaced, so that stopping a team
// starts nobody.
#[test]
fn the_reviewers_of_a_team_whose_lead_is_over_are_not_replaced() {
    for lead in [TaskStatus::Stopped, TaskStatus::Failed] {
        for reviewer in [TaskStatus::Stopped, TaskStatus::Failed, TaskStatus::Paused] {
            let review = Awaited {
                lead,
                ..awaited(reviewer, Duration::from_hours(8))
            };
            assert!(!replace_now(&review), "lead {lead:?}, reviewer {reviewer:?}");
        }
    }
    // A lead that is done handed its work over: the review of it still has to come.
    let review = Awaited {
        lead: TaskStatus::Done,
        ..awaited(TaskStatus::Failed, Duration::ZERO)
    };
    assert!(replace_now(&review));
}

/// When the review is asked in the tests of [`quiet_since`].
fn asked() -> Timestamp {
    "2026-11-23T21:40:00Z".parse().unwrap()
}

/// `minutes` after the review was asked; before it when negative.
fn at(minutes: i64) -> Timestamp {
    Timestamp::from_utc(asked().as_utc() + time::Duration::minutes(minutes)).unwrap()
}

/// A run that ended `minutes` after the review was asked, the way `end` says.
fn ended(minutes: i64, end: crate::state::RunEnd) -> RunState {
    RunState::Ended { at: at(minutes), end }
}

/// A run in which the agent reported, `minutes` after the review was asked.
fn reported(minutes: i64) -> RunState {
    ended(
        minutes,
        crate::state::RunEnd::Reported {
            outcome: crate::status::RunOutcome::Sleeping,
        },
    )
}

/// A run its provider refused with a rate limit, `minutes` after the review was asked.
fn rate_limited(minutes: i64) -> RunState {
    ended(minutes, crate::state::RunEnd::RateLimited)
}

/// A run its provider refused for a used-up quota, `minutes` after the review was asked.
fn out_of_quota(minutes: i64) -> RunState {
    ended(minutes, crate::state::RunEnd::QuotaExhausted { resets_at: None })
}

// Rule: a reviewer that its provider refused at every run since the review was asked has been quiet since the request.
#[test]
fn a_reviewer_refused_at_every_run_is_quiet_since_the_request() {
    let rate_limits = [rate_limited(1), rate_limited(9), rate_limited(17), rate_limited(26)];
    assert_eq!(quiet_since(rate_limits.iter(), asked()), asked());

    let quotas = [out_of_quota(5), out_of_quota(30)];
    assert_eq!(quiet_since(quotas.iter(), asked()), asked());

    let both = [rate_limited(3), out_of_quota(11), rate_limited(19)];
    assert_eq!(quiet_since(both.iter(), asked()), asked());
}

// Rule: runs refused by the provider after the reviewer worked do not end its quiet time: it counts from its work.
#[test]
fn refusals_after_work_count_from_the_work() {
    let runs = [reported(10), rate_limited(20), out_of_quota(28)];
    assert_eq!(quiet_since(runs.iter(), asked()), at(10));
}

// Rule: the quiet time of a review never starts before it was asked, whatever the reviewer did before.
#[test]
fn runs_before_the_request_do_not_count() {
    let worked_before = [reported(-60), rate_limited(5)];
    assert_eq!(quiet_since(worked_before.iter(), asked()), asked());

    let only_before = [reported(-60)];
    assert_eq!(quiet_since(only_before.iter(), asked()), asked());
}

// Rule: a reviewer that has not run, or runs now, counts from the request, as before.
#[test]
fn a_reviewer_without_an_ended_run_counts_from_the_request() {
    assert_eq!(quiet_since(std::iter::empty(), asked()), asked());

    let running = [rate_limited(4), RunState::Running];
    assert_eq!(quiet_since(running.iter(), asked()), asked());
}

// Rule: a run that ended any other way than refused by the provider ends the quiet time, as before: a stop of the
// human, a run without a status, a hung run, a run out of turns, a report.
#[test]
fn every_other_end_counts_as_its_end() {
    use crate::state::RunEnd;
    for end in [
        RunEnd::Stopped,
        RunEnd::NoStatus { exit_code: Some(1) },
        RunEnd::Hung,
        RunEnd::TurnLimit,
        RunEnd::Reported {
            outcome: crate::status::RunOutcome::Done,
        },
    ] {
        let runs = [rate_limited(5), ended(12, end.clone())];
        assert_eq!(quiet_since(runs.iter(), asked()), at(12), "{end:?}");
    }
}

// Rule: a reviewer that its provider refuses again and again is replaced once it was refused as long as a reviewer may
// be paused, however short each pause is (issue #234).
#[test]
fn a_reviewer_refused_again_and_again_is_replaced_after_the_patience() {
    let runs = [rate_limited(1), rate_limited(9), rate_limited(17), rate_limited(26)];
    let quiet_for = |now: Timestamp| {
        Duration::try_from(now.as_utc() - quiet_since(runs.iter(), asked()).as_utc()).unwrap_or_default()
    };

    assert!(!replace_now(&awaited(TaskStatus::Paused, quiet_for(at(14)))));
    assert!(replace_now(&awaited(TaskStatus::Paused, quiet_for(at(15)))));
    assert!(replace_now(&awaited(TaskStatus::Paused, quiet_for(at(27)))));
}

// Rule: a paused reviewer whose agent the refusal rule changes is replaced at once, whatever the patience.
#[test]
fn a_paused_reviewer_whose_agent_the_rule_changes_is_replaced_at_once() {
    assert!(replace_after_refusal(
        &awaited(TaskStatus::Paused, Duration::ZERO),
        Some(Decision::ChangeAgent)
    ));
}

// Rule: a reviewer that was not refused is replaced by the patience alone, as before.
#[test]
fn a_reviewer_the_rule_keeps_or_that_was_not_refused_waits_the_patience() {
    let almost = PATIENCE.saturating_sub(Duration::from_secs(1));
    let refusal = None;
    assert!(!replace_after_refusal(
        &awaited(TaskStatus::Paused, Duration::ZERO),
        refusal
    ));
    assert!(!replace_after_refusal(&awaited(TaskStatus::Paused, almost), refusal));
    assert!(replace_after_refusal(&awaited(TaskStatus::Paused, PATIENCE), refusal));
}

// Rule: a reviewer the rule pauses (refused once, its retry still to come) is kept until the retry, however long its
// pause lasts: the silence of a refused run is not the patience's to count (review of #273).
#[test]
fn a_reviewer_the_rule_pauses_is_kept_whatever_the_silence() {
    for quiet_for in [Duration::ZERO, PATIENCE, Duration::from_hours(8)] {
        assert!(
            !replace_after_refusal(&awaited(TaskStatus::Paused, quiet_for), Some(Decision::Pause)),
            "{quiet_for:?}"
        );
    }
}

// Rule: the answer of the rule moves only a paused reviewer: one that works, waits, asks or reported is never replaced
// by it, and one that failed or was stopped is replaced at once whatever the answer.
#[test]
fn the_rule_changes_nothing_for_a_reviewer_that_is_not_paused() {
    for refusal in [Some(Decision::ChangeAgent), Some(Decision::Pause), None] {
        for status in [
            TaskStatus::Queued,
            TaskStatus::Running,
            TaskStatus::Questions,
            TaskStatus::Waiting,
            TaskStatus::Done,
        ] {
            assert!(
                !replace_after_refusal(&awaited(status, Duration::from_hours(8)), refusal),
                "{status:?} {refusal:?}"
            );
        }
        for status in [TaskStatus::Failed, TaskStatus::Stopped] {
            assert!(
                replace_after_refusal(&awaited(status, Duration::ZERO), refusal),
                "{status:?} {refusal:?}"
            );
        }
    }
}

// Rule: a team whose lead is over does not go on: a refused reviewer is not replaced either.
#[test]
fn the_rule_does_not_replace_the_reviewer_of_a_team_whose_lead_is_over() {
    for lead in [TaskStatus::Stopped, TaskStatus::Failed] {
        let review = Awaited {
            lead,
            ..awaited(TaskStatus::Paused, Duration::from_hours(8))
        };
        assert!(!replace_after_refusal(&review, Some(Decision::ChangeAgent)), "{lead:?}");
    }
}

/// An investigator name that is valid.
fn investigator(name: &str) -> InvestigatorName {
    name.parse().unwrap()
}

// Rule: the investigators of the refused provider are the ones a replacement skips; a provider that is only a prefix of
// another one, or another CLI, is not the same provider.
#[test]
fn the_investigators_of_the_refused_provider_are_picked_out() {
    let (kimi, qwen, router, claude) = (
        investigator("inv-kimi"),
        investigator("inv-qwen"),
        investigator("inv-router"),
        investigator("inv-claude"),
    );
    let all = [
        Provided {
            name: &kimi,
            provider: "deepinfra",
        },
        Provided {
            name: &qwen,
            provider: "deepinfra",
        },
        Provided {
            name: &router,
            provider: "openrouter",
        },
        Provided {
            name: &claude,
            provider: "claude",
        },
    ];

    assert_eq!(
        on_provider("deepinfra", &all),
        BTreeSet::from([kimi.clone(), qwen.clone()])
    );
    assert_eq!(on_provider("deep", &all), BTreeSet::new());
    assert_eq!(on_provider("codex", &all), BTreeSet::new());
    assert_eq!(on_provider("claude", &[]), BTreeSet::new());
}
