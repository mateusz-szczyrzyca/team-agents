//! The versioned checks-question metadata and its explicit verdict grammar.

use super::{Entry, Part, Verdict};
use crate::decide::checks::Subject;

/// A strict verdict for a group of findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChecksVerdict {
    /// Allow every subject of this question.
    Allow,
    /// Refuse every subject of this question.
    Refuse,
    /// Allow the group except these exact paths.
    Except(Vec<String>),
}

/// A checks verdict must name one of the supported forms and only paths of its group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{}", INVALID_VERDICT.trim())]
pub struct InvalidChecksVerdict;

/// The shared human-facing correction for a malformed checks verdict.
pub(super) const INVALID_VERDICT: &str = include_str!("../../../../prompts/harness/checks/question-invalid-verdict.md");

impl Entry {
    /// Adds versioned group membership without changing the persisted checks record.
    #[must_use]
    pub fn with_check_subjects(mut self, subjects: &[Subject]) -> Self {
        self.parts
            .retain(|part| !matches!(part, Part::Other { label, .. } if label == GROUP_LABEL));
        self.parts.insert(
            0,
            Part::Other {
                label: GROUP_LABEL.to_owned(),
                text: subjects.iter().map(subject_key).collect::<Vec<_>>().join("\n"),
            },
        );
        self
    }

    /// Whether this question names the subject in its versioned group.
    #[must_use]
    pub fn checks_contains(&self, subject: &Subject) -> bool {
        self.check_group()
            .is_some_and(|group| group.lines().any(|line| line == subject_key(subject)))
    }

    /// The metadata is a labelled part, preserved by old parsers without adding a checks-record field.
    fn check_group(&self) -> Option<&str> {
        self.parts.iter().find_map(|part| match part {
            Part::Other { label, text } if label == GROUP_LABEL => Some(text.as_str()),
            Part::Recommended(_) | Part::Verdict(_) | Part::Other { .. } => None,
        })
    }

    /// The strict verdict, or `None` for a legacy question.
    ///
    /// # Errors
    /// [`InvalidChecksVerdict`] for an absent verdict, an unsupported form or an unknown exception path.
    pub fn checks_verdict(&self) -> Result<Option<ChecksVerdict>, InvalidChecksVerdict> {
        let Some(group) = self.check_group() else {
            return Ok(None);
        };
        let Some(Verdict::Own(answer)) = self.verdict() else {
            return Err(InvalidChecksVerdict);
        };
        let answer = answer.as_str();
        match answer {
            "allow" => Ok(Some(ChecksVerdict::Allow)),
            "refuse" => Ok(Some(ChecksVerdict::Refuse)),
            _ => {
                let rest = answer.strip_prefix("allow except ").ok_or(InvalidChecksVerdict)?;
                let paths: Vec<&str> = group
                    .lines()
                    .filter_map(|line| line.strip_prefix("p:").and_then(|path| path.strip_suffix(':')))
                    .collect();
                let mut rest = rest.trim();
                let mut exceptions = Vec::new();
                while !rest.is_empty() {
                    // Prefer the longest complete path, so spaces inside a repository path stay meaningful.
                    let path = paths
                        .iter()
                        .filter(|path| {
                            rest.strip_prefix(**path)
                                .is_some_and(|after| after.is_empty() || after.starts_with(char::is_whitespace))
                        })
                        .max_by_key(|path| path.len())
                        .ok_or(InvalidChecksVerdict)?;
                    exceptions.push((*path).to_owned());
                    rest = rest.strip_prefix(*path).ok_or(InvalidChecksVerdict)?.trim_start();
                }
                if exceptions.is_empty() {
                    return Err(InvalidChecksVerdict);
                }
                Ok(Some(ChecksVerdict::Except(exceptions)))
            }
        }
    }
}

/// The version distinguishes grouped questions from the per-file legacy grammar.
const GROUP_LABEL: &str = "Checks version one";

/// Delimiters preserve spaces in paths when a labelled part is trimmed by the Markdown parser.
fn subject_key(subject: &Subject) -> String {
    match subject {
        Subject::Path(path) => format!("p:{path}:"),
        Subject::NoTests => "n:".to_owned(),
    }
}

#[cfg(test)]
mod tests;
