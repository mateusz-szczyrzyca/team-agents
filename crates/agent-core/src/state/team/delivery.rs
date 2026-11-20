//! Checked delivery claims and completion through another round's pull request.

use super::{CheckedMut, Team};
use crate::ids::RoundId;
use crate::state::{ClosedBeforeOpened, Completion, ItemPhase, PrNumber, PullRequest, Round, RoundEnd, Timestamp};

/// Why a round cannot take part in a delivery: the check both operations make first.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkingRoundError {
    /// The team does not own this round.
    NoSuchRound(RoundId),
    /// The round is not that of an item in progress.
    NotWorking(RoundId),
}

/// Why [`Team::deliver_round`] refused a claim; a refusal leaves the team unchanged.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeliverRoundError {
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
}

impl From<WorkingRoundError> for DeliverRoundError {
    fn from(error: WorkingRoundError) -> Self {
        match error {
            WorkingRoundError::NoSuchRound(round) => Self::NoSuchRound { round },
            WorkingRoundError::NotWorking(round) => Self::NotWorking { round },
        }
    }
}

/// Why [`Team::complete_delivery`] refused a completion; a refusal leaves the team unchanged.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompleteDeliveryError {
    /// The team does not own this round.
    #[error("no such round {round}")]
    NoSuchRound {
        /// The requested round.
        round: RoundId,
    },
    /// Only a round whose item is still in progress can complete.
    #[error("round {round} is not a working item")]
    NotWorking {
        /// The requested round.
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

impl From<WorkingRoundError> for CompleteDeliveryError {
    fn from(error: WorkingRoundError) -> Self {
        match error {
            WorkingRoundError::NoSuchRound(round) => Self::NoSuchRound { round },
            WorkingRoundError::NotWorking(round) => Self::NotWorking { round },
        }
    }
}

/// Why an observed closed delivery claim cannot be ended atomically.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EndDeliveryError {
    /// The requested round is not in this team.
    #[error("no such round {round}")]
    NoSuchRound {
        /// The requested round.
        round: RoundId,
    },
    /// Only the latest round of an item in progress can return to work.
    #[error("round {round} is not a working item")]
    NotWorking {
        /// The requested round.
        round: RoundId,
    },
    /// The claim is absent or has already ended.
    #[error("round {round} has no waiting delivery claim")]
    NotWaiting {
        /// The requested round.
        round: RoundId,
    },
    /// The observation cannot predate the opening.
    #[error(transparent)]
    BeforeOpening(#[from] ClosedBeforeOpened),
}

impl Team {
    /// Ends a delivery claim after its PR is observed closed without merging.
    ///
    /// # Errors
    /// Returns [`EndDeliveryError`] without mutation for an unknown, non-working or non-waiting round, or an early time.
    pub fn end_delivery(&mut self, _id: &RoundId, _at: Timestamp) -> Result<(), EndDeliveryError> {
        Ok(())
    }

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

    /// The rounds whose accepted delivery still waits for its PR to merge: open ones, and ones abandoned by a stop of the
    /// team, while their item is in progress and they are its latest round.
    pub fn delivery_waits(&self) -> impl Iterator<Item = &Round> {
        self.rounds.iter().filter(|round| {
            round.delivered_by().is_some()
                    && self.item_works(round)
                    // A part that completed is followed by the round of the next part, which is the one that works.
                    && round
                        .item()
                        .and_then(|item| self.round_of(item))
                        .is_some_and(|latest| latest.id() == round.id())
        })
    }

    /// Records a claim after the caller verifies the item's clean git tree and ancestry.
    ///
    /// # Errors
    /// Returns [`DeliverRoundError`] unless the round is open, working, unclaimed and names another round's team PR.
    pub fn deliver_round(&mut self, id: &RoundId, number: PrNumber) -> Result<(), DeliverRoundError> {
        let round = self.working_delivery_round(id)?;
        if round.delivered_by().is_some() {
            return Err(DeliverRoundError::AlreadyDelivered { round: id.clone() });
        }
        let pr = self
            .delivery_pull_request(number)
            .cloned()
            .ok_or(DeliverRoundError::NoTeamPr { number })?;
        let target = self
            .rounds
            .iter_mut()
            .find(|round| round.id() == id)
            .ok_or_else(|| DeliverRoundError::NoSuchRound { round: id.clone() })?;
        target.deliver(pr);
        Ok(())
    }

    /// Completes an accepted delivery after the caller observes its PR merged.
    ///
    /// # Errors
    /// Returns [`CompleteDeliveryError`] for an unknown, non-working or unclaimed round, or an invalid completion time.
    pub fn complete_delivery(&mut self, id: &RoundId, at: Timestamp) -> Result<(), CompleteDeliveryError> {
        let round = self.waiting_delivery_round(id)?;
        let pr = round
            .delivered_by()
            .cloned()
            .ok_or_else(|| CompleteDeliveryError::NotWaiting { round: id.clone() })?;
        // Stage the fallible close before changing either the item or the durable round. A round that a stop
        // abandoned is closed already and stays as it was: its end is recorded once.
        let mut completed = round.clone();
        if completed.is_open() {
            completed.close(at, RoundEnd::Merged)?;
        } else {
            completed.check_completion_at(at)?;
        }
        if completed.part().is_some() {
            completed.merge_part(pr.clone());
        }
        let item = round
            .item()
            .ok_or_else(|| CompleteDeliveryError::NotWorking { round: id.clone() })?;
        let target = self
            .rounds
            .iter_mut()
            .find(|round| round.id() == id)
            .ok_or_else(|| CompleteDeliveryError::NoSuchRound { round: id.clone() })?;
        let progress = self
            .items
            .iter_mut()
            .find(|progress| progress.id() == item)
            .ok_or_else(|| CompleteDeliveryError::NotWorking { round: id.clone() })?;
        progress.complete_delivery(id, pr, at)?;
        *target = completed;
        if self.next_part(item).is_some()
            && let Some(progress) = self.items.iter_mut().find(|progress| progress.id() == item)
        {
            *progress = crate::state::ItemProgress::new(item, crate::task_list::ItemMark::Open);
        }
        Ok(())
    }

    /// Restricts a completion to a round of a working item that is open or was abandoned by a stop of the team.
    fn waiting_delivery_round(&self, id: &RoundId) -> Result<&Round, WorkingRoundError> {
        let round = self
            .round(id)
            .ok_or_else(|| WorkingRoundError::NoSuchRound(id.clone()))?;
        if !self.item_works(round) || !(round.is_open() || round.ended() == Some(RoundEnd::Abandoned)) {
            return Err(WorkingRoundError::NotWorking(id.clone()));
        }
        Ok(round)
    }

    /// Restricts a claim to an open round of a working item.
    fn working_delivery_round(&self, id: &RoundId) -> Result<&Round, WorkingRoundError> {
        let round = self
            .round(id)
            .ok_or_else(|| WorkingRoundError::NoSuchRound(id.clone()))?;
        if !self.item_works(round) || !round.is_open() {
            return Err(WorkingRoundError::NotWorking(id.clone()));
        }
        Ok(round)
    }

    /// Whether the item of `round` is in progress.
    fn item_works(&self, round: &Round) -> bool {
        round
            .item()
            .and_then(|item| self.item(item))
            .is_some_and(|item| matches!(item.phase(), ItemPhase::InProgress { .. }))
    }
}

impl CheckedMut<'_, Team> {
    /// See [`Team::end_delivery`].
    ///
    /// # Errors
    /// Returns [`EndDeliveryError`] if the claim cannot be ended.
    pub fn end_delivery(&mut self, round: &RoundId, at: Timestamp) -> Result<(), EndDeliveryError> {
        self.value.end_delivery(round, at)
    }

    /// See [`Team::deliver_round`].
    ///
    /// # Errors
    /// Returns [`DeliverRoundError`] when the claim cannot be recorded.
    pub fn deliver_round(&mut self, round: &RoundId, number: PrNumber) -> Result<(), DeliverRoundError> {
        self.value.deliver_round(round, number)
    }

    /// See [`Team::complete_delivery`].
    ///
    /// # Errors
    /// Returns [`CompleteDeliveryError`] when the accepted delivery cannot be completed.
    pub fn complete_delivery(&mut self, round: &RoundId, at: Timestamp) -> Result<(), CompleteDeliveryError> {
        self.value.complete_delivery(round, at)
    }
}

#[cfg(test)]
mod tests;
