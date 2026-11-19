//! Independent opinions while the lead's current fix pass remains held.
use super::super::teams::Look;
use super::super::{Located, Owner, OwnerError};
use super::reviewers::Prepared;
use super::verdicts::Asked;
use super::{PublishError, Pulls};
use crate::daemon::review::{choose_opinion, free_reviewers, pending_opinion};
use crate::daemon::team::Why;
use agent_core::decide::opinion::{Next, Tip, after_opinions};
use agent_core::history::HistoryEvent;
use agent_core::ids::{ItemId, TaskId, TeamId};
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::review_response::opinion::{Outcome, parse_report};
use agent_core::state::{Findings, ItemPhase, OpinionStage, Timestamp};
use std::collections::HashSet;
use std::num::NonZeroU32;

/// Failure to schedule, render or persist a second opinion.
#[derive(Debug, thiserror::Error)]
enum OpinionCause {
    /// A worktree could not be prepared.
    #[error("cannot prepare second opinion")]
    Prepare(#[from] PublishError),
    /// The exact commit could not be read.
    #[error("cannot read second opinion commit")]
    Git(#[from] agent_runtime::git::GitError),
    /// An opinion transition was invalid.
    #[error("cannot record second opinion")]
    State(#[from] agent_core::state::OpinionStateError),
    /// A catalog value was invalid.
    #[error("cannot set second opinion variable")]
    Variable(#[from] agent_core::prompt::VariablesError),
    /// A catalog template could not be rendered.
    #[error("cannot render second opinion")]
    Render(#[from] agent_runtime::autopilot_prompts::RenderAutopilotTextError),
    /// A finding could not be rendered.
    #[error("cannot render second opinion finding")]
    Findings(#[from] super::findings::TextError),
}

/// Context and cause of a second-opinion lifecycle failure.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub(crate) struct OpinionDaemonError(#[from] OpinionCause);

macro_rules! opinion_error {
    ($source:ty) => {
        impl From<$source> for OpinionDaemonError {
            fn from(source: $source) -> Self {
                Self(OpinionCause::from(source))
            }
        }
    };
}
opinion_error!(PublishError);
opinion_error!(agent_runtime::git::GitError);
opinion_error!(agent_core::state::OpinionStateError);
opinion_error!(agent_core::prompt::VariablesError);
opinion_error!(agent_runtime::autopilot_prompts::RenderAutopilotTextError);
opinion_error!(super::findings::TextError);

impl Owner {
    /// Copies one PR's authoritative ledger to all its items.
    fn keep_opinions(&mut self, team: &TeamId, items: &[ItemId], ledger: &Findings) -> Result<(), OwnerError> {
        let mut found = self
            .parts
            .state
            .team_mut(team)
            .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?;
        for id in items {
            let mut item = found.item_mut(*id).ok_or_else(|| OwnerError::LostItem {
                team: team.clone(),
                item: *id,
            })?;
            if let Some(saved) = item.findings_mut() {
                *saved = ledger.clone();
            }
        }
        Ok(())
    }

    /// Gives each waiting opinion its own independent free investigator, or leaves it waiting.
    pub(super) async fn ask_opinions(
        &mut self,
        _pulls: &Pulls,
        team_id: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let items = self
            .parts
            .state
            .team(team_id)
            .map_or_else(Vec::new, |team| team.items().to_vec());
        let lead = self.lead_of(team_id)?;
        let mut seen = HashSet::new();
        for item in items {
            let ItemPhase::Fixing { pr, .. } = item.phase() else {
                continue;
            };
            if !seen.insert(pr.number) {
                continue;
            }
            let Some(mut ledger) = item.findings().cloned() else {
                continue;
            };
            let ids = self
                .parts
                .state
                .team(team_id)
                .map_or_else(Vec::new, |team| crate::daemon::merge::items_of(team, pr.number));
            let waiting: Vec<_> = ledger
                .opinions()
                .iter()
                .filter(|(_, opinion)| matches!(opinion.stage, OpinionStage::Waiting))
                .map(|(number, opinion)| (*number, opinion.request.clone()))
                .collect();
            for (number, request) in waiting {
                let mut excluded: HashSet<_> = item
                    .fix_passes()
                    .last()
                    .into_iter()
                    .flat_map(|pass| pass.requested_by.iter().map(|review| review.reviewer.clone()))
                    .collect();
                if let Some(entry) = ledger.entries().iter().find(|entry| entry.number == number) {
                    excluded.insert(entry.reporter.clone());
                }
                let free = free_reviewers(&self.parts.state, &self.parts.settings.config, team_id);
                let Some(reviewer) = choose_opinion(free, &excluded, &mut self.parts.rng) else {
                    continue;
                };
                let now = Timestamp::now(&self.parts.clock)?;
                let git = self
                    .worktree_git(&lead.repo, &lead.task, &lead.agent)
                    .map_err(OpinionDaemonError::from)?;
                let mut prepared = Prepared::default();
                if let Err(error) = self
                    .prepare_reviewer(&git, &lead, reviewer, &request.head, now, &mut prepared)
                    .await
                {
                    self.undo(&git, prepared.made()).await;
                    return Err(OpinionDaemonError::from(error).into());
                }
                let review = prepared
                    .reviews()
                    .first()
                    .ok_or(PublishError::Lost)
                    .map_err(OpinionDaemonError::from)?;
                let mut next = self.parts.state.clone();
                for member in prepared.members() {
                    next.add_reviewer(team_id, member.clone())
                        .map_err(PublishError::from)
                        .map_err(OpinionDaemonError::from)?;
                }
                ledger
                    .assign_opinion(number, review.reviewer.clone(), review.task.clone())
                    .map_err(OpinionDaemonError::from)?;
                self.parts.state = next;
                self.keep_opinions(team_id, &ids, &ledger)?;
                self.record_event_at(
                    now,
                    team_id,
                    self.rounds_of_items(team_id, &ids),
                    HistoryEvent::SecondOpinionAsked {
                        pr: pr.number,
                        number,
                        pass: request.pass.get(),
                        by: lead.agent.clone(),
                        why: request.reason.as_str().to_owned(),
                    },
                );
                self.release(&review.task, Why::Review, look)?;
                look.changed = true;
            }
        }
        Ok(())
    }

    /// Accepts or corrects the report of the currently assigned task only.
    pub(super) async fn opinion_ended(
        &mut self,
        pulls: &Pulls,
        task: &TaskId,
        located: &Located,
        message: Option<&str>,
    ) -> Result<bool, OwnerError> {
        let Some((id, number, opinion)) = self
            .parts
            .state
            .team(&located.team)
            .and_then(|team| pending_opinion(team, task))
            .map(|(id, number, opinion)| (id, number, opinion.clone()))
        else {
            return Ok(false);
        };
        let mut look = Look::default();
        let report = match message.map(parse_report) {
            Some(Ok(report)) => report,
            Some(Err(_problem)) => {
                let bounces = self.watch.bounces.entry(task.clone()).or_default();
                *bounces = bounces.saturating_add(1);
                if *bounces <= super::verdicts::MAX_VERDICT_BOUNCES {
                    self.release(task, Why::VerdictMissing, &mut look)?;
                    self.finish_look(&located.team, look).await?;
                } else {
                    self.hold_opinion(pulls, &located.team, task).await?;
                }
                return Ok(true);
            }
            None => {
                self.hold_opinion(pulls, &located.team, task).await?;
                return Ok(true);
            }
        };
        let team_id = &located.team;
        let team = self
            .parts
            .state
            .team(team_id)
            .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
        let item = team.item(id).ok_or_else(|| OwnerError::LostItem {
            team: team_id.clone(),
            item: id,
        })?;
        let ItemPhase::Fixing { pr, .. } = item.phase() else {
            return Ok(true);
        };
        let OpinionStage::Assigned { investigator, .. } = opinion.stage else {
            return Ok(true);
        };
        let asked = Asked {
            pr: pr.clone(),
            items: crate::daemon::merge::items_of(team, pr.number),
            passes: item.fix_passes().len(),
            reviewer: investigator.clone(),
        };
        let mut ledger = item
            .findings()
            .cloned()
            .ok_or(PublishError::Lost)
            .map_err(OpinionDaemonError::from)?;
        ledger
            .record_opinion(number, task, report.clone())
            .map_err(OpinionDaemonError::from)?;
        self.keep_opinions(team_id, &asked.items, &ledger)?;
        let event = match report.outcome {
            Outcome::Confirmed => HistoryEvent::FindingConfirmed {
                pr: asked.pr.number,
                number,
                pass: opinion.request.pass.get(),
                by: investigator.as_str().parse()?,
                why: report.evidence.as_str().to_owned(),
            },
            Outcome::NotConfirmed => HistoryEvent::FindingNotConfirmed {
                pr: asked.pr.number,
                number,
                pass: opinion.request.pass.get(),
                by: investigator.as_str().parse()?,
                why: report.evidence.as_str().to_owned(),
            },
        };
        self.record_event(team_id, self.rounds_of_items(team_id, &asked.items), event);
        let lead = self.lead_of(team_id)?;
        self.post_findings(pulls, &lead, team_id, &asked).await;
        look.changed = true;
        self.finish_look(team_id, look).await?;
        Ok(true)
    }

    /// Holds an unanswered task and sends its human notice once, including across restarts.
    pub(in crate::daemon::owner) async fn hold_opinion(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        task: &TaskId,
    ) -> Result<(), OwnerError> {
        let Some((id, number, _)) = self
            .parts
            .state
            .team(team_id)
            .and_then(|team| pending_opinion(team, task))
        else {
            return Ok(());
        };
        let team = self
            .parts
            .state
            .team(team_id)
            .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
        let item = team.item(id).ok_or_else(|| OwnerError::LostItem {
            team: team_id.clone(),
            item: id,
        })?;
        let ItemPhase::Fixing { pr, .. } = item.phase() else {
            return Ok(());
        };
        let ids = crate::daemon::merge::items_of(team, pr.number);
        let mut ledger = item
            .findings()
            .cloned()
            .ok_or(PublishError::Lost)
            .map_err(OpinionDaemonError::from)?;
        let fresh = ledger.mark_opinion_notice(number).map_err(OpinionDaemonError::from)?;
        self.keep_opinions(team_id, &ids, &ledger)?;
        let mut look = Look::default();
        if self
            .parts
            .state
            .find_task(task)
            .is_some_and(|found| found.member.status() == agent_core::status::TaskStatus::Queued)
        {
            self.move_to(task, team_id, agent_core::status::TaskStatus::Stopped, &mut look.events)?;
        }
        if fresh {
            let lead = self.lead_of(team_id)?;
            let rounds = self
                .rounds_of_items(team_id, &ids)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let values = Variables::new()
                .with("round", rounds)
                .and_then(|v| v.with("number", number.to_string()))
                .and_then(|v| v.with("task", task.as_str()));
            self.tell_human(
                pulls,
                super::AutopilotNotice::Opinion,
                values,
                &lead,
                Timestamp::now(&self.parts.clock)?,
            );
        }
        look.changed = true;
        self.finish_look(team_id, look).await
    }

    /// Identifies unanswered opinions after startup without attempting part 2b recovery.
    pub(in crate::daemon::owner) async fn hold_restarted_opinions(&mut self) -> Result<(), OwnerError> {
        let Some(pulls) = self.pulls.clone() else {
            return Ok(());
        };
        let pending: Vec<_> = self
            .parts
            .state
            .teams()
            .iter()
            .flat_map(|team| {
                team.items()
                    .iter()
                    .filter_map(|item| item.findings())
                    .flat_map(|ledger| ledger.opinions().values())
                    .filter_map(|opinion| match &opinion.stage {
                        OpinionStage::Assigned { task, .. } => Some((team.id().clone(), task.clone())),
                        OpinionStage::Waiting | OpinionStage::Reported { .. } => None,
                    })
            })
            .collect();
        for (team, task) in pending {
            self.hold_opinion(&pulls, &team, &task).await?;
        }
        Ok(())
    }

    /// Returns confirmed blockers to the lead only after every requested opinion is in.
    pub(super) fn continue_opinions(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let lead = self.lead_of(team_id)?;
        let items = self
            .parts
            .state
            .team(team_id)
            .map_or_else(Vec::new, |team| team.items().to_vec());
        for item in items {
            let ItemPhase::Fixing { .. } = item.phase() else {
                continue;
            };
            let Some(ledger) = item.findings() else {
                continue;
            };
            let Some(pass) = NonZeroU32::new(crate::daemon::merge::count(item.fix_passes().len())) else {
                continue;
            };
            if let Next::FixConfirmed(numbers) =
                after_opinions(ledger, pass, &ledger.fixed_answers(pass), Tip::Unchanged)
                && ledger
                    .disputes()
                    .is_some_and(|disputes| disputes.answered_pass == pass.get())
            {
                let findings =
                    super::findings::selected_text(&pulls.texts, Some(ledger), |entry| numbers.contains(&entry.number))
                        .map_err(OpinionDaemonError::from)?;
                let text = pulls
                    .texts
                    .render(
                        AutopilotPrompt::ReviewOpinionConfirmed,
                        &Variables::new()
                            .with("findings", findings)
                            .map_err(OpinionDaemonError::from)?,
                    )
                    .map_err(OpinionDaemonError::from)?;
                if let Some(mut team) = self.parts.state.team_mut(team_id)
                    && let Some(mut progress) = team.item_mut(item.id())
                    && let Some(disputes) = progress.findings_mut().and_then(Findings::disputes_mut)
                {
                    disputes.answered_pass = 0;
                }
                self.release(&lead.task, Why::Gate(text), look)?;
            }
        }
        Ok(())
    }
}
