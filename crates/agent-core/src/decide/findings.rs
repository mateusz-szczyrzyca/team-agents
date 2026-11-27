//! Pure matching and presentation of open repository findings for a run.

use crate::{ids::TaskId, local_time::Zone, state::Timestamp};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt, num::NonZeroU64, str::FromStr};

/// What another agent learned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FindingKind {
    /// A test fails without a cause in the code.
    FlakyTest,
    /// Code or documentation misleads its reader.
    Trap,
    /// A tool, toolchain or machine behaves unexpectedly.
    Environment,
}

/// A positive bus sequence number, ordered numerically rather than lexically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FindingId(NonZeroU64);

/// A finding identifier is not `F-` followed by a positive decimal u64.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid finding id {value:?}: expected F-<positive number>")]
pub struct InvalidFindingId {
    /// The rejected identifier.
    value: String,
}

impl FromStr for FindingId {
    type Err = InvalidFindingId;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Err(InvalidFindingId {
            value: value.to_owned(),
        })
    }
}
impl TryFrom<String> for FindingId {
    type Error = InvalidFindingId;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl fmt::Display for FindingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "F-{}", self.0)
    }
}
impl From<FindingId> for String {
    fn from(value: FindingId) -> Self {
        value.to_string()
    }
}

/// A normalized relative repository path, without parent traversal or whitespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RepoPath(String);

/// A path is empty, absolute, contains parent traversal or whitespace, or exceeds 200 characters.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid finding path {value:?}")]
pub struct InvalidRepoPath {
    /// The rejected path.
    value: String,
}
impl RepoPath {
    /// Normalizes a relative path, removing `.` and empty components.
    ///
    /// # Errors
    /// Returns [`InvalidRepoPath`] for absolute, empty, traversing, whitespace-containing or overlong paths.
    pub fn parse(value: &str) -> Result<Self, InvalidRepoPath> {
        Err(InvalidRepoPath {
            value: value.to_owned(),
        })
    }
    /// The normalized path, with `/` separators.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for RepoPath {
    type Error = InvalidRepoPath;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}
impl From<RepoPath> for String {
    fn from(value: RepoPath) -> Self {
        value.0
    }
}

/// An open finding returned by the bus; resolution records remain the bus's responsibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// The bus's stable sequence identifier.
    pub id: FindingId,
    /// When the finding was added; the bus calls this field `ts`.
    #[serde(rename = "ts")]
    pub added_at: Timestamp,
    /// The agent that added it.
    pub author: String,
    /// The task that added it.
    pub task: TaskId,
    /// The kind of lesson.
    pub kind: FindingKind,
    /// The affected files or directories; empty only for repository-wide environment findings.
    pub paths: Vec<RepoPath>,
    /// The lesson, as written by the author.
    pub text: String,
}

/// One borrowed finding and its best path match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selected<'a> {
    /// The open finding chosen for the run.
    pub finding: &'a Finding,
    /// The highest matching score, from 1 through 3.
    pub score: u8,
}

/// Extracts backtick paths, excluding commands, URLs, absolute paths and parent traversal.
/// The daemon checks which of these paths exist in its worktree.
#[must_use]
pub fn item_paths(_text: &str) -> Vec<RepoPath> {
    Vec::new()
}

/// The best exact (3), ancestor (2) or sufficiently deep sibling (1) match; otherwise 0.
#[must_use]
pub fn score(_finding: &Finding, _run_paths: &[RepoPath]) -> u8 {
    0
}

/// Chooses from **open findings**, as returned by `agentbus findings --repo … --json`.
/// Excludes the task's own and already shown findings, then orders by score, recency and numeric ID.
/// A zero retention disables selection; budgets remove findings from the end without truncating text.
#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "the pure selector takes the inputs specified by the findings contract"
)]
pub fn select_findings<'a>(
    _findings: &'a [Finding],
    _run_paths: &[RepoPath],
    _shown: &BTreeSet<FindingId>,
    _own_task: &TaskId,
    _now: Timestamp,
    _keep_days: u16,
    _max_shown: usize,
    _max_chars: usize,
) -> Vec<Selected<'a>> {
    Vec::new()
}

/// Renders the section's lines, using the date in `zone`; no findings produces an empty string.
#[must_use]
pub fn render_findings(_selected: &[Selected<'_>], _zone: Zone) -> String {
    String::new()
}

#[cfg(test)]
mod tests;
