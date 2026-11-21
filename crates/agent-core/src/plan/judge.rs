//! Judgement of steps in a plan: finding which steps are to be judged, check refusal rules, and judgement states.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{Plan, Step, StepState};
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
pub fn refuse_command(command: &str, refused_prefixes: &[String]) -> bool {
    refused_prefixes.iter().any(|prefix| {
        let mut words = command.split_whitespace();
        let mut prefix_words = prefix.split_whitespace().peekable();
        prefix_words.peek().is_some() && prefix_words.all(|word| words.next() == Some(word))
    })
}

/// Finds the steps of `current` that are to be judged: steps marked done that were not done in `last_plan`
/// (or whose text changed since `last_plan`, or that were `mismatch` according to `last_state`).
///
/// Steps not marked done are never judged.
#[must_use]
pub fn steps_to_judge<'a, 'b>(
    current: &'a Plan,
    last_plan: Option<&'b Plan>,
    last_state: impl Fn(&ItemId) -> Option<JudgementState>,
) -> Vec<&'a Step> {
    let old_steps: HashMap<ItemId, &'b Step> = match last_plan {
        Some(plan) => plan.steps().map(|step| (step.id(), step)).collect(),
        None => HashMap::new(),
    };
    steps_to_judge_with(current, |id| old_steps.get(id).copied(), last_state)
}

/// Finds the steps of `current` that are to be judged, determining previous step state via `step_before`.
///
/// Steps not marked done are never judged.
#[must_use]
pub fn steps_to_judge_with<'a, 'b>(
    current: &'a Plan,
    step_before: impl Fn(&ItemId) -> Option<&'b Step>,
    last_state: impl Fn(&ItemId) -> Option<JudgementState>,
) -> Vec<&'a Step> {
    let mut to_judge = Vec::new();

    for step in current.steps() {
        if step.state() != StepState::Done {
            continue;
        }
        let Some(old_step) = step_before(&step.id()) else {
            to_judge.push(step);
            continue;
        };
        if old_step.state() != StepState::Done {
            to_judge.push(step);
            continue;
        }
        if last_state(&step.id()).is_none() {
            to_judge.push(step);
            continue;
        }
        if last_state(&step.id()) == Some(JudgementState::Mismatch) {
            to_judge.push(step);
            continue;
        }
        if *old_step != *step {
            to_judge.push(step);
        }
    }

    to_judge
}
