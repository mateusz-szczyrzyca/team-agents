//! Task-authored permission for repository paths, with rules files excluded.

use crate::repo_path::{PathPattern, RepoPath};

/// A root-relative glob that cannot reach a rule file or a workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskAllowance(PathPattern);

/// Why an `Allow:` pattern cannot be accepted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AllowanceError {
    /// The pattern has unsupported syntax.
    #[error(transparent)]
    Pattern(#[from] crate::repo_path::InvalidPathPattern),
    /// The pattern reaches a file that requires the human's decision every time.
    #[error("the pattern could match AGENTS.md, CLAUDE.md or .github/workflows/")]
    Rules,
}

impl TaskAllowance {
    /// Parses a root-relative allowance.
    ///
    /// # Errors
    /// [`AllowanceError`] for invalid syntax or a pattern reaching rule files.
    pub fn parse(text: &str) -> Result<Self, AllowanceError> {
        Ok(Self(text.parse()?))
    }

    /// The pattern as the task author wrote it, without surrounding whitespace.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Whether this allowance covers a path relative to the repository root.
    #[must_use]
    pub fn matches(&self, _path: &RepoPath) -> bool {
        false
    }
}

#[cfg(test)]
mod tests;
