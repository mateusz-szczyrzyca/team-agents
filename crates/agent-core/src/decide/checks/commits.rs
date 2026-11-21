//! The findings of the checks that name a commit already in the branch.
//!
//! A frozen test that a commit took lines out of cannot be repaired by the team: the commit is in the branch for good,
//! and published history is never rewritten. Only the human may allow such a finding, so the round is not sent back to
//! the team for it; what they say is kept by the commit and the path for the item until it is closed, and the same
//! finding is neither asked nor sent back again. A new commit, or a new path, is a new finding.

use std::collections::{BTreeMap, HashSet};

use super::{Issue, Lines, RoundFacts, Subject};
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
    let mut found = Findings::default();
    for issue in issues {
        match issue {
            Issue::RedTestEdited { path, red, edit } => found.commits.push(CommitFinding {
                commit: edit,
                path,
                red,
            }),
            other @ (Issue::TestChanged { .. }
            | Issue::ProtectedChanged { .. }
            | Issue::NoTests
            | Issue::Marker { .. }) => found.rest.push(other),
        }
    }
    found
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
pub fn settle(findings: Vec<CommitFinding>, facts: &RoundFacts, said: &Said<'_>) -> Settled {
    let mut settled = Settled::default();
    for finding in findings {
        let subject = finding.subject();
        if said.allowed.contains(&subject)
            || said.refused.contains(&subject)
            || said.task_list.iter().any(|allowance| allowance.matches(&finding.path))
        {
            continue;
        }
        let covered = said.paths.get(&finding.path).is_some_and(|head| match head {
            Some(head) => reaches(facts, head, &finding.commit),
            None => true,
        });
        if covered {
            settled.inherited.push(subject);
        } else {
            settled.ask.push(finding);
        }
    }
    settled
}

/// Whether `head` is `commit` or comes after it in the history of the round.
fn reaches(facts: &RoundFacts, head: &CommitId, commit: &CommitId) -> bool {
    let mut seen: HashSet<&CommitId> = HashSet::new();
    let mut pending = vec![head];
    while let Some(next) = pending.pop() {
        if next == commit {
            return true;
        }
        if !seen.insert(next) {
            continue;
        }
        if let Some(known) = facts.commits.iter().find(|known| known.id == *next) {
            pending.extend(known.parents.iter());
        }
    }
    false
}

/// What the human is shown about a [`CommitFinding`] besides its commit and its path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitNote {
    /// The finding.
    pub finding: CommitFinding,
    /// The first line of the commit's message.
    pub subject: String,
    /// The name of the commit's author.
    pub author: String,
    /// What the commit did to the lines of the test.
    pub lines: Lines,
}

/// Why a finding cannot be described.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoteError {
    /// The commit is not among the commits of the round.
    #[error("commit {commit} is not a commit of the round")]
    NoCommit {
        /// The commit.
        commit: CommitId,
    },
    /// The commit did not change the test.
    #[error("commit {commit} did not change {path}")]
    NoFile {
        /// The commit.
        commit: CommitId,
        /// The test.
        path: RepoPath,
    },
    /// Nobody is known to have written the commit.
    #[error("the author of commit {commit} is not known")]
    NoAuthor {
        /// The commit.
        commit: CommitId,
    },
}

/// Describes `finding` from the commits of the round and the `authors` git named.
///
/// # Errors
/// [`NoteError`] when the commit is not in the round, did not change the test, or has no known author.
pub fn describe(
    finding: &CommitFinding,
    facts: &RoundFacts,
    authors: &BTreeMap<CommitId, String>,
) -> Result<CommitNote, NoteError> {
    let commit = facts
        .commits
        .iter()
        .find(|known| known.id == finding.commit)
        .ok_or_else(|| NoteError::NoCommit {
            commit: finding.commit.clone(),
        })?;
    // A merge has no files of its own: what it changed against its first parent is what its resolution did to the test.
    let files = if commit.parents.len() > 1 {
        commit.merge_files.first().map_or(&[][..], Vec::as_slice)
    } else {
        commit.files.as_slice()
    };
    let changed = files
        .iter()
        .find(|file| file.path == finding.path)
        .ok_or_else(|| NoteError::NoFile {
            commit: finding.commit.clone(),
            path: finding.path.clone(),
        })?;
    let author = authors.get(&finding.commit).ok_or_else(|| NoteError::NoAuthor {
        commit: finding.commit.clone(),
    })?;
    Ok(CommitNote {
        finding: finding.clone(),
        subject: commit.subject.clone(),
        author: author.clone(),
        lines: changed.lines,
    })
}

#[cfg(test)]
mod tests;
