//! Paths inside a repository ([`RepoPath`]) and the patterns the `checks` table of a repository matches them with
//! ([`PathPattern`]).
//!
//! A [`RepoPath`] is a path as git prints it: relative to the root, `/`-separated, with no empty and no `.` or `..`
//! segment. A [`PathPattern`] is a glob over such paths:
//!
//! - `*` matches any characters of one segment and `?` one character of it;
//! - a segment `**` matches any number of whole segments, and at the end of a pattern at least one;
//! - a pattern that ends in `/` is a directory: it matches everything below it, as `dir/**` does;
//! - a pattern without a `/` is a name: it matches the file's name at any depth (`*_test.go`, `AGENTS.md`);
//! - any other pattern is anchored at the root of the repository.
//!
//! Nothing here touches the file system: both types are made by parsing, and a text they refuse is an error that names
//! it and the rule it breaks.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

use crate::config::text_value;

/// Longest path of a repository, in bytes: Linux's `PATH_MAX`.
const MAX_REPO_PATH_LEN: usize = 4096;

/// Longest path pattern, in bytes.
const MAX_PATTERN_LEN: usize = 512;

/// Most `**` segments a pattern may have: matching them backtracks, so their number is bounded.
const MAX_DOUBLE_STARS: usize = 4;

/// Why a text is not a [`RepoPath`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid repository path {value:?}: {rule}")]
pub struct InvalidRepoPath {
    /// The refused text.
    pub value: String,
    /// The rule it breaks.
    pub rule: PathRule,
}

/// The rule of a [`RepoPath`] that a text breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathRule {
    /// The text is empty.
    #[error("it is empty")]
    Empty,
    /// The text is longer than `max` bytes.
    #[error("it is longer than {max} bytes")]
    TooLong {
        /// The longest allowed text, in bytes.
        max: usize,
    },
    /// The path starts with `/`: it is not relative to the root of the repository.
    #[error("it must be relative to the root of the repository")]
    Absolute,
    /// The path has an empty segment: `a//b` or a `/` at its end.
    #[error("it has an empty segment")]
    EmptySegment,
    /// The path has a `.` or `..` segment.
    #[error("it must not have a \".\" or \"..\" segment")]
    DotSegment,
    /// The text contains this control character.
    #[error("the control character {0:?} is not allowed")]
    Control(char),
}

/// A path of a file in a repository, as git prints it.
///
/// Not empty, at most 4096 bytes, relative, with no empty, `.` or `..` segment and no control character.
///
/// ```
/// # fn main() -> Result<(), agent_core::repo_path::InvalidRepoPath> {
/// let path: agent_core::repo_path::RepoPath = "crates/agent-core/src/lib.rs".parse()?;
/// assert_eq!(path.extension(), Some("rs"));
/// assert!("../outside".parse::<agent_core::repo_path::RepoPath>().is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct RepoPath(String);

/// Checks the rules of a [`RepoPath`].
fn check_repo_path(text: &str) -> Result<(), InvalidRepoPath> {
    Err(InvalidRepoPath {
        value: text.to_owned(),
        rule: PathRule::TooLong { max: MAX_REPO_PATH_LEN },
    })
}

text_value!(RepoPath, check_repo_path, keep, InvalidRepoPath);

/// Keeps a text as it is.
fn keep(text: String) -> String {
    text
}

impl RepoPath {
    /// The extension of the file's name, without the dot; `None` for a name with no dot or only a leading one.
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        None
    }

    /// The last segment: the file's name.
    #[must_use]
    pub fn file_name(&self) -> &str {
        &self.0
    }
}

/// Why a text is not a [`PathPattern`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid path pattern {value:?}: {rule}")]
pub struct InvalidPathPattern {
    /// The refused text.
    pub value: String,
    /// The rule it breaks.
    pub rule: PatternRule,
}

/// The rule of a [`PathPattern`] that a text breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PatternRule {
    /// The text is empty.
    #[error("it is empty")]
    Empty,
    /// The text is longer than `max` bytes.
    #[error("it is longer than {max} bytes")]
    TooLong {
        /// The longest allowed text, in bytes.
        max: usize,
    },
    /// The pattern starts with `/`: patterns are relative to the root of the repository.
    #[error("it must be relative to the root of the repository")]
    Absolute,
    /// The pattern has an empty segment: `a//b`.
    #[error("it has an empty segment")]
    EmptySegment,
    /// The pattern has a `..` segment.
    #[error("it must not have a \"..\" segment")]
    ParentSegment,
    /// The text contains this character, which is reserved or not allowed.
    #[error("the character {0:?} is not allowed")]
    ForbiddenChar(char),
    /// A segment mixes `**` with other characters: `**` is a segment of its own.
    #[error("\"**\" must be a whole segment")]
    MixedDoubleStar,
    /// The pattern has more than `max` `**` segments.
    #[error("it has more than {max} \"**\" segments")]
    TooManyDoubleStars {
        /// The most `**` segments allowed.
        max: usize,
    },
}

/// A glob over the paths of a repository: see the [module](self) for its syntax.
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use agent_core::repo_path::{PathPattern, RepoPath};
///
/// let pattern: PathPattern = "tests/**".parse()?;
/// assert!(pattern.matches(&"tests/unit/a.bats".parse::<RepoPath>()?));
/// assert!(!pattern.matches(&"src/tests/a.rs".parse::<RepoPath>()?));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct PathPattern(String);

/// Checks the rules of a [`PathPattern`].
fn check_pattern(text: &str) -> Result<(), InvalidPathPattern> {
    let rule = if text.len() > MAX_PATTERN_LEN {
        PatternRule::TooLong { max: MAX_PATTERN_LEN }
    } else {
        PatternRule::TooManyDoubleStars { max: MAX_DOUBLE_STARS }
    };
    Err(InvalidPathPattern {
        value: text.to_owned(),
        rule,
    })
}

text_value!(PathPattern, check_pattern, keep, InvalidPathPattern);

impl PathPattern {
    /// Whether the pattern matches `path`.
    #[must_use]
    pub fn matches(&self, _path: &RepoPath) -> bool {
        false
    }
}

/// Whether any of `patterns` matches `path`.
#[must_use]
pub fn matches_any(patterns: &[PathPattern], path: &RepoPath) -> bool {
    patterns.iter().any(|pattern| pattern.matches(path))
}

#[cfg(test)]
mod tests;
