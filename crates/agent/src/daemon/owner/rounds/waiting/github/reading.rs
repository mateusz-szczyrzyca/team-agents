//! What the tests of the waits read of them, and the daemon never does.

use agent_core::ids::RoundId;

use super::{Step, Wait, WaitingStep, Waits};

impl Waits {
    /// The wait of the round `round` for `step`, if it has one.
    pub(in crate::daemon::owner) fn for_round(&self, round: &RoundId, step: WaitingStep) -> Option<&Wait> {
        self.steps.get(&(Step::Round { round: round.clone() }, step))
    }

    /// How many waits it keeps.
    pub(in crate::daemon::owner) fn kept(&self) -> usize {
        self.steps.len()
    }
}
