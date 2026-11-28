//! Pure selection of the next holder of a lead task.

use crate::ids::{AgentName, TaskId};
use crate::state::{State, Team, Timestamp};

/// Clock and configured candidates supplied by the caller, without reading configuration or time here.
#[derive(Debug)]
pub struct ShiftLimits<'a> {
    /// Maximum worked minutes per holder; zero disables shifts.
    pub lead_shift_minutes: u32,
    /// Time at which a running lead's work is measured.
    pub now: Timestamp,
    /// Solo-task rotation order from `[replacement] agents`.
    pub replacement_agents: &'a [AgentName],
}

/// Whether a lead should retain its task or give it to another model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShiftCheck {
    /// The task has not reached a shift boundary or is not eligible.
    NotDue,
    /// Work is due to rotate, but every other eligible agent is occupied.
    NoneFree,
    /// The next free agent in rotation order.
    To(AgentName),
}

/// Chooses a lead's next holder without mutating state or consulting a clock.
#[must_use]
pub fn shift_target(_task: &TaskId, _team: &Team, _state: &State, _limits: &ShiftLimits<'_>) -> ShiftCheck {
    ShiftCheck::NotDue
}

#[cfg(test)]
mod tests;
