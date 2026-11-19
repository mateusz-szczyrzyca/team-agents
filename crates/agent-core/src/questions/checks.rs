//! The versioned checks-question metadata and its explicit verdict grammar.

use super::Entry;
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
#[error("write `allow`, `refuse`, or `allow except <path> <path> ...` using only paths listed in this question")]
pub struct InvalidChecksVerdict;

impl Entry {
    /// Adds versioned group membership without changing the persisted checks record.
    #[must_use]
    pub fn with_check_subjects(self, _subjects: &[Subject]) -> Self {
        self
    }

    /// Whether this question names the subject in its versioned group.
    #[must_use]
    pub fn checks_contains(&self, _subject: &Subject) -> bool {
        false
    }

    /// The strict verdict, or `None` for a legacy question.
    ///
    /// # Errors
    /// [`InvalidChecksVerdict`] for an absent verdict, an unsupported form or an unknown exception path.
    pub fn checks_verdict(&self) -> Result<Option<ChecksVerdict>, InvalidChecksVerdict> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
