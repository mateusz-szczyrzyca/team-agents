//! The check of a commit that is about to be made: would it change a test that a `RED:` commit froze (TDD-14)?
//!
//! The checks before a review find such a commit after it is made, and by then it may be published and nothing but a
//! rewrite of history could remove it. This is the same rule asked earlier, while the commit is only staged: a commit
//! that takes lines out of a test file that a `RED:` commit of the round touched is made as a `TEST-FIX:` commit that
//! changes test files only, or not at all. [`frozen_tests`] says which files are frozen and [`judge_commit`] decides on
//! the staged change; the daemon's checks stay the second look, where a commit slipped through (`--no-verify`).

use crate::config::Checks;
use crate::decide::checks::{CommitFacts, FileChange};
use crate::repo_path::RepoPath;
use crate::state::CommitId;

/// A test file that a `RED:` commit of the round touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenTest {
    /// The file.
    pub path: RepoPath,
    /// The first `RED:` commit of the round that touched it.
    pub red: CommitId,
}

/// The test files that the `RED:` commits among `commits` touched, each once, with the first such commit, in the order
/// the files first appear.
#[must_use]
pub fn frozen_tests(_rules: &Checks, _commits: &[CommitFacts]) -> Vec<FrozenTest> {
    Vec::new()
}

/// The commit that is about to be made: its subject and what it changes against its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingCommit<'a> {
    /// The first line of its message.
    pub subject: &'a str,
    /// The files it changes against its parent; a rename is a deletion and an addition.
    pub files: &'a [FileChange],
}

/// What a commit takes out of a frozen test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    /// Some lines of a text file: the lines removed or replaced.
    Lines(u32),
    /// The whole file: it is deleted, or it is binary and replaced.
    WholeFile,
}

/// Why a commit is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The commit takes lines out of a frozen test and its subject does not start with `TEST-FIX:`.
    FrozenTestEdited {
        /// The frozen test.
        path: RepoPath,
        /// The `RED:` commit that froze it.
        red: CommitId,
        /// What the commit takes out of it.
        taken: Taken,
    },
    /// The commit is a `TEST-FIX:` that takes lines out of a frozen test and changes files that are not tests.
    TestFixTouchesOtherFiles {
        /// The files that are not tests, in the order of the change.
        others: Vec<RepoPath>,
    },
}

/// What is to happen to a commit that is about to be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// It is made.
    Allowed,
    /// It is not made, for these reasons.
    Refused(Vec<Refusal>),
}

/// Whether `commit` may be made, given the test files `frozen` by the round's `RED:` commits.
#[must_use]
pub fn judge_commit(_rules: &Checks, _frozen: &[FrozenTest], _commit: &PendingCommit<'_>) -> Verdict {
    Verdict::Allowed
}

#[cfg(test)]
mod tests;
