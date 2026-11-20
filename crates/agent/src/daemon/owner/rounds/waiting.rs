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
    /// The pause counts from now, after the failed call: the quick retries inside the call do not shorten it. One
    /// warning is logged when the step starts to wait and none per try; a refusal that will not pass is told to the
    /// human once while it waits.
    ///
    /// # Errors
    /// [`OwnerError::Clock`] when the clock gives a time that cannot be recorded or the pause would pass the last year
    /// a time records.
    pub(super) fn github_step_failed(
        &mut self,
        step: &Step,
        team: &TeamId,
        which: WaitingStep,
        pr: Option<PrNumber>,
        error: &PublishError,
    ) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let outlook = outlook_of(error);
        let offset = self.parts.local_offset;
        let (failure, wait) =
            self.watch
                .github_waits
                .failed(step, team, which, pr, error_chain(error), outlook, now)?;
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
            self.tell_refused(step, team, which, &wait, now)?;
        }
        Ok(())
    }

    /// Forgets the steps that wait for GitHub or git in a round that closed, or of a team that is closed or gone: nothing
    /// tries them again, and the owner's memory holds the waits of open rounds and teams only.
    pub(in crate::daemon::owner) fn drop_closed_waits(&mut self) {
        let state = &self.parts.state;
        let closed = &self.closed;
        self.watch.github_waits.keep(|place| match place {
            Step::Round { round } => state
                .find_round(round)
                .is_some_and(|(_, found)| *found.life() == agent_core::state::RoundLife::Open),
            Step::Team { team } => state.team(team).is_some() && !closed.contains(team),
        });
    }

    /// Where a step for `items` of `team` waits: their round, or the team's rounds as a whole for items whose round
    /// the state cannot name.
    pub(super) fn place_of(&self, team: &TeamId, items: &[agent_core::ids::ItemId]) -> Step {
        self.rounds_of_items(team, items)
            .into_iter()
            .next()
            .map_or_else(|| Step::Team { team: team.clone() }, |round| Step::Round { round })
    }

    /// The commit of the worktree `git` opens, read as the step `which` of `place` of `team`: `None` while the step
    /// waits, or when git cannot open the worktree or read its commit, which makes the step wait. The caller leaves
    /// what depends on the commit for a later look.
    ///
    /// # Errors
    /// [`OwnerError::Clock`] when the time of a failure cannot be recorded.
    pub(super) async fn head_of(
        &mut self,
        place: &Step,
        team: &TeamId,
        which: WaitingStep,
        pr: Option<PrNumber>,
        git: Result<agent_runtime::git::Git, PublishError>,
    ) -> Result<Option<agent_core::state::CommitId>, OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        if !self.watch.github_waits.due(place, which, now) {
            return Ok(None);
        }
        let read = match git {
            Ok(git) => git
                .rev_parse(&agent_runtime::git::Revision::Head)
                .await
                .map_err(PublishError::from),
            Err(error) => Err(error),
        };
        match read {
            Ok(head) => {
                self.github_step_done(place, team, which);
                Ok(Some(head))
            }
            Err(error) => {
                self.github_step_failed(place, team, which, pr, &error)?;
                Ok(None)
            }
        }
    }

    /// Takes again the verdicts of the verifiers of `team` that were held because git could not read the commit they
    /// verified; one that still cannot be read is held again.
    ///
    /// # Errors
    /// [`OwnerError`] when a verdict cannot be recorded.
    pub(super) async fn take_held_verdicts(&mut self, team: &TeamId) -> Result<(), OwnerError> {
        let tasks: Vec<agent_core::ids::TaskId> = self
            .watch
            .held_verdicts
            .keys()
            .filter(|task| {
                self.parts
                    .state
                    .find_task(task)
                    .is_some_and(|found| found.team.id() == team)
            })
            .cloned()
            .collect();
        for task in tasks {
            let Some(message) = self.watch.held_verdicts.remove(&task) else {
                continue;
            };
            let located = self.locate(&task)?;
            self.review_ended(&task, &located, Some(&message)).await?;
        }
        Ok(())
    }

    /// The human's `agent resume` of `target`: every step of its team that waits for GitHub or git is tried at the
    /// next look, a refusal too. Returns how many steps waited: when some did, the resume is for them, and the tasks
    /// of the team are not started again. A target the state does not have resumes nothing here; the control says so.
    pub(in crate::daemon::owner) fn resume_waits(&mut self, target: &agent_core::protocol::Target) -> usize {
        use agent_core::protocol::Target;
        let team = match target {
            Target::Team(team) => Some(team.clone()),
            Target::Task(task) => self.parts.state.find_task(task).map(|found| found.team.id().clone()),
            Target::Round(round) => self.parts.state.find_round(round).map(|(team, _)| team.id().clone()),
        };
        let Some(team) = team else {
            return 0;
        };
        let waited = self.watch.github_waits.forget_team(&team);
        self.look_at(&team);
        waited
    }

    /// Records that `step` of `team` went through: a step that waited is done, logged once, and a refusal of it later
    /// is told to the human again.
    pub(super) fn github_step_done(&mut self, step: &Step, team: &TeamId, which: WaitingStep) {
        if self.watch.github_waits.done(step, which) {
            self.watch.announced.remove(&Announcement::GithubRefused {
                team: team.clone(),
                place: step.clone(),
                operation: which,
            });
            tracing::info!(
                event = LogEvent::GithubStepResumed.as_str(),
                team = %team,
                operation = %which,
                "the step of the round that waited for GitHub went through"
            );
        }
    }

    /// Tells the lead of `team` that GitHub refuses `step` of its round, once while the step waits, and how to try again.
    fn tell_refused(
        &mut self,
        step: &Step,
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
            place: step.clone(),
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
        let obligation = if self.pulls.is_none() {
            Some(Obligation::Human)
        } else {
            self.stuck_obligation(team, round)
                .or_else(|| self.round_obligation(team, round))
                // A step that waits for GitHub or git owns the round: its next try, or the human's resume of a
                // refusal. The status says which before anything else.
                .or_else(|| {
                    self.watch
                        .github_waits
                        .line_of(team.id(), round.id(), self.parts.local_offset)
                        .map(|_| Obligation::Human)
                })
        };
        round_wait(activity, obligation)
    }

    /// What a round with a member that cannot work waits for, or `None` when no member is stuck: the human, or, for a
    /// reviewer whose review another free investigator takes, a free investigator once the owner found none.
    ///
    /// A reviewer whose review is no longer asked — one that failed and was replaced — holds nothing of the round:
    /// only its open question waits for the human.
    pub(in crate::daemon::owner) fn stuck_obligation(&self, team: &Team, round: &Round) -> Option<Obligation> {
        let stuck = |status| {
            matches!(
                status,
                TaskStatus::Questions | TaskStatus::Paused | TaskStatus::Stopped | TaskStatus::Failed
            )
        };
        let members = team.members_in(round.id());
        if members.iter().any(|(role, member)| {
            member.open_question().is_some() || (stuck(member.status()) && *role != Role::Reviewer)
        }) {
            return Some(Obligation::Human);
        }
        let replaceable = members.iter().any(|(role, member)| {
            *role == Role::Reviewer && stuck(member.status()) && crate::daemon::merge::review_asked(team, member.task())
        });
        replaceable.then(|| {
            if self.watch.waits.get(round.id()) == Some(&Obligation::FreeInvestigator) {
                Obligation::FreeInvestigator
            } else {
                Obligation::Human
            }
        })
    }

    /// The pending operation, including gates which do not run a model task.
    fn round_obligation(&self, team: &Team, round: &Round) -> Option<Obligation> {
        if round.delivered_by().is_some() {
            return Some(Obligation::Merge);
        }
        let lead = team.crew().roster().into_iter().next().map(|(_, member)| member)?;
        // Only the harness's own gate is tracked to its end. The token the owner keeps for a gate a lead asked for stays
        // after the gate is over, and that gate is awaited by the lead's run, which is active, not idle.
        if matches!(self.watch.own_gates.get(lead.task()), Some(OwnGate::Running { .. })) {
            return Some(Obligation::Gate);
        }
        let item = round.item().and_then(|id| team.item(id));
        if let Some(ledger) = item.and_then(agent_core::state::ItemProgress::findings) {
            // An opinion that is still unassigned after the scheduling of a look has no free independent investigator.
            if ledger
                .opinions()
                .values()
                .any(|opinion| matches!(opinion.stage, OpinionStage::Waiting))
            {
                return Some(Obligation::FreeInvestigator);
            }
            if ledger
                .opinions()
                .values()
                .any(|opinion| matches!(opinion.stage, OpinionStage::Assigned { .. }))
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
