//! What the harness does after GitHub refuses to merge an approved pull request: the lead merges the base into its
//! branch when the refusal is a conflict and the round has passes left for it, and the change the lead made is
//! verified before the merge is tried again when it touched files the task did not allow.

use crate::repo_path::RepoPath;
use crate::task_allowance::TaskAllowance;

/// Why GitHub refused the merge of an approved pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The pull request conflicts with its base; the lead can merge the base and resolve the conflicts.
    Conflict,
    /// Any other refusal: a required check failed, the token may not merge, or the values were refused.
    Other,
}

/// The lead's passes on merge conflicts in one round: how many were spent and how many the configuration allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConflictBudget {
    /// The passes that were already run for this round.
    pub spent: u32,
    /// The configured limit (`[merge] conflict_passes`); zero keeps the behaviour from before the passes existed.
    pub limit: u32,
}

/// What the daemon does after a refused merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterRefusal {
    /// The lead merges the base into its branch in a pass of its own.
    LeadMergesBase,
    /// The human is told about the refusal, as for any refusal.
    TellHuman,
    /// The human is asked once, in a questions file: the conflict passes of the round are spent.
    AskHuman,
}

/// How the lead's pass after the merge of the base ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeadEnd {
    /// The run ended with a result; `changed` are the files it changed after the merge of the base.
    Result {
        /// Every file the run changed, excluding what the merge of the base brought in.
        changed: Vec<RepoPath>,
    },
    /// The run ended without a result: nothing it did can be trusted.
    NoResult,
}

/// What the daemon does after the lead's pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterLead {
    /// The change touched only files the task allows; the approval stands and the merge is tried again.
    RetryMerge,
    /// The change touched other files; a verification review runs before the merge is tried again.
    VerifyFirst,
    /// The lead gave no result; the human is asked once, in a questions file.
    AskHuman,
}

#[must_use]
/// What follows a refused merge, given the refusal and the conflict passes the round has left.
pub fn after_refusal(refusal: Refusal, budget: ConflictBudget) -> AfterRefusal {
    match refusal {
        Refusal::Other => AfterRefusal::TellHuman,
        Refusal::Conflict if budget.limit == 0 => AfterRefusal::TellHuman,
        Refusal::Conflict if budget.spent >= budget.limit => AfterRefusal::AskHuman,
        Refusal::Conflict => AfterRefusal::LeadMergesBase,
    }
}

#[must_use]
/// What follows the lead's pass, given what the run changed and the files the task allows.
pub fn after_lead(end: &LeadEnd, allowances: &[TaskAllowance]) -> AfterLead {
    match end {
        LeadEnd::NoResult => AfterLead::AskHuman,
        LeadEnd::Result { changed } => {
            let allowed = changed
                .iter()
                .all(|path| allowances.iter().any(|allowance| allowance.matches(path)));
            if allowed {
                AfterLead::RetryMerge
            } else {
                AfterLead::VerifyFirst
            }
        }
    }
}

/// The files the lead changed after the merge of the base, given the files changed since the approval (`since_approved`),
/// the files the approved branch changed itself (`on_branch`) and the files where the merged result differs from the new
/// base (`vs_base`): a file that only the merge of the base brought in is in none of the last two and is not counted.
#[must_use]
pub fn changed_by_lead(since_approved: &[RepoPath], on_branch: &[RepoPath], vs_base: &[RepoPath]) -> Vec<RepoPath> {
    since_approved
        .iter()
        .filter(|path| on_branch.contains(path) || vs_base.contains(path))
        .cloned()
        .collect()
}

/// The result of attempting to publish a conflict resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPublishResult {
    /// Publication succeeded and the resolution was pushed to the pull request branch.
    Published,
    /// Publication failed before reaching the pull request branch.
    Failed,
}

/// The accounting state of a conflict resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountingState {
    /// The pass has not yet been recorded for this conflict resolution.
    Unrecorded,
    /// The pass has already been recorded for this conflict resolution.
    AlreadyRecorded,
}

/// The outcome of conflict pass accounting on publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAccountingStep {
    /// The pass is recorded as spent exactly once upon successful publication.
    SpendPass,
    /// Publication failed or repeated failure occurred; no pass is spent and earlier passes are untouched.
    RetainBudget,
}

#[must_use]
/// Decides whether a conflict pass should be spent given the publication result and its accounting state.
pub fn account_conflict_pass(result: ConflictPublishResult, accounting: AccountingState) -> ConflictAccountingStep {
    match (result, accounting) {
        (ConflictPublishResult::Published, AccountingState::Unrecorded) => ConflictAccountingStep::SpendPass,
        (ConflictPublishResult::Published, AccountingState::AlreadyRecorded)
        | (ConflictPublishResult::Failed, _) => ConflictAccountingStep::RetainBudget,
    }
}

/// Whether the conflict resolution was published to the pull request branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPublication {
    /// The conflict pass was successfully published to the pull request branch.
    Published,
    /// The conflict pass was never published to the pull request branch.
    Unpublished,
}

/// The gating status of the round's commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatingStatus {
    /// A valid gated commit is recorded for this round.
    Gated,
    /// No valid gated commit is recorded.
    Ungated,
}

/// The state of the pull request and its conflict publication for split round recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitPullStatus {
    /// The pull request has already merged on remote.
    Merged,
    /// The pull request is open and ready for merge attempts.
    Open {
        /// The gating status of the commit to be merged.
        gating: GatingStatus,
        /// The conflict budget for the round.
        budget: ConflictBudget,
        /// Whether the last conflict pass reached the pull request branch.
        conflict: ConflictPublication,
    },
    /// The pull request was closed without merging.
    ClosedUnmerged,
}

/// What the daemon does when recovering an exhausted split round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExhaustedRoundRecovery {
    /// Complete the round because it was already merged.
    CompleteMerged,
    /// Resume merge attempts because valid gated work was published or ready without exhausting valid passes.
    ResumeMerge,
    /// Retain the exhausted state and continue waiting for the human.
    WaitHuman,
}

#[must_use]
/// Decides how to recover a split round that appeared exhausted.
pub fn recover_exhausted_split_round(status: SplitPullStatus) -> ExhaustedRoundRecovery {
    match status {
        SplitPullStatus::Merged => ExhaustedRoundRecovery::CompleteMerged,
        SplitPullStatus::Open {
            gating: GatingStatus::Gated,
            budget,
            conflict,
        } if budget.spent < budget.limit || conflict == ConflictPublication::Unpublished => {
            ExhaustedRoundRecovery::ResumeMerge
        }
        SplitPullStatus::Open { .. } | SplitPullStatus::ClosedUnmerged => ExhaustedRoundRecovery::WaitHuman,
    }
}

#[cfg(test)]
mod tests;
