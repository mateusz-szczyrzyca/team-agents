//! Judgement of steps in a plan: finding which steps are to be judged, check refusal rules, and judgement states.

use serde::{Deserialize, Serialize};

use super::{Plan, Step};
use crate::ids::ItemId;

#[cfg(test)]
mod tests;

/// The outcome of judging a step of a plan that is marked done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgementState {
    /// The step names a check and the check passed.
    Confirmed,
    /// The step has no check, its check is `reviewer`, check execution is disabled, or its check is refused.
    Declared,
    /// The step is marked done and its check fails.
    Mismatch,
}

impl JudgementState {
    /// The state as a short string.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Declared => "declared",
            Self::Mismatch => "mismatch",
        }
    }
}

/// Whether `command` is refused as a step's check because it starts with a refused gate prefix.
///
/// Compared after trimming, as a prefix of whole words.
#[must_use]
pub fn refuse_command(_command: &str, _refused_prefixes: &[String]) -> bool {
    false
}

/// Finds the steps of `current` that are to be judged: steps marked done that were not done in `last_plan`
/// (or whose text changed since `last_plan`, or that were `mismatch` according to `last_state`).
///
/// Steps not marked done are never judged.
#[must_use]
pub fn steps_to_judge<'a>(
    _current: &'a Plan,
    _last_plan: Option<&Plan>,
    _last_state: impl Fn(&ItemId) -> Option<JudgementState>,
) -> Vec<&'a Step> {
    Vec::new()
}
