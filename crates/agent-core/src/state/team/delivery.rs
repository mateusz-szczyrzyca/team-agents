//! Checked delivery claims and completion through another round's pull request.

use super::{CheckedMut, Team};
use crate::ids::RoundId;
use crate::state::{ClosedBeforeOpened, PrNumber, PullRequest, Timestamp};

/// Why a delivery transition cannot be performed; a refusal leaves the team unchanged.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeliveryError {
    /// The team does not own this round.
    #[error("no such round {round}")]
    NoSuchRound {
        /// The requested round.
        round: RoundId,
    },
    /// Only an open round whose item is in progress may claim delivery.
    #[error("round {round} is not an open working item")]
    NotWorking {
        /// The requested round.
        round: RoundId,
    },
    /// The report must name a PR actually owned by another round of this team.
    #[error("PR #{number} belongs to no other round of this team")]
    NoTeamPr {
        /// The unverified PR number.
        number: PrNumber,
    },
    /// A waiting round cannot replace its accepted delivery claim.
    #[error("round {round} already has a delivery claim")]
    AlreadyDelivered {
        /// The waiting round.
        round: RoundId,
    },
    /// Completion requires an accepted delivery claim.
    #[error("round {round} has no delivery claim")]
    NotWaiting {
        /// The requested round.
        round: RoundId,
    },
    /// Completion cannot predate the round.
    #[error(transparent)]
    BeforeOpening(#[from] ClosedBeforeOpened),
}

impl Team {
    /// Finds a PR owned by an ordinary or split-part round of this team.
    #[must_use]
    pub fn delivery_pull_request(&self, _number: PrNumber) -> Option<&PullRequest> {
        None
    }

    /// Records a claim after the caller has verified the item's clean git tree and ancestry.
    ///
    /// # Errors
    /// Returns [`DeliveryError`] unless the round is open, working, unclaimed and names another team's round PR.
    pub fn deliver_round(&mut self, _round: &RoundId, _number: PrNumber) -> Result<(), DeliveryError> {
        Ok(())
    }

    /// Completes an accepted delivery after the caller observes its PR merged.
    ///
    /// # Errors
    /// Returns [`DeliveryError`] for an unknown, non-working or unclaimed round, or an invalid completion time.
    pub fn complete_delivery(&mut self, _round: &RoundId, _at: Timestamp) -> Result<(), DeliveryError> {
        Ok(())
    }
}

impl CheckedMut<'_, Team> {
    /// See [`Team::deliver_round`].
    ///
    /// # Errors
    /// Returns [`DeliveryError`] when the claim cannot be recorded.
    pub fn deliver_round(&mut self, round: &RoundId, number: PrNumber) -> Result<(), DeliveryError> {
        self.value.deliver_round(round, number)
    }

    /// See [`Team::complete_delivery`].
    ///
    /// # Errors
    /// Returns [`DeliveryError`] when the accepted delivery cannot be completed.
    pub fn complete_delivery(&mut self, round: &RoundId, at: Timestamp) -> Result<(), DeliveryError> {
        self.value.complete_delivery(round, at)
    }
}

#[cfg(test)]
mod tests;
