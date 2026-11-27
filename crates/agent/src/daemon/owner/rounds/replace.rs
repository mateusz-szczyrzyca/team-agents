//! A reviewer that cannot work: another free investigator takes its review, and when none is free the human is told.

use agent_core::decide::limit::{Decision, decide};
use agent_core::decide::pool::provider_of;
use agent_core::decide::reviewer::{Awaited, Provided, on_provider, quiet_since, replace_after_refusal};
use agent_core::history::HistoryEvent;
use agent_core::ids::{InvestigatorName, ItemId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::prompt::{Variables, VariablesError};
use agent_core::state::{ItemPhase, Member, PullRequest, ReviewOutcome, Run, RunState, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;
use std::collections::{BTreeSet, HashSet};

use super::super::teams::Look;
use super::super::{Owner, OwnerError};
use super::asleep::saved_notice_where;
use super::reviewers::Prepared;
use super::{Announcement, AutopilotNotice, Lead, PublishError, Pulls};
use crate::daemon::merge::count;
use crate::daemon::review::{free_reviewers, investigator_of, select_reviewers};
use crate::daemon::settle::limit_of;
use agent_core::decide::investigator::refused_investigators;

/// A review that does not come: its reviewer cannot work.
pub(super) struct Gone {
    /// The pull request under review.
    pr: PullRequest,
    /// The items the pull request carries, whose review passes all have the review.
    items: Vec<ItemId>,
    /// The reviewer that cannot work.
    reviewer: InvestigatorName,
    /// Its task.
    pub(super) task: TaskId,
    /// What its task is: failed, stopped or paused.
    status: TaskStatus,
    /// The reviewers of the review pass, the one that is gone included: none of them takes the review.
    pass: Vec<InvestigatorName>,
    /// Set when the refusal rule changed the reviewer's agent: the provider that refused it, whose investigators meet the
    /// same refusal and so do not take the review, and when the reviewer is back. `None` for a reviewer that failed, was
    /// stopped or was quiet for another reason, which any free investigator may replace.
    refused: Option<Refused>,
}

/// The provider that refused a reviewer, after the refusal rule changed its agent.
struct Refused {
    /// The provider, as [`provider_of`] names it.
    provider: String,
    /// When the refused agent is back in the pool; `None` when no return time is recorded.
    until: Option<Timestamp>,
}

impl Owner {
    /// Replaces opinion owners after the configured interval without output.
    pub(super) async fn replace_silent_opinions(
        &mut self,
        pulls: &Pulls,
        team: &TeamId,
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        use super::opinions::{OpinionCause, OpinionDaemonError};
        use agent_core::state::{OpinionFailure, OpinionStage};
        let patience = self.parts.settings.config.review().replace_after_minutes.as_duration();
        let tasks: HashSet<_> = self
            .parts
            .state
            .team(team)
            .into_iter()
            .flat_map(agent_core::state::Team::items)
            .filter_map(|item| item.findings())
            .flat_map(|ledger| ledger.opinions().values())
            .filter_map(|opinion| match &opinion.stage {
                OpinionStage::Assigned { task, .. } => Some(task.clone()),
                OpinionStage::Waiting | OpinionStage::Reported { .. } => None,
            })
            .collect();
        for task in tasks {
            let Some(run) = self
                .parts
                .state
                .find_task(&task)
                .and_then(|found| found.member.current_run())
            else {
                continue;
            };
            let run_id = run.id().clone();
            let path = self.parts.store.home().run_dir(&task, &run_id).join("out.jsonl");
            let bytes = match tokio::fs::metadata(&path).await {
                Ok(metadata) => metadata.len(),
                // Before the engine creates output, the empty stream is still inactive.
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => 0,
                Err(source) => return Err(OpinionDaemonError(OpinionCause::Inspect { path, source }).into()),
            };
            let activity = self
                .watch
                .opinion_activity
                .entry(task.clone())
                .or_insert((run_id.clone(), bytes, now));
            if activity.0 != run_id || activity.1 != bytes {
                *activity = (run_id, bytes, now);
            }
            let quiet = now.as_utc() - activity.2.as_utc();
            if i128::from(quiet.whole_seconds()) < i128::from(patience.as_secs()) {
                continue;
            }
            if let Some(cancel) = self.cancels.get(&task) {
                self.halts.insert(task.clone(), crate::daemon::control::Halt::Stop);
                cancel.cancel();
            }
            self.retry_opinion(pulls, team, &task, OpinionFailure::Silent).await?;
            self.watch.opinion_activity.remove(&task);
        }
        Ok(())
    }

    /// The reviews of the team `team_id` that are still to come and whose reviewers cannot work, at `now`.
    pub(super) fn reviews_that_do_not_come(&self, team_id: &TeamId, now: Timestamp) -> Vec<Gone> {
        let mut found: Vec<Gone> = Vec::new();
        let Some(team) = self.parts.state.team(team_id) else {
            return found;
        };
        let patience = self.parts.settings.config.review().replace_after_minutes.as_duration();
        let Some(lead) = team.crew().roster().first().map(|(_, member)| member.status()) else {
            return found;
        };
        for progress in team.items() {
            let ItemPhase::InReview { pr, reviews } = progress.phase() else {
                continue;
            };
            for review in reviews {
                if !matches!(review.outcome, ReviewOutcome::Pending) {
                    continue;
                }
                if let Some(known) = found.iter_mut().find(|gone| gone.task == review.task) {
                    known.items.push(progress.id());
                    continue;
                }
                let Some(member) = team.reviewers().iter().find(|member| member.task() == &review.task) else {
                    continue;
                };
                let since = quiet_since(
                    member.runs().iter().map(agent_core::state::Run::state),
                    review.requested_at,
                );
                let quiet_for = std::time::Duration::try_from(now.as_utc() - since.as_utc()).unwrap_or_default();
                let awaited = Awaited {
                    reviewer: member.status(),
                    lead,
                    quiet_for,
                    patience,
                };
                let refusal = self.refusal_of(member);
                if replace_after_refusal(&awaited, refusal) {
                    let refused = (refusal == Some(Decision::ChangeAgent))
                        .then(|| self.refused_by(member))
                        .flatten();
                    found.push(Gone {
                        pr: pr.clone(),
                        items: vec![progress.id()],
                        reviewer: review.reviewer.clone(),
                        task: review.task.clone(),
                        status: member.status(),
                        pass: reviews.iter().map(|review| review.reviewer.clone()).collect(),
                        refused,
                    });
                }
            }
        }
        found
    }

    /// Gives every review of the team `team_id` whose reviewer cannot work to another free investigator; when none is
    /// free the human is told, once for a review.
    pub(super) async fn replace_reviewers(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let gone = self.reviews_that_do_not_come(team_id, now);
        if gone.is_empty() {
            return Ok(());
        }
        let lead = self.lead_of(team_id)?;
        for gone in gone {
            let rounds = self.rounds_of_items(team_id, &gone.items);
            match self.try_replace(team_id, &lead, &gone, now).await {
                Ok(Some(chosen)) => {
                    for round in &rounds {
                        self.watch.waits.remove(round);
                    }
                    self.tell_replaced(pulls, &lead, &gone, &chosen, now);
                    look.changed = true;
                }
                Ok(None) => {
                    for round in rounds {
                        self.watch
                            .waits
                            .insert(round, agent_core::decide::wait::Obligation::FreeInvestigator);
                    }
                    self.nobody_reviews(pulls, team_id, &lead, &gone, now).await;
                }
                Err(error) => {
                    tracing::warn!(
                        event = LogEvent::GitCallFailed.as_str(),
                        team = %team_id,
                        pr = %gone.pr.number,
                        task = %gone.task,
                        error = %error_chain(&error),
                        "a reviewer that cannot work cannot be replaced: it is tried again at the next look"
                    );
                }
            }
        }
        Ok(())
    }

    /// What the refusal rule answers for the last run of the reviewer `member`, counted as the daemon counted it when the
    /// run ended; `None` when that run was not refused by its provider, or was refused before the daemon counted refusals
    /// (a pause from before the upgrade): such a reviewer is held to `replace_after_minutes` as it was, until its next
    /// run is counted. The run's own end time is used, so the answer does not change as a named reset comes nearer.
    fn refusal_of(&self, member: &Member) -> Option<Decision> {
        let Some(RunState::Ended { at, end }) = member.runs().last().map(Run::state) else {
            return None;
        };
        let limit = limit_of(end, *at)?;
        let refusals = self.parts.state.limit_refusals(member.task());
        if refusals == 0 {
            return None;
        }
        Some(decide(&self.refusal_facts(limit, refusals)))
    }

    /// The provider that refused the reviewer `member` and when its agent is back; `None` when it is no configured
    /// investigator.
    fn refused_by(&self, member: &Member) -> Option<Refused> {
        let name = investigator_of(member)?;
        let config = self.parts.settings.config.investigators().get(name)?;
        Some(Refused {
            provider: provider_of(config.cli, config.model.as_ref()).to_owned(),
            until: self.parts.state.refused_until(&name.into()),
        })
    }

    /// Gives the review of `gone` to a free investigator that is not of its review pass; `None` when none is free, else the investigator that was given it.
    ///
    /// As for the first reviewers, the worktree is ready before the state names the reviewer, and what was made is
    /// removed again when the state cannot be changed.
    async fn try_replace(
        &mut self,
        team_id: &TeamId,
        lead: &Lead,
        gone: &Gone,
        now: Timestamp,
    ) -> Result<Option<InvestigatorName>, PublishError> {
        let free = free_reviewers(&self.parts.state, &self.parts.settings.config, team_id);
        let among: Vec<_> = free.iter().map(|reviewer| reviewer.name.clone()).collect();
        let mut excluded: HashSet<InvestigatorName> = gone.pass.iter().cloned().collect();
        // An investigator its provider refused reviews nothing until its return.
        excluded.extend(refused_investigators(
            &self.parts.state,
            &self.parts.settings.config,
            now,
        ));
        // An investigator of the provider that refused the reviewer meets the same refusal: it is no replacement.
        if let Some(refused) = &gone.refused {
            excluded.extend(self.on_provider(&refused.provider));
        }
        let Some(chosen) = select_reviewers(
            &self.parts.state,
            &self.parts.settings.config,
            team_id,
            std::num::NonZeroUsize::MIN,
            &excluded,
            &mut self.parts.rng,
        )
        .and_then(|reviewers| reviewers.into_iter().next()) else {
            return Ok(None);
        };
        log_decision(
            "replace_reviewer",
            &format!(
                "pr={}, gone={} ({}), among={among:?}",
                gone.pr.number, gone.reviewer, gone.status
            ),
            chosen.name.as_str(),
        );
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let Some(head) = git.branch_tip(&gone.pr.branch).await? else {
            return Ok(None);
        };
        let replacement = chosen.name.clone();
        let mut prepared = Prepared::default();
        self.prepare_reviewer(&git, lead, chosen, &head, now, &mut prepared)
            .await?;
        if let Err(error) = self.record_replacement(team_id, gone, &prepared) {
            self.undo(&git, prepared.made()).await;
            return Err(error);
        }
        let pass = count(self.review_passes_of(team_id, &gone.items));
        let rounds = self.rounds_of_items(team_id, &gone.items);
        for review in prepared.reviews() {
            // The replacement is briefed from what the reviewer that is gone left, before its run is composed.
            self.job_handed_over(&gone.task, &review.task, team_id, rounds.clone())
                .await;
            self.record_event(
                team_id,
                rounds.clone(),
                HistoryEvent::ReviewAsked {
                    pr: gone.pr.number,
                    pass,
                    head: head.clone(),
                    reviewer: review.reviewer.clone(),
                    task: review.task.clone(),
                    among: among.clone(),
                },
            );
            tracing::info!(
                event = LogEvent::ReviewsRequested.as_str(),
                team = %team_id,
                pr = %gone.pr.number,
                gone = %gone.reviewer,
                gone_task = %gone.task,
                gone_status = %gone.status,
                reviewer = %review.reviewer,
                task = %review.task,
                "a reviewer cannot work: its review is asked of another"
            );
        }
        Ok(Some(replacement))
    }

    /// The investigators that run on `provider`.
    fn on_provider(&self, provider: &str) -> BTreeSet<InvestigatorName> {
        let investigators = self.parts.settings.config.investigators();
        let all: Vec<_> = investigators
            .iter()
            .map(|(name, config)| Provided {
                name,
                provider: provider_of(config.cli, config.model.as_ref()),
            })
            .collect();
        on_provider(provider, &all)
    }

    /// The variables of a notice about the review of `gone`, a reviewer its provider refused: the reviewer, the pull
    /// request, the task, the provider and the time the refused agent is back, in the local time zone.
    fn refused_values(&self, gone: &Gone, refused: &Refused) -> Result<Variables, VariablesError> {
        let until = refused.until.map_or_else(
            || "the end of its pause".to_owned(),
            |until| self.parts.zone.stamp_minute(until),
        );
        Variables::new()
            .with("reviewer", gone.reviewer.as_str())
            .and_then(|values| values.with("pr", gone.pr.number.to_string()))
            .and_then(|values| values.with("task", gone.task.as_str()))
            .and_then(|values| values.with("provider", refused.provider.as_str()))
            .and_then(|values| values.with("until", until))
    }

    /// Records the new reviewer and puts its review in the place of the one of `gone`, in every item of the pull request,
    /// on a copy of the state that replaces it only when all of it is recorded.
    fn record_replacement(&mut self, team_id: &TeamId, gone: &Gone, prepared: &Prepared) -> Result<(), PublishError> {
        let Some(review) = prepared.reviews().first() else {
            return Err(PublishError::Lost);
        };
        let mut next = self.parts.state.clone();
        for member in prepared.members() {
            next.add_reviewer(team_id, member.clone())?;
        }
        for id in &gone.items {
            let mut team = next.team_mut(team_id).ok_or(PublishError::Lost)?;
            let mut item = team.item_mut(*id).ok_or(PublishError::Lost)?;
            item.replace_review(&gone.task, review.clone())?;
        }
        self.parts.state = next;
        Ok(())
    }

    /// Tells the human that the review of `gone`, a reviewer its provider refused again after its retries, is given to
    /// `replacement`; a reviewer that failed or was stopped is not announced, as before.
    fn tell_replaced(
        &mut self,
        pulls: &Pulls,
        lead: &Lead,
        gone: &Gone,
        replacement: &InvestigatorName,
        now: Timestamp,
    ) {
        let Some(refused) = &gone.refused else {
            return;
        };
        let values = self
            .refused_values(gone, refused)
            .and_then(|values| values.with("replacement", replacement.as_str()));
        self.tell_human(pulls, AutopilotNotice::ReviewerChanged, values, lead, now);
    }

    /// Tells the human, once for a review, that the reviewer of `gone` cannot work and no investigator is free to take
    /// its review.
    async fn nobody_reviews(&mut self, pulls: &Pulls, team_id: &TeamId, lead: &Lead, gone: &Gone, now: Timestamp) {
        if let Some(refused) = &gone.refused {
            self.tell_reviewer_waits(pulls, team_id, lead, gone, refused, now).await;
            return;
        }
        let announcement = Announcement::NoVerdict {
            task: gone.task.clone(),
        };
        if !self.watch.announced.insert(announcement) {
            return;
        }
        tracing::warn!(
            event = LogEvent::ReviewsRequested.as_str(),
            team = %team_id,
            pr = %gone.pr.number,
            reviewer = %gone.reviewer,
            task = %gone.task,
            status = %gone.status,
            "a reviewer cannot work and no investigator is free to take its review: the human is told"
        );
        let why = format!(
            "its task is {} and no other investigator is free to take the review, so the pull request waits.",
            gone.status
        );
        let values = Variables::new()
            .with("reviewer", gone.reviewer.as_str())
            .and_then(|values| values.with("pr", gone.pr.number.to_string()))
            .and_then(|values| values.with("task", gone.task.as_str()))
            .and_then(|values| values.with("why", why));
        self.tell_human(pulls, AutopilotNotice::NoVerdict, values, lead, now);
    }

    /// Tells the human, once for a review, that the reviewer of `gone` waits for its provider because no investigator of
    /// another provider is free to take its review. A notice saved by an earlier life of the daemon counts as told: a
    /// restart does not tell it again.
    async fn tell_reviewer_waits(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        lead: &Lead,
        gone: &Gone,
        refused: &Refused,
        now: Timestamp,
    ) {
        let announcement = Announcement::ReviewerWaits {
            task: gone.task.clone(),
        };
        if !self.watch.announced.insert(announcement) {
            return;
        }
        let told = format!("(task {}) waits: the provider", gone.task);
        match saved_notice_where(&self.notice_directory(), |text| text.contains(&told)).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                // A failed lookup must not hide that a review waits: telling it twice is the lesser harm.
                tracing::warn!(
                    event = LogEvent::NotificationFailed.as_str(),
                    team = %team_id,
                    task = %gone.task,
                    error = %error_chain(&error),
                    "saved notices cannot be inspected; telling that the reviewer waits"
                );
            }
        }
        tracing::warn!(
            event = LogEvent::ReviewsRequested.as_str(),
            team = %team_id,
            pr = %gone.pr.number,
            reviewer = %gone.reviewer,
            task = %gone.task,
            provider = %refused.provider,
            "a reviewer its provider refused waits: no investigator of another provider is free to take its review"
        );
        let values = self.refused_values(gone, refused);
        self.tell_human(pulls, AutopilotNotice::ReviewerWaits, values, lead, now);
    }
}
