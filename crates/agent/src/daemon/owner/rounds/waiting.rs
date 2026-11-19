//! Project the owner's queued work and the durable round state into an idle wait reason.

use agent_core::decide::wait::{Activity, Obligation, Waiting, round_wait};
use agent_core::kinds::Role;
use agent_core::state::{ItemPhase, OpinionStage, Round, RoundLife, Team};
use agent_core::status::TaskStatus;

use super::super::Owner;
use super::publish::{GATE_SEND_BACKS, OwnGate};

#[cfg(test)]
mod tests;

impl Owner {
    /// Records an unowned idle round once, after all scheduling in a look has finished.
    pub(in crate::daemon::owner) fn observe_wait(
        &mut self,
        round: &agent_core::ids::RoundId,
        waiting: Option<Waiting>,
    ) {
        if waiting == Some(Waiting::Defect) && self.watch.unowned.insert(round.clone()) {
            tracing::error!(
                event = agent_core::log::LogEvent::DaemonTaskFailed.as_str(),
                rounds = %round,
                operation = "round_wait",
                error = Waiting::Defect.text(),
            );
        }
    }

    /// Checks for a lost scheduling obligation after the owner has assigned all work.
    pub(in crate::daemon::owner) fn observe_round_waits(&mut self, team: &agent_core::ids::TeamId) {
        let waits: Vec<_> = self
            .parts
            .state
            .team(team)
            .into_iter()
            .flat_map(|team| {
                team.rounds()
                    .iter()
                    .map(|round| (round.id().clone(), self.waiting_for(team, round)))
            })
            .collect();
        for (round, waiting) in waits {
            self.observe_wait(&round, waiting);
        }
    }

    /// The action that owns the next step of an idle round.
    pub(in crate::daemon::owner) fn waiting_for(&self, team: &Team, round: &Round) -> Option<Waiting> {
        let members = team.members_in(round.id());
        let activity = if *round.life() != RoundLife::Open {
            Activity::Closed
        } else if members
            .iter()
            .any(|(_, member)| matches!(member.status(), TaskStatus::Queued | TaskStatus::Running))
        {
            Activity::Active
        } else {
            Activity::Idle
        };
        // A reviewer whose review is no longer asked — one that failed and was replaced — holds nothing of the round:
        // only its open question waits for the human.
        let human = members.iter().any(|(role, member)| {
            member.open_question().is_some()
                || (matches!(
                    member.status(),
                    TaskStatus::Questions | TaskStatus::Paused | TaskStatus::Stopped | TaskStatus::Failed
                ) && (*role != Role::Reviewer || crate::daemon::merge::review_asked(team, member.task())))
        });
        let obligation = if human || self.pulls.is_none() {
            Some(Obligation::Human)
        } else {
            self.round_obligation(team, round)
        };
        round_wait(activity, obligation)
    }

    /// The pending operation, including gates which do not run a model task.
    fn round_obligation(&self, team: &Team, round: &Round) -> Option<Obligation> {
        let lead = team.crew().roster().into_iter().next().map(|(_, member)| member)?;
        // Only the harness's own gate is tracked to its end. The token the owner keeps for a gate a lead asked for stays
        // after the gate is over, and that gate is awaited by the lead's run, which is active, not idle.
        if matches!(self.watch.own_gates.get(lead.task()), Some(OwnGate::Running { .. })) {
            return Some(Obligation::Gate);
        }
        let item = round.item().and_then(|id| team.item(id));
        if let Some(ledger) = item.and_then(agent_core::state::ItemProgress::findings) {
            if ledger
                .opinions()
                .values()
                .any(|opinion| matches!(opinion.stage, OpinionStage::Waiting | OpinionStage::Assigned { .. }))
            {
                return Some(Obligation::SecondOpinion);
            }
            if let Some(disputes) = ledger.disputes()
                && disputes.correction.is_some()
            {
                return Some(if disputes.bounces > super::verdicts::MAX_VERDICT_BOUNCES {
                    Obligation::Human
                } else {
                    Obligation::LeadCorrection
                });
            }
        }
        if self
            .watch
            .gate_sent_back
            .get(lead.task())
            .is_some_and(|sent| *sent >= GATE_SEND_BACKS)
        {
            return Some(Obligation::Human);
        }
        if let Some(obligation) = self.watch.waits.get(round.id()) {
            return Some(*obligation);
        }
        match item.map(agent_core::state::ItemProgress::phase) {
            Some(ItemPhase::InReview { .. }) => Some(Obligation::Review),
            Some(ItemPhase::Approved { .. }) => Some(Obligation::Human),
            Some(ItemPhase::InProgress { .. } | ItemPhase::Fixing { .. }) | None
                if !matches!(
                    lead.runs().last().map(agent_core::state::Run::state),
                    Some(agent_core::state::RunState::Ended {
                        end: agent_core::state::RunEnd::Reported {
                            outcome: agent_core::status::RunOutcome::Done
                        },
                        ..
                    })
                ) =>
            {
                // The sleeping-team watchdog owns the wake or its bounded human escalation.
                Some(Obligation::LeadCorrection)
            }
            Some(
                ItemPhase::Pending | ItemPhase::Done { .. } | ItemPhase::InProgress { .. } | ItemPhase::Fixing { .. },
            )
            | None => None,
        }
    }
}
