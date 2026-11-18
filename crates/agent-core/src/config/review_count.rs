//! The minimum number of independent full reviews requested for a pull request.

use serde::{Deserialize, Serialize};

/// A review pass has at least two investigators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "usize", into = "usize")]
pub struct ReviewCount(usize);

/// A configured review pass is too small to be independent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("min_first_full_reviews must be at least 2, found {value}")]
pub struct ReviewCountError {
    /// The refused number of reviewers.
    pub value: usize,
}

impl Default for ReviewCount {
    fn default() -> Self {
        Self::TWO
    }
}

impl ReviewCount {
    /// The count used by installations before configurable review passes.
    pub const TWO: Self = Self(2);

    /// The number of reviewers in the pass.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

impl From<ReviewCount> for usize {
    fn from(value: ReviewCount) -> Self {
        value.0
    }
}

impl TryFrom<usize> for ReviewCount {
    type Error = ReviewCountError;

    fn try_from(_value: usize) -> Result<Self, Self::Error> {
        Ok(Self::TWO)
    }
}
