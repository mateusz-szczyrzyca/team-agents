//! The decisions after a refused merge, as a table of cases.

use super::*;

/// An allowance written as the task list writes it.
fn allow(text: &str) -> TaskAllowance {
    TaskAllowance::parse(text).expect("a valid allowance")
}

/// A repository path written as git prints it.
fn path(text: &str) -> RepoPath {
    text.parse().expect("a valid repository path")
}

/// The allowances of a task that lets its version, its lock file and its history of versions change.
fn version_allowances() -> Vec<TaskAllowance> {
    vec![allow("Cargo.toml"), allow("Cargo.lock"), allow("WhatsNew.md")]
}

#[test]
fn a_conflict_with_passes_left_sends_the_lead_to_merge_the_base() {
    let budget = ConflictBudget { spent: 0, limit: 3 };

    assert_eq!(after_refusal(Refusal::Conflict, budget), AfterRefusal::LeadMergesBase);
}

#[test]
fn a_conflict_on_the_last_pass_still_sends_the_lead() {
    let budget = ConflictBudget { spent: 2, limit: 3 };

    assert_eq!(after_refusal(Refusal::Conflict, budget), AfterRefusal::LeadMergesBase);
}

#[test]
fn a_conflict_after_the_limit_asks_the_human() {
    let budget = ConflictBudget { spent: 3, limit: 3 };

    assert_eq!(after_refusal(Refusal::Conflict, budget), AfterRefusal::AskHuman);
}

#[test]
fn a_conflict_with_limit_zero_tells_the_human_as_before() {
    let budget = ConflictBudget { spent: 0, limit: 0 };

    assert_eq!(after_refusal(Refusal::Conflict, budget), AfterRefusal::TellHuman);
}

#[test]
fn another_refusal_tells_the_human_whatever_the_budget() {
    let fresh = ConflictBudget { spent: 0, limit: 3 };
    let spent = ConflictBudget { spent: 3, limit: 3 };

    assert_eq!(after_refusal(Refusal::Other, fresh), AfterRefusal::TellHuman);
    assert_eq!(after_refusal(Refusal::Other, spent), AfterRefusal::TellHuman);
}

#[test]
fn a_lead_that_changed_only_allowed_files_retries_the_merge() {
    let end = LeadEnd::Result {
        changed: vec![path("Cargo.toml"), path("Cargo.lock"), path("WhatsNew.md")],
    };

    assert_eq!(after_lead(&end, &version_allowances()), AfterLead::RetryMerge);
}

#[test]
fn a_lead_that_changed_nothing_retries_the_merge() {
    let end = LeadEnd::Result { changed: Vec::new() };

    assert_eq!(after_lead(&end, &version_allowances()), AfterLead::RetryMerge);
}

#[test]
fn a_lead_that_changed_another_file_is_verified_before_the_merge() {
    let end = LeadEnd::Result {
        changed: vec![path("crates/agent-core/src/lib.rs"), path("Cargo.toml")],
    };

    assert_eq!(after_lead(&end, &version_allowances()), AfterLead::VerifyFirst);
}

#[test]
fn a_changed_file_the_task_does_not_allow_is_verified_even_without_allowances() {
    let end = LeadEnd::Result {
        changed: vec![path("Cargo.toml")],
    };

    assert_eq!(after_lead(&end, &[]), AfterLead::VerifyFirst);
}

#[test]
fn a_lead_without_a_result_asks_the_human() {
    assert_eq!(
        after_lead(&LeadEnd::NoResult, &version_allowances()),
        AfterLead::AskHuman
    );
}

#[test]
fn a_file_only_the_merge_of_the_base_brought_in_is_not_a_change_of_the_lead() {
    let since = vec![path("crates/base_only.rs")];

    assert_eq!(changed_by_lead(&since, &[], &[]), Vec::<RepoPath>::new());
}

#[test]
fn a_file_the_approved_branch_changed_is_a_change_of_the_lead_when_it_changed_again() {
    let since = vec![path("crates/a.rs")];
    let on_branch = vec![path("crates/a.rs")];

    assert_eq!(changed_by_lead(&since, &on_branch, &[]), vec![path("crates/a.rs")]);
}

#[test]
fn a_file_the_merged_result_differs_in_from_the_new_base_is_a_change_of_the_lead() {
    let since = vec![path("Cargo.toml")];
    let vs_base = vec![path("Cargo.toml")];

    assert_eq!(changed_by_lead(&since, &[], &vs_base), vec![path("Cargo.toml")]);
}

#[test]
fn only_the_changed_files_of_the_lead_are_listed_in_their_order() {
    let since = vec![path("a.rs"), path("base_only.rs"), path("c.rs")];
    let on_branch = vec![path("c.rs")];
    let vs_base = vec![path("a.rs")];

    assert_eq!(
        changed_by_lead(&since, &on_branch, &vs_base),
        vec![path("a.rs"), path("c.rs")]
    );
}

#[test]
fn an_unpublished_conflict_result_consumes_no_conflict_pass() {
    assert_eq!(
        account_conflict_pass(ConflictPublishResult::Failed, AccountingState::Unrecorded),
        ConflictAccountingStep::RetainBudget
    );
}

#[test]
fn successful_publication_records_conflict_pass_exactly_once() {
    assert_eq!(
        account_conflict_pass(ConflictPublishResult::Published, AccountingState::Unrecorded),
        ConflictAccountingStep::SpendPass
    );
    assert_eq!(
        account_conflict_pass(ConflictPublishResult::Published, AccountingState::AlreadyRecorded),
        ConflictAccountingStep::RetainBudget
    );
}

#[test]
fn repeated_failures_of_same_result_cannot_refund_earlier_passes() {
    assert_eq!(
        account_conflict_pass(ConflictPublishResult::Failed, AccountingState::AlreadyRecorded),
        ConflictAccountingStep::RetainBudget
    );
}

#[test]
fn recover_exhausted_split_round_table() {
    // 1. Merged PR recovers to CompleteMerged
    assert_eq!(
        recover_exhausted_split_round(SplitPullStatus::Merged),
        ExhaustedRoundRecovery::CompleteMerged
    );

    // 2. Open PR with valid gated commit and unexhausted passes recovers to ResumeMerge
    let open_ready = SplitPullStatus::Open {
        gating: GatingStatus::Gated,
        budget: ConflictBudget { spent: 1, limit: 2 },
        conflict: ConflictPublication::Published,
    };
    assert_eq!(
        recover_exhausted_split_round(open_ready),
        ExhaustedRoundRecovery::ResumeMerge
    );

    // 3. Open PR with exhausted passes but unpublished conflict pass recovers to ResumeMerge
    let open_unpublished = SplitPullStatus::Open {
        gating: GatingStatus::Gated,
        budget: ConflictBudget { spent: 2, limit: 2 },
        conflict: ConflictPublication::Unpublished,
    };
    assert_eq!(
        recover_exhausted_split_round(open_unpublished),
        ExhaustedRoundRecovery::ResumeMerge
    );

    // 4. Open PR with exhausted passes and published conflict pass waits for human
    let exhausted = SplitPullStatus::Open {
        gating: GatingStatus::Gated,
        budget: ConflictBudget { spent: 2, limit: 2 },
        conflict: ConflictPublication::Published,
    };
    assert_eq!(
        recover_exhausted_split_round(exhausted),
        ExhaustedRoundRecovery::WaitHuman
    );

    // 5. Open PR without gated commit waits for human
    let ungated = SplitPullStatus::Open {
        gating: GatingStatus::Ungated,
        budget: ConflictBudget { spent: 0, limit: 2 },
        conflict: ConflictPublication::Unpublished,
    };
    assert_eq!(
        recover_exhausted_split_round(ungated),
        ExhaustedRoundRecovery::WaitHuman
    );

    // 6. Closed unmerged PR waits for human
    assert_eq!(
        recover_exhausted_split_round(SplitPullStatus::ClosedUnmerged),
        ExhaustedRoundRecovery::WaitHuman
    );
}

/// Retains the pre-investigator decision cases with the optional pass disabled.
fn after_refusal(refusal: Refusal, budget: ConflictBudget) -> AfterRefusal {
    super::after_refusal(
        refusal,
        budget,
        InvestigatorPass {
            enabled: false,
            filed_for_head: false,
        },
    )
}

#[test]
fn a_spent_conflict_budget_asks_an_investigator_only_when_enabled_and_not_filed() {
    for (enabled, filed_for_head, expected) in [
        (true, false, AfterRefusal::AskInvestigator),
        (true, true, AfterRefusal::AskHuman),
        (false, false, AfterRefusal::AskHuman),
        (false, true, AfterRefusal::AskHuman),
    ] {
        assert_eq!(
            super::after_refusal(
                Refusal::Conflict,
                ConflictBudget { spent: 3, limit: 3 },
                InvestigatorPass {
                    enabled,
                    filed_for_head
                }
            ),
            expected
        );
    }
}
