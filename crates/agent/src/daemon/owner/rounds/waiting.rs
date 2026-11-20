//! Project the owner's queued work and the durable round state into an idle wait reason.

use agent_core::decide::wait::{Activity, Obligation, Waiting, round_wait};
use agent_core::ids::TeamId;
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::prompt::Variables;
use agent_core::state::{ItemPhase, OpinionStage, PrNumber, Round, RoundLife, Team, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::github::Outlook;
use agent_runtime::process::error_chain;

use super::super::{Owner, OwnerError};
use super::publish::{GATE_SEND_BACKS, OwnGate};
use super::{Announcement, AutopilotNotice, PublishError};

mod github;
#[cfg(test)]
mod tests;

pub(in crate::daemon::owner) use self::github::{Failure, Step, Wait, WaitingStep, Waits, outlook_of};
#[cfg(test)]
#[path = "waiting/tests/github.rs"]
mod github_tests;

impl Owner {
    /// Records that `step` of `team` failed with `error` and waits: the round loses nothing, the daemon goes on with
    /// its other rounds, and the step is tried again after its pause -- at once, for a refusal, on `agent resume`.
    ///
    /// One warning is logged when the step starts to wait and none per try; a refusal that will not pass is told to
    /// the human once.
    ///
    /// # Errors
    /// [`OwnerError::Clock`] when the pause would pass the last year a time records.
    pub(in crate::daemon::owner) fn github_step_failed(
        &mut self,
        step: &Step,
        team: &TeamId,
        which: WaitingStep,
        pr: Option<PrNumber>,
        error: &PublishError,
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        let outlook = outlook_of(error);
        let offset = self.parts.local_offset;
        let (failure, wait) = self
            .watch
            .github_waits
            .failed(step, team, which, pr, error_chain(error), outlook, now)
            .map_err(|missed| missed.owner_error())?;
        let starts = matches!(failure, Failure::First | Failure::Starts);
        if starts {
            tracing::warn!(
                event = LogEvent::GithubStepWaiting.as_str(),
                team = %team,
                operation = %which,
                pr = pr.map_or_else(String::new, |number| number.to_string()),
                outlook = match outlook {
                    Outlook::MayPass => "may_pass",
                    Outlook::WillNotPass => "will_not_pass",
                },
                next_try = wait.next_at.map_or_else(String::new, |at| at.to_string()),
                detail = %wait.line(offset),
                error = %error_chain(error),
                "a step of the round failed and waits for GitHub: {}",
                wait.line(offset)
            );
        }
        if outlook == Outlook::WillNotPass && starts {
            self.tell_refused(team, which, &wait, now)?;
        }
        Ok(())
    }

    /// Records that `step` of `team` went through: a step that waited is done, logged once.
    pub(in crate::daemon::owner) fn github_step_done(&mut self, step: &Step, team: &TeamId, which: WaitingStep) {
        if self.watch.github_waits.done(step, which) {
            tracing::info!(
                event = LogEvent::GithubStepResumed.as_str(),
                team = %team,
                operation = %which,
                "the step of the round that waited for GitHub went through"
            );
        }
    }

    /// Tells the lead of `team` that GitHub refuses a step of its round, once for the step, and how to try again.
    fn tell_refused(
        &mut self,
        team: &TeamId,
        which: WaitingStep,
        wait: &Wait,
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        let Some(pulls) = self.pulls.clone() else {
            return Ok(());
        };
        let announcement = Announcement::GithubRefused {
            team: team.clone(),
            operation: which,
        };
        if !self.watch.announced.insert(announcement) {
            return Ok(());
        }
        let lead = self.lead_of(team)?;
        let values = Variables::new()
            .with("team", team.as_str())
            .and_then(|values| values.with("operation", which.operation()))
            .and_then(|values| values.with("answer", wait.answer.as_str()));
        self.tell_human(&pulls, AutopilotNotice::GithubRefused, values, &lead, now);
        Ok(())
    }

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
