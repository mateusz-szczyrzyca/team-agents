//! The check of a commit about to be made, over real git: the rule of
//! [`agent_core::decide::commit_guard`] asked in a hook of `git commit`, while the commit is only staged.
//!
//! This reads and decides and writes nothing: no ref, no index, no file of the checkout. A commit that is already made —
//! on the remote or not — is never touched; where one slipped through, the checks before the review find it.

pub mod hooks;
pub mod text;

use agent_core::config::Checks;
use agent_core::decide::commit_guard::{PendingCommit, Verdict, frozen_tests, judge_commit};
use agent_core::ids::BranchName;

use crate::git::{Git, GitError, Revision, StagedAgainst};

/// The exit status of `agent guard-commit` that refuses a commit, and that the hook stops on: every other status of the
/// program (it cannot start, it cannot write, it was usage) lets the commit through.
pub const COMMIT_REFUSED_STATUS: u8 = 86;

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

/// The files whose presence in the git directory says which [`Operation`] is under way, in the order they are looked for.
const OPERATION_FILES: [(&str, Operation); 5] = [
    ("MERGE_HEAD", Operation::Merge),
    ("CHERRY_PICK_HEAD", Operation::CherryPick),
    ("REVERT_HEAD", Operation::Revert),
    ("rebase-merge", Operation::Rebase),
    ("rebase-apply", Operation::Rebase),
];

/// The operation under way in the checkout of `git`, if one is.
///
/// # Errors
/// [`GuardError::Git`] when git cannot say where the files of an operation are; [`GuardError::Look`].
pub async fn operation_in_progress(git: &Git) -> Result<Option<Operation>, GuardError> {
    for (name, operation) in OPERATION_FILES {
        let path = git.git_path(name).await.map_err(|source| GuardError::Git {
            step: GuardStep::Operation,
            source,
        })?;
        match tokio::fs::try_exists(&path).await {
            Ok(true) => return Ok(Some(operation)),
            Ok(false) => {}
            Err(source) => return Err(GuardError::Look { path, source }),
        }
    }
    Ok(None)
}

/// `step` as the way to turn the failure of a git call into a [`GuardError`].
fn at(step: GuardStep) -> impl Fn(GitError) -> GuardError {
    move |source| GuardError::Git { step, source }
}

/// Whether the commit with `subject` that is about to be made in the checkout of `git` may be made, for the branch that
/// started at the base branch `base`, under the test files of `rules`.
///
/// The base is the remote-tracking branch as it was last fetched: nothing is fetched here.
///
/// # Errors
/// [`GuardError::Git`] for a step git fails; [`GuardError::Look`].
pub async fn judge_staged(
    git: &Git,
    rules: &Checks,
    base: &BranchName,
    subject: &str,
    kind: CommitKind,
) -> Result<Judged, GuardError> {
    if let Some(operation) = operation_in_progress(git).await? {
        return Ok(Judged::NotAuthored(operation));
    }
    let head = git.rev_parse(&Revision::Head).await.map_err(at(GuardStep::Head))?;
    // The commit that is made sits on `parent`: `HEAD`, or for an amend what `HEAD` sat on.
    let parent = match kind {
        CommitKind::New => Some(head),
        CommitKind::Amend => git
            .resolve_commit(&format!("{head}^"))
            .await
            .map_err(at(GuardStep::Head))?,
    };
    let (commits, against) = match parent {
        Some(parent) => {
            let merge_base = git
                .merge_base(&Revision::RemoteBranch(base.clone()), &Revision::Commit(parent.clone()))
                .await
                .map_err(at(GuardStep::MergeBase))?;
            let commits = git
                .range_commits(&merge_base, &parent)
                .await
                .map_err(at(GuardStep::Commits))?;
            (commits, StagedAgainst::Commit(parent))
        }
        // An amended commit with no parent is the first of its history: nothing came before it to freeze a test.
        None => (Vec::new(), StagedAgainst::EmptyTree),
    };
    let files = git.staged_changes(&against).await.map_err(at(GuardStep::Staged))?;
    let frozen = frozen_tests(rules, &commits);
    Ok(Judged::Verdict(judge_commit(
        rules,
        &frozen,
        &PendingCommit { subject, files: &files },
    )))
}
