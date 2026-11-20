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
