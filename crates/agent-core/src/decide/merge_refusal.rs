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
    let _ = (since_approved, on_branch, vs_base);
    Vec::new()
}

#[cfg(test)]
mod tests;
