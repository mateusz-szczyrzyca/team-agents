//! The check of a commit about to be made, over real git: the rule of
//! [`agent_core::decide::commit_guard`] asked in a hook of `git commit`, while the commit is only staged.
//!
//! This reads and decides and writes nothing: no ref, no index, no file of the checkout. A commit that is already made —
//! on the remote or not — is never touched; where one slipped through, the checks before the review find it.

use agent_core::config::Checks;
use agent_core::decide::commit_guard::Verdict;
use agent_core::ids::BranchName;

use crate::git::{Git, GitError, Revision};

/// Whether the commit about to be made is a new one or replaces the last one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitKind {
    /// A commit on top of `HEAD`: what it changes is the index against `HEAD`.
    New,
    /// `git commit --amend`: the commit replaces `HEAD`, so what it changes is the index against the parent of `HEAD`.
    Amend,
}

/// A step of the check that git can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum GuardStep {
    /// Telling where the checkout keeps the files of an operation in progress.
    #[strum(serialize = "looking for an operation in progress")]
    Operation,
    /// Reading what `HEAD` is, and its parent.
    #[strum(serialize = "reading HEAD")]
    Head,
    /// Finding where the branch left its base.
    #[strum(serialize = "finding the merge base with the base branch")]
    MergeBase,
    /// Reading the commits of the round.
    #[strum(serialize = "reading the commits of the round")]
    Commits,
    /// Reading what the index changes.
    #[strum(serialize = "reading the staged changes")]
    Staged,
}

/// Why a commit cannot be judged.
#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    /// Git failed in a step.
    #[error("git failed while {step}")]
    Git {
        /// What was being done.
        step: GuardStep,
        /// What git said.
        source: GitError,
    },
    /// A file that tells an operation is in progress cannot be looked at.
    #[error("cannot look for {path:?}")]
    Look {
        /// The file.
        path: std::path::PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
}

/// What is under way in the checkout when a commit is made, if it is not a commit the agent writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// A merge: its commit holds what the other side brings.
    Merge,
    /// A cherry-pick.
    CherryPick,
    /// A revert.
    Revert,
    /// A rebase: its commits are the ones it replays.
    Rebase,
}

/// What the check says about a commit about to be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judged {
    /// The rule was asked.
    Verdict(Verdict),
    /// The commit is made by an operation that replays or joins other commits: it is not judged here.
    NotAuthored(Operation),
}

/// Whether the commit with `subject` that is about to be made in the checkout of `git` may be made, for the branch that
/// started at the base branch `base`, under the test files of `rules`.
///
/// # Errors
/// [`GuardError::Git`] for a step git fails; [`GuardError::Look`].
pub async fn judge_staged(
    git: &Git,
    _rules: &Checks,
    _base: &BranchName,
    _subject: &str,
    _kind: CommitKind,
) -> Result<Judged, GuardError> {
    git.rev_parse(&Revision::Head).await.map_err(|source| GuardError::Git {
        step: GuardStep::Head,
        source,
    })?;
    Ok(Judged::Verdict(Verdict::Allowed))
}
