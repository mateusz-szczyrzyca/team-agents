//! The checks before a review: the facts of a round's diff in, the issues out.
//!
//! The daemon reads the facts from git — what changed from the merge base with the base branch to the head, the commits
//! on the way, the lines they added and the pull request's description — and [`assess`] turns them into [`Issue`]s with
//! no model and no I/O. What is a test, what is production code and what judges a round comes from the repository's
//! [`Checks`] table; nothing here names a repository or its layout.
//!
//! The rules, each of them one function of the facts:
//!
//! - **An existing test file changed other than by added lines**, removed or renamed ([`Issue::TestChanged`]): only the
//!   human may allow it.
//! - **A protected file changed** ([`Issue::ProtectedChanged`]): only the human may allow it.
//! - **A test file that a `RED:` commit touched has lines removed or replaced by a later commit**
//!   ([`Issue::RedTestEdited`], TDD-14), unless that commit touches test files only, its subject starts with
//!   `TEST-FIX:` and the pull request's description names it under `## Tests changed after RED`.
//! - **The round changes production code and adds no line to a test file** ([`Issue::NoTests`], TDD-1), unless a commit
//!   of the round says `Test-First: not possible — <reason>`.
//! - **An added line that skips a test, suppresses a lint or asks whether the code runs under test**
//!   ([`Issue::Marker`], TDD-11, LINT-3, TDD-12), in a file the description does not name; the markers are a built-in
//!   table per file extension ([`markers`]).
//!
//! A test module inside a source file is not a test file: it stays with the reviewers.
//!
//! The issues of the first two rules are [`Judge::Human`]'s: the team cannot allow them. The others are the team's to
//! fix, and the human's to allow when the team cannot.

use serde::{Deserialize, Serialize};

use crate::config::Checks;
use crate::repo_path::RepoPath;
use crate::state::CommitId;

pub mod markers;

/// How a file differs between two commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The file is new.
    Added,
    /// The file has other contents.
    Modified,
    /// The file is gone.
    Deleted,
    /// The file moved; it may have other contents too.
    Renamed {
        /// Where it was.
        from: RepoPath,
    },
}

/// How many lines a change added and took away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lines {
    /// A text file.
    Text {
        /// Lines the change added.
        added: u32,
        /// Lines the change removed; a replaced line counts as one added and one removed.
        removed: u32,
    },
    /// A binary file: git counts no lines.
    Binary,
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// The file's path after the change; for a deleted file, where it was.
    pub path: RepoPath,
    /// How it changed.
    pub change: Change,
    /// What the change did to its lines.
    pub lines: Lines,
}

/// One commit of the round with the files it changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFacts {
    /// The commit.
    pub id: CommitId,
    /// Its parents; none for a commit with no history.
    pub parents: Vec<CommitId>,
    /// The first line of its message.
    pub subject: String,
    /// The rest of its message.
    pub body: String,
    /// The files it changed against its first parent; none for a merge. A rename is a deletion and an addition here.
    pub files: Vec<FileChange>,
}

/// One line a round added to a file, for the markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedLine {
    /// The file.
    pub path: RepoPath,
    /// The line's number in the file at the head, counted from 1.
    pub number: u32,
    /// The line's text, without its line break.
    pub text: String,
}

/// What the checks look at: the round's diff and its commits.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoundFacts {
    /// The files that differ between the merge base and the head.
    pub files: Vec<FileChange>,
    /// The commits from the merge base to the head, oldest first: a commit comes after its parents.
    pub commits: Vec<CommitFacts>,
    /// The lines the round added to the files whose extension has markers ([`markers::scans`]).
    pub added: Vec<AddedLine>,
    /// The description of the pull request; empty when there is none yet.
    pub description: String,
}

/// What the checks found wrong with a round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// An existing test file was changed other than by added lines.
    TestChanged {
        /// The file as it was at the merge base.
        path: RepoPath,
        /// What was done to it.
        how: TestChange,
    },
    /// A protected file was changed.
    ProtectedChanged {
        /// The file; where it was, when it moved.
        path: RepoPath,
        /// What was done to it.
        change: Touch,
    },
    /// A commit removed or replaced lines of a test file that a `RED:` commit of the round touched.
    RedTestEdited {
        /// The test file.
        path: RepoPath,
        /// The `RED:` commit that touched it.
        red: CommitId,
        /// The later commit that took lines out of it.
        edit: CommitId,
    },
    /// The round changes production code and adds no line to a test file.
    NoTests,
    /// Added lines that skip a test, suppress a lint or ask whether the code runs under test.
    Marker {
        /// The file.
        path: RepoPath,
        /// Which kind of marker.
        marker: markers::Marker,
        /// The first line that has one.
        line: u32,
        /// How many lines of the file have one.
        count: u32,
    },
}

/// What was done to an existing test file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestChange {
    /// It was deleted.
    Deleted,
    /// It was renamed.
    Renamed {
        /// Where it is now.
        to: RepoPath,
    },
    /// Lines were removed or replaced.
    Edited {
        /// How many lines were taken out.
        removed: u32,
    },
    /// A binary file was replaced.
    EditedBinary,
}

/// What was done to a protected file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Touch {
    /// It was added.
    Added,
    /// Its contents changed.
    Modified,
    /// It was deleted.
    Deleted,
    /// It was renamed.
    Renamed,
}

/// Who can allow an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judge {
    /// The team can: it fixes the round, or says why in the commits or the description.
    Team,
    /// Only the human can.
    Human,
}

/// What an issue is about: what the human allows or refuses, and what is recorded for the item when they allow it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subject {
    /// A file.
    Path(RepoPath),
    /// That the round has no tests for the code it changes.
    NoTests,
}

impl Issue {
    /// Who can allow the issue.
    #[must_use]
    pub const fn judge(&self) -> Judge {
        Judge::Team
    }

    /// What the issue is about.
    #[must_use]
    pub const fn subject(&self) -> Subject {
        Subject::NoTests
    }
}

/// The issues the checks find in `facts`, under the lists of `rules`, without those about a `Subject` in `allowed`.
///
/// The order is fixed: protected files, changed tests, edited `RED:` tests, missing tests, markers; within a rule, the
/// order of the facts.
#[must_use]
pub fn assess(_rules: &Checks, _facts: &RoundFacts, _allowed: &[Subject]) -> Vec<Issue> {
    Vec::new()
}

#[cfg(test)]
mod tests;
