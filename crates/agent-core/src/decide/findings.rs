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
        let invalid = || InvalidFindingId {
            value: value.to_owned(),
        };
        let digits = value.strip_prefix("F-").ok_or_else(invalid)?;
        let number = digits.chars().try_fold(0_u64, |number, digit| {
            let digit = digit.to_digit(10).filter(|_| digit.is_ascii()).ok_or_else(invalid)?;
            number
                .checked_mul(10)
                .and_then(|n| n.checked_add(u64::from(digit)))
                .ok_or_else(invalid)
        })?;
        NonZeroU64::new(number).map(Self).ok_or_else(invalid)
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

/// The bus path limit in Unicode characters.
const MAX_PATH_CHARS: usize = 200;

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
        let invalid = || InvalidRepoPath {
            value: value.to_owned(),
        };
        if value.chars().count() > MAX_PATH_CHARS || value.chars().any(char::is_whitespace) {
            return Err(invalid());
        }
        let normalized = value.replace('\\', "/");
        if normalized.starts_with('/')
            || normalized.as_bytes().get(1) == Some(&b':')
            || normalized.split('/').any(|part| part == "..")
        {
            return Err(invalid());
        }
        let normalized = normalized
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect::<Vec<_>>()
            .join("/");
        if normalized.is_empty() {
            return Err(invalid());
        }
        Ok(Self(normalized))
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
pub fn item_paths(text: &str) -> Vec<RepoPath> {
    let mut paths = Vec::new();
    let mut tokens = text.split('`').skip(1);
    while let Some(token) = tokens.next() {
        // An unmatched opening backtick does not name a token.
        if tokens.next().is_none() {
            break;
        }
        if !token.contains('/')
            || token.starts_with(['/', '-'])
            || token.starts_with("http")
            || token.chars().any(char::is_whitespace)
        {
            continue;
        }
        let token = token.trim_end_matches(['.', ',', ';', ':', ')']);
        let token = match token.rsplit_once(':') {
            Some((path, line)) if !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit()) => path,
            Some(_) | None => token,
        };
        let path = match RepoPath::parse(token) {
            Ok(path) => path,
            // Invalid tokens are deliberately excluded; the item text also contains commands and prose.
            Err(InvalidRepoPath { .. }) => continue,
        };
        paths.push(path);
    }
    paths
}

/// The best exact (3), ancestor (2) or sufficiently deep sibling (1) match; otherwise 0.
#[must_use]
pub fn score(finding: &Finding, run_paths: &[RepoPath]) -> u8 {
    if finding.kind == FindingKind::Environment && finding.paths.is_empty() {
        return 1;
    }
    finding
        .paths
        .iter()
        .flat_map(|p| run_paths.iter().map(move |t| path_score(p.as_str(), t.as_str())))
        .max()
        .unwrap_or(0)
}

/// Scores one normalized path pair, respecting component boundaries.
fn path_score(p: &str, t: &str) -> u8 {
    if p == t {
        return 3;
    }
    let contains = |parent: &str, child: &str| child.strip_prefix(parent).is_some_and(|rest| rest.starts_with('/'));
    if contains(p, t) || contains(t, p) {
        return 2;
    }
    match (p.rsplit_once('/'), t.rsplit_once('/')) {
        (Some((parent, _)), Some((other, _))) if parent == other && parent.split('/').count() >= 3 => 1,
        _ => 0,
    }
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
    findings: &'a [Finding],
    run_paths: &[RepoPath],
    shown: &BTreeSet<FindingId>,
    own_task: &TaskId,
    now: Timestamp,
    keep_days: u16,
    max_shown: usize,
    max_chars: usize,
) -> Vec<Selected<'a>> {
    if keep_days == 0 {
        return Vec::new();
    }
    let retention = time::Duration::days(i64::from(keep_days));
    let mut selected: Vec<_> = findings
        .iter()
        .filter(|f| f.task != *own_task && !shown.contains(&f.id) && now.as_utc() - f.added_at.as_utc() <= retention)
        .map(|finding| Selected {
            finding,
            score: score(finding, run_paths),
        })
        .filter(|s| s.score >= 1)
        .collect();
    selected.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.finding.added_at.cmp(&a.finding.added_at))
            .then_with(|| b.finding.id.cmp(&a.finding.id))
    });
    selected.truncate(max_shown);
    let mut remaining = max_chars;
    let fitting = selected
        .iter()
        .take_while(|s| match remaining.checked_sub(s.finding.text.chars().count()) {
            Some(rest) => {
                remaining = rest;
                true
            }
            None => false,
        })
        .count();
    selected.truncate(fitting);
    selected
}

/// Renders the section's lines, using the date in `zone`; no findings produces an empty string.
#[must_use]
pub fn render_findings(selected: &[Selected<'_>], zone: Zone) -> String {
    selected
        .iter()
        .map(|s| {
            let f = s.finding;
            let paths = if f.paths.is_empty() {
                "the repository".to_owned()
            } else {
                f.paths.iter().map(RepoPath::as_str).collect::<Vec<_>>().join(", ")
            };
            let local = zone.local_minute(f.added_at);
            let date = local.split_once(' ').map_or(local.as_str(), |(date, _)| date);
            format!("- {} [{}] {paths} — {} ({}, {date})", f.id, f.kind, f.text, f.author)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests;
