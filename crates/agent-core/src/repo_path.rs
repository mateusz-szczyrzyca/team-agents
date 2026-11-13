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
    let refuse = |rule| InvalidRepoPath {
        value: text.to_owned(),
        rule,
    };
    if text.is_empty() {
        return Err(refuse(PathRule::Empty));
    }
    if text.len() > MAX_REPO_PATH_LEN {
        return Err(refuse(PathRule::TooLong { max: MAX_REPO_PATH_LEN }));
    }
    if text.starts_with('/') {
        return Err(refuse(PathRule::Absolute));
    }
    if let Some(control) = text.chars().find(|c| c.is_control()) {
        return Err(refuse(PathRule::Control(control)));
    }
    for segment in text.split('/') {
        match segment {
            "" => return Err(refuse(PathRule::EmptySegment)),
            "." | ".." => return Err(refuse(PathRule::DotSegment)),
            _ => {}
        }
    }
    Ok(())
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
        let (stem, extension) = self.file_name().rsplit_once('.')?;
        (!stem.is_empty() && !extension.is_empty()).then_some(extension)
    }

    /// The last segment: the file's name.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.0.rsplit_once('/').map_or(self.0.as_str(), |(_, name)| name)
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
    let refuse = |rule| InvalidPathPattern {
        value: text.to_owned(),
        rule,
    };
    if text.is_empty() {
        return Err(refuse(PatternRule::Empty));
    }
    if text.len() > MAX_PATTERN_LEN {
        return Err(refuse(PatternRule::TooLong { max: MAX_PATTERN_LEN }));
    }
    if text.starts_with('/') {
        return Err(refuse(PatternRule::Absolute));
    }
    // `[`, `]`, `{` and `}` are reserved for classes and alternatives, which a later version may add.
    if let Some(bad) = text
        .chars()
        .find(|c| c.is_control() || matches!(c, '\\' | '[' | ']' | '{' | '}'))
    {
        return Err(refuse(PatternRule::ForbiddenChar(bad)));
    }
    let segments: Vec<&str> = text.split('/').collect();
    // A directory pattern ends in `/`: only its last segment is empty.
    let last = segments.len().saturating_sub(1);
    let mut double_stars = 0_usize;
    for (position, segment) in segments.iter().enumerate() {
        match *segment {
            "" if position == last => {}
            "" => return Err(refuse(PatternRule::EmptySegment)),
            ".." => return Err(refuse(PatternRule::ParentSegment)),
            "**" => double_stars = double_stars.saturating_add(1),
            other if other.contains("**") => return Err(refuse(PatternRule::MixedDoubleStar)),
            _ => {}
        }
    }
    if double_stars > MAX_DOUBLE_STARS {
        return Err(refuse(PatternRule::TooManyDoubleStars { max: MAX_DOUBLE_STARS }));
    }
    Ok(())
}

text_value!(PathPattern, check_pattern, keep, InvalidPathPattern);

impl PathPattern {
    /// Whether the pattern matches `path`.
    #[must_use]
    pub fn matches(&self, path: &RepoPath) -> bool {
        if !self.0.contains('/') {
            return glob_segment(&self.0, path.file_name());
        }
        let mut pattern: Vec<&str> = self.0.split('/').collect();
        // A directory matches everything below it.
        if let Some(last) = pattern.last_mut()
            && last.is_empty()
        {
            *last = "**";
        }
        let segments: Vec<&str> = path.as_str().split('/').collect();
        match_segments(&pattern, &segments)
    }
}

/// Whether the segments of a pattern match the segments of a path; `**` takes any number of segments, and at the end of
/// the pattern at least one.
fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", [])) => !path.is_empty(),
        Some((&"**", rest)) => {
            (0..=path.len()).any(|skipped| path.get(skipped..).is_some_and(|tail| match_segments(rest, tail)))
        }
        Some((segment, rest)) => path
            .split_first()
            .is_some_and(|(name, tail)| glob_segment(segment, name) && match_segments(rest, tail)),
    }
}

/// Whether `text`, one segment, matches the glob `pattern`: `*` for any characters and `?` for one.
fn glob_segment(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut at_pattern, mut at_text) = (0_usize, 0_usize);
    // Where the last `*` was and how much of the text it has taken: the place to go back to when a match fails.
    let mut star: Option<(usize, usize)> = None;
    while at_text < text.len() {
        match (pattern.get(at_pattern), text.get(at_text)) {
            (Some('*'), _) => {
                star = Some((at_pattern, at_text));
                at_pattern = at_pattern.saturating_add(1);
            }
            (Some('?'), _) => {
                at_pattern = at_pattern.saturating_add(1);
                at_text = at_text.saturating_add(1);
            }
            (Some(wanted), Some(found)) if wanted == found => {
                at_pattern = at_pattern.saturating_add(1);
                at_text = at_text.saturating_add(1);
            }
            _ => {
                let Some((star_at, taken)) = star else {
                    return false;
                };
                let taken = taken.saturating_add(1);
                star = Some((star_at, taken));
                at_pattern = star_at.saturating_add(1);
                at_text = taken;
            }
        }
    }
    pattern
        .get(at_pattern..)
        .is_some_and(|rest| rest.iter().all(|c| *c == '*'))
}

/// Whether any of `patterns` matches `path`.
#[must_use]
pub fn matches_any(patterns: &[PathPattern], path: &RepoPath) -> bool {
    patterns.iter().any(|pattern| pattern.matches(path))
}

/// Most patterns a list of a configuration may have.
pub const MAX_PATTERNS: usize = 256;

/// A list of [`PathPattern`]s, at most [`MAX_PATTERNS`] of them: what one of the lists of a repository's `checks`
/// table is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(try_from = "Vec<PathPattern>")]
pub struct PathPatterns(Vec<PathPattern>);

/// A list with more patterns than [`MAX_PATTERNS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a list of {len} path patterns is longer than the {MAX_PATTERNS} allowed")]
pub struct TooManyPatterns {
    /// How many patterns the list had.
    pub len: usize,
}

impl TryFrom<Vec<PathPattern>> for PathPatterns {
    type Error = TooManyPatterns;

    fn try_from(patterns: Vec<PathPattern>) -> Result<Self, Self::Error> {
        Ok(Self(patterns))
    }
}

impl PathPatterns {
    /// The patterns, in the order written.
    #[must_use]
    pub fn as_slice(&self) -> &[PathPattern] {
        &self.0
    }

    /// Whether any pattern of the list matches `path`.
    #[must_use]
    pub fn matches(&self, path: &RepoPath) -> bool {
        matches_any(&self.0, path)
    }
}

#[cfg(test)]
mod tests;
