//! The findings of the checks that name a commit already in the branch.
//!
//! A frozen test that a commit took lines out of cannot be repaired by the team: the commit is in the branch for good,
//! and published history is never rewritten. Only the human may allow such a finding, so the round is not sent back to
//! the team for it; what they say is kept by the commit and the path for the item until it is closed, and the same
//! finding is neither asked nor sent back again. A new commit, or a new path, is a new finding.

use std::collections::BTreeMap;

use super::{Issue, RoundFacts, Subject};
use crate::repo_path::RepoPath;
use crate::state::CommitId;
use crate::task_allowance::TaskAllowance;

/// A commit that took lines out of a test that a `RED:` commit of the round froze, and is not a `TEST-FIX:` commit of
/// test files only that the description names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFinding {
    /// The commit that took the lines out.
    pub commit: CommitId,
    /// The test it edited.
    pub path: RepoPath,
    /// The `RED:` commit that touched the test first.
    pub red: CommitId,
}

impl CommitFinding {
    /// What the human allows or refuses, and what is kept for the item: the commit and the path.
    #[must_use]
    pub fn subject(&self) -> Subject {
        Subject::Commit {
            commit: self.commit.clone(),
            path: self.path.clone(),
        }
    }
}

/// The issues of a round told apart by whom they wait for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Findings {
    /// What the team cannot repair: it names a commit that is in the branch.
    pub commits: Vec<CommitFinding>,
    /// Every other issue, in the order the checks found it.
    pub rest: Vec<Issue>,
}

/// Tells the issues about a commit from the others.
#[must_use]
pub fn split(issues: Vec<Issue>) -> Findings {
    Findings {
        commits: Vec::new(),
        rest: issues,
    }
}

/// What was said about the findings of the item before.
#[derive(Debug, Clone, Copy)]
pub struct Said<'a> {
    /// What the human allowed.
    pub allowed: &'a [Subject],
    /// What the human refused.
    pub refused: &'a [Subject],
    /// The files the human allowed before the findings had a subject of their own, with the head the checks looked at
    /// then; none for a file allowed before a head was recorded.
    pub paths: &'a BTreeMap<RepoPath, Option<CommitId>>,
    /// What the task list allows beforehand.
    pub task_list: &'a [TaskAllowance],
}

/// What is left of the findings once what was said is taken into account.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settled {
    /// The findings nobody has said anything about: the human is asked.
    pub ask: Vec<CommitFinding>,
    /// The subjects of the findings that a file allowed before covers: they are kept as they are from now on.
    pub inherited: Vec<Subject>,
}

/// Which of the `findings` of the round `facts` still need the human.
#[must_use]
pub fn settle(findings: Vec<CommitFinding>, _facts: &RoundFacts, _said: &Said<'_>) -> Settled {
    Settled {
        ask: findings,
        inherited: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
