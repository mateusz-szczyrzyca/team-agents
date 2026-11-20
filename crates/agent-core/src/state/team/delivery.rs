//! Checked delivery claims and completion through another round's pull request.

use super::{CheckedMut, Team};
use crate::ids::RoundId;
use crate::state::{ClosedBeforeOpened, Completion, ItemPhase, PrNumber, PullRequest, Round, RoundEnd, Timestamp};

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
    pub fn delivery_pull_request(&self, number: PrNumber) -> Option<&PullRequest> {
        self.rounds
            .iter()
            .filter(|round| round.delivered_by().is_none())
            .find_map(|round| {
                let pr = round.part_pull_request().or_else(|| {
                    round
                        .item()
                        .and_then(|item| self.item(item))
                        .and_then(|item| match item.phase() {
                            ItemPhase::InReview { pr, .. }
                            | ItemPhase::Fixing { pr, .. }
                            | ItemPhase::Approved { pr, .. }
                            | ItemPhase::Done {
                                completion: Completion::Merged { pr, .. },
                            } => Some(pr),
                            ItemPhase::Pending
                            | ItemPhase::InProgress { .. }
                            | ItemPhase::Done {
                                completion: Completion::Listed,
                            } => None,
                        })
                });
                pr.filter(|pr| pr.number == number)
            })
    }

    /// Records a claim after the caller verifies the item's clean git tree and ancestry.
    ///
    /// # Errors
    /// Returns [`DeliveryError`] unless the round is open, working, unclaimed and names another round's team PR.
    pub fn deliver_round(&mut self, id: &RoundId, number: PrNumber) -> Result<(), DeliveryError> {
        let round = self.working_delivery_round(id)?;
        if round.delivered_by().is_some() {
            return Err(DeliveryError::AlreadyDelivered { round: id.clone() });
        }
        let pr = self
            .delivery_pull_request(number)
            .cloned()
            .ok_or(DeliveryError::NoTeamPr { number })?;
        let target = self
            .rounds
            .iter_mut()
            .find(|round| round.id() == id)
            .ok_or_else(|| DeliveryError::NoSuchRound { round: id.clone() })?;
        target.deliver(pr);
        Ok(())
    }

    /// Completes an accepted delivery after the caller observes its PR merged.
    ///
    /// # Errors
    /// Returns [`DeliveryError`] for an unknown, non-working or unclaimed round, or an invalid completion time.
    pub fn complete_delivery(&mut self, id: &RoundId, at: Timestamp) -> Result<(), DeliveryError> {
        let round = self.working_delivery_round(id)?;
        let pr = round
            .delivered_by()
            .cloned()
            .ok_or_else(|| DeliveryError::NotWaiting { round: id.clone() })?;
        // Stage the fallible close before changing either the item or the durable round.
        let mut completed = round.clone();
        completed.close(at, RoundEnd::Merged)?;
        if completed.part().is_some() {
            completed.merge_part(pr.clone());
        }
        let item = round
            .item()
            .ok_or_else(|| DeliveryError::NotWorking { round: id.clone() })?;
        let target = self
            .rounds
            .iter_mut()
            .find(|round| round.id() == id)
            .ok_or_else(|| DeliveryError::NoSuchRound { round: id.clone() })?;
        let progress = self
            .items
            .iter_mut()
            .find(|progress| progress.id() == item)
            .ok_or_else(|| DeliveryError::NotWorking { round: id.clone() })?;
        progress.complete_delivery(id, pr, at)?;
        *target = completed;
        if self.next_part(item).is_some()
            && let Some(progress) = self.items.iter_mut().find(|progress| progress.id() == item)
        {
            *progress = crate::state::ItemProgress::new(item, crate::task_list::ItemMark::Open);
        }
        Ok(())
    }

    /// Restricts both operations to an open round of a working item.
    fn working_delivery_round(&self, id: &RoundId) -> Result<&Round, DeliveryError> {
        let round = self
            .round(id)
            .ok_or_else(|| DeliveryError::NoSuchRound { round: id.clone() })?;
        if !round.is_open()
            || !round
                .item()
                .and_then(|item| self.item(item))
                .is_some_and(|item| matches!(item.phase(), ItemPhase::InProgress { .. }))
        {
            return Err(DeliveryError::NotWorking { round: id.clone() });
        }
        Ok(round)
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
