//! A step's line in the daemon log: what the agent did, and who ran it.

#[cfg(test)]
mod tests;

use crate::history::Step;
use crate::ids::AgentName;

/// The line of `step` that `agent log` prints as text.
#[must_use]
pub fn step_line(step: &Step, _run: Option<&crate::history::RunStart>, _followed: &[AgentName]) -> String {
    format!("{} step:", step.at)
}
