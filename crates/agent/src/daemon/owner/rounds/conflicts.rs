//! The merge conflict pass of an approved pull request: the lead merges the base into the branch, the result is gated
//! and pushed, and then the merge is tried again, the change is reviewed first, or the human is asked. The state of the
//! item says where the pass is, so a restart of the daemon goes on from it.

use agent_core::decide::merge_refusal::{
    AccountingState, AfterLead, ConflictAccountingStep, ConflictPublication, ConflictPublishResult,
    ExhaustedRoundRecovery, GatingStatus, LeadEnd, SplitPullStatus, account_conflict_pass, after_lead, changed_by_lead,
    recover_exhausted_split_round,
};
use agent_core::history::CheckpointCause;
use agent_core::history::HistoryEvent;
use agent_core::ids::{ItemId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::repo_path::RepoPath;
use agent_core::state::{
    AdvanceError, CommitId, ConflictInvestigation, ItemMut, ItemPhase, MergeConflict, PullRequest, QuestionState,
    Timestamp,
};
use agent_core::status::TaskStatus;
use agent_runtime::git::{Git, Revision};
use agent_runtime::github::PullState;
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;

use super::super::teams::Look;
use super::super::{Owner, OwnerError};
use super::publish::{ItemTip, Own, Site};
use super::{Lead, PublishError, Pulls};
use crate::daemon::merge::{items_of, ran_since};
use crate::daemon::pr::{Joining, Publication, Publish};
use crate::daemon::team::Why;

/// What the daemon starts on an approved pull request after a conflict refusal or a conflict pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConflictStep {
    /// The lead merges the base branch into the pull request's branch.
    MergeBase,
    /// The daemon asks an investigator about the refused pull request.
    AskInvestigator {
        /// Address of the pull request whose tip must be investigated.
        url: String,
    },
    /// The lead asks the human once in its questions file.
    AskHuman,
}

/// What the advance of a pass decided for this look.
enum Judgement {
    /// Nothing can be decided yet: the lead works, or a gate of the harness runs.
    Wait,
    /// The lead gave no usable result: the human is asked.
    Ask,
    /// The lead's tip passed the gate; it is pushed and the change is read.
    Tip(CommitId),
}

/// The origin of a merge conflict pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConflictPassOrigin {
    /// A fresh merge conflict pass initiated during normal round execution.
    Fresh,
    /// Recovered from an exhausted split round with valid gated work ready.
    Recovered,
}

/// An approved pull request whose merge conflict pass or answered conflict question is in progress.
struct ConflictPass {
    /// The pull request.
    pr: PullRequest,
    /// The head GitHub refused to merge.
    approved_head: CommitId,
    /// When the pass or the human's answer began.
    at: Timestamp,
    /// The items of the pull request.
    items: Vec<ItemId>,
    /// Origin of this conflict pass (fresh or recovered).
    origin: ConflictPassOrigin,
}

/// Context for reporting a conflict publication failure.
struct ConflictPublishSite<'a> {
    /// The site context holding team, lead, and autopilot texts.
    site: Site<'a>,
    /// The publication parameters for the pull request.
    publication: &'a Publication,
    /// The items belonging to the pull request.
    items: &'a [ItemId],
    /// The pull request number.
    pr: agent_core::state::PrNumber,
}

/// Context for pushing and recording a conflict publication.
struct ConflictPublicationTarget<'a> {
    /// The site context holding team, lead, and autopilot texts.
    site: Site<'a>,
    /// The publication parameters for the pull request.
    publication: &'a Publication,
    /// The items belonging to the pull request.
    items: &'a [ItemId],
    /// The pull request to publish to.
    pr: &'a PullRequest,
    /// The approved head GitHub refused to merge.
    approved_head: &'a CommitId,
    /// The gated tip commit to push.
    tip: &'a CommitId,
}

/// Context for applying the results of a lead's conflict pass.
struct ConflictResolution<'a> {
    /// The team whose conflict pass is being applied.
    team: &'a TeamId,
    /// The items belonging to the pull request.
    items: &'a [ItemId],
    /// The pull request number.
    pr: agent_core::state::PrNumber,
    /// The approved head GitHub refused to merge.
    approved_head: &'a CommitId,
    /// The files changed by the lead beyond the merge of the base.
    changed: Vec<RepoPath>,
}

impl Owner {
    /// Sends the lead of `team` to merge the base, or asks the human, and records that on `items`: `head` is the head
    /// GitHub refused, and the pass is recorded before the lead is woken so that a restart sends it again.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved or is found inconsistent.
    pub(super) async fn start_merge_conflict(
        &mut self,
        team: &TeamId,
        items: &[ItemId],
        step: ConflictStep,
        head: &CommitId,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        match step {
            ConflictStep::AskInvestigator { url } => {
                return self.file_conflict_investigations(team, items, head, &url).await;
            }
            ConflictStep::AskHuman => self.each_item(team, items, |item| {
                if let Some(record) = item.conflict_investigation().filter(|record| &record.head == head) {
                    let mut consumed = record.clone();
                    consumed.answered = true;
                    item.set_conflict_investigation(consumed);
                }
                item.set_conflict_approved_head(head.clone());
                if let Some(context) = item.findings_mut().and_then(|ledger| ledger.verification_mut()) {
                    context.reviewed_head = Some(head.clone());
                }
                item.ask_human_about_merge_conflict(now);
                Ok(())
            })?,
            ConflictStep::MergeBase => self.each_item(team, items, |item| {
                item.begin_merge_conflict(head.clone(), now);
                Ok(())
            })?,
        }
        self.parts.state.record_activities(now);
        self.save().await?;
        for item in items {
            if let Ok(round) = self.round_of_item(team, *item) {
                self.queue_checkpoint(now, team, &round, CheckpointCause::MergeRefused, None, None);
            }
        }
        let lead = self.lead_of(team)?;
        self.release_conflict_lead(&lead.task, look)
    }

    /// Files and durably associates each automatic request with its refused head.
    async fn file_conflict_investigations(
        &mut self,
        team: &TeamId,
        items: &[ItemId],
        head: &CommitId,
        url: &str,
    ) -> Result<(), OwnerError> {
        let lead = self.lead_of(team)?;
        let base = self
            .base_branch(&lead.repo)
            .map_err(|source| OwnerError::InvestigationWorkspace(source.into()))?;
        for id in items {
            let item = self
                .parts
                .state
                .team(team)
                .and_then(|found| found.item(*id))
                .ok_or_else(|| OwnerError::LostItem {
                    team: team.clone(),
                    item: *id,
                })?;
            if item.conflict_investigation().is_some_and(|record| &record.head == head) {
                continue;
            }
            let ItemPhase::Approved { pr, .. } = item.phase() else {
                continue;
            };
            let superseded = item.conflict_investigation().map(|record| record.id);
            let request = agent_runtime::agentbus::NewConflictRequest {
                from_agent: lead.agent.clone(),
                from_task: lead.task.clone(),
                repo: lead.repo.clone(),
                branch: pr.branch.clone(),
                item: *id,
                question: format!(
                    "Pull request #{} conflicts with {base} after {} passes of the lead. Resolve it locally as your instructions say.",
                    pr.number,
                    item.merge_passes()
                ),
                pr_url: url.to_owned(),
                pr_number: pr.number,
            };
            if let Some(id) = superseded {
                self.fail_superseded_conflict_request(id).await?;
            }
            let request_id = self
                .parts
                .bus
                .inv_add_conflict(&request)
                .await
                .map_err(|source| OwnerError::Bus { source })?;
            self.each_item(team, &[*id], |item| {
                item.set_conflict_investigation(ConflictInvestigation {
                    head: head.clone(),
                    id: request_id,
                    answered: false,
                });
                Ok(())
            })?;
            self.save().await?;
            tracing::info!(event = LogEvent::MergeConflictInvestigated.as_str(), item = %id, id = request_id.get());
        }
        Ok(())
    }

    /// Finds the requesting team only for requests filed by the automatic conflict path.
    pub(in crate::daemon::owner) fn automatic_conflict_team(
        &self,
        request: &agent_runtime::agentbus::Investigation,
    ) -> Option<TeamId> {
        self.parts
            .state
            .find_task(&request.from_task)
            .filter(|found| {
                found.team.items().iter().any(|item| {
                    item.conflict_investigation()
                        .is_some_and(|record| record.id == request.id)
                })
            })
            .map(|found| found.team.id().clone())
    }

    /// Stops pending work before its head association is replaced.
    async fn fail_superseded_conflict_request(&mut self, id: std::num::NonZeroU32) -> Result<(), OwnerError> {
        use agent_runtime::agentbus::InvestigationStatus;
        let request = self
            .parts
            .bus
            .inv_get(id)
            .await
            .map_err(|source| OwnerError::Bus { source })?;
        match request.status {
            Some(
                InvestigationStatus::Queued
                | InvestigationStatus::Waitroom
                | InvestigationStatus::Starting
                | InvestigationStatus::Running
                | InvestigationStatus::WaitingHuman,
            ) => {
                self.fail_investigation(&request, "the pull request head was superseded", None)
                    .await?;
            }
            Some(
                InvestigationStatus::Done
                | InvestigationStatus::Failed
                | InvestigationStatus::Rejected
                | InvestigationStatus::Unknown(_),
            )
            | None => {}
        }
        Ok(())
    }

    /// Resumes a durable investigation after its answer, failure, or a daemon restart.
    async fn advance_conflict_investigations(
        &mut self,
        team: &TeamId,
        look: &mut Look,
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        use agent_runtime::agentbus::InvestigationStatus;
        let pending: Vec<_> = self
            .parts
            .state
            .team(team)
            .into_iter()
            .flat_map(agent_core::state::Team::items)
            .filter_map(|item| {
                let record = item.conflict_investigation()?;
                (matches!(item.phase(), ItemPhase::Approved { .. })
                    && item.merge_conflict().is_none()
                    && !record.answered)
                    .then(|| (item.id(), record.clone()))
            })
            .collect();
        for (item, mut record) in pending {
            let lead = self.lead_of(team)?;
            let place = self.place_of(team, &[item]);
            let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent);
            let Some(head) = self
                .head_of(&place, team, super::waiting::WaitingStep::ReadMergeHead, None, git)
                .await?
            else {
                continue;
            };
            if head != record.head {
                continue;
            }
            let request = self
                .parts
                .bus
                .inv_get(record.id)
                .await
                .map_err(|source| OwnerError::Bus { source })?;
            if self.parts.settings.config.investigators().is_empty() || !self.is_investigation_requester_live(&request)
            {
                self.fail_investigation(&request, "no investigator or live requester is available", None)
                    .await?;
                self.start_merge_conflict(team, &[item], ConflictStep::AskHuman, &record.head, now, look)
                    .await?;
                continue;
            }
            let step = match request.status {
                Some(InvestigationStatus::Done) => {
                    record.answered = true;
                    self.each_item(team, &[item], |item| {
                        item.set_conflict_investigation(record.clone());
                        Ok(())
                    })?;
                    Some(ConflictStep::MergeBase)
                }
                Some(InvestigationStatus::Failed | InvestigationStatus::Rejected | InvestigationStatus::Unknown(_))
                | None => Some(ConflictStep::AskHuman),
                Some(
                    InvestigationStatus::Queued
                    | InvestigationStatus::Waitroom
                    | InvestigationStatus::Starting
                    | InvestigationStatus::Running
                    | InvestigationStatus::WaitingHuman,
                ) => None,
            };
            if let Some(step) = step {
                self.start_merge_conflict(team, &[item], step, &record.head, now, look)
                    .await?;
            }
        }
        Ok(())
    }

    /// Queues the lead task `task` for a merge conflict pass or question, including when the lead's last run failed.
    fn release_conflict_lead(&mut self, task: &TaskId, look: &mut Look) -> Result<(), OwnerError> {
        let located = self.locate(task)?;
        let status = self
            .parts
            .state
            .find_task(task)
            .map(|found| found.member.status())
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        match status {
            TaskStatus::Waiting | TaskStatus::Failed => {
                self.move_to(task, &located.team, TaskStatus::Queued, &mut look.events)?;
                look.changed = true;
            }
            TaskStatus::Queued => {}
            TaskStatus::Running
            | TaskStatus::Questions
            | TaskStatus::Paused
            | TaskStatus::Stopped
            | TaskStatus::Done => return Ok(()),
        }
        self.reasons.insert(task.clone(), Why::MergeConflict);
        Ok(())
    }

    /// Checks whether the lead of `team` is ready to proceed with a conflict pass.
    fn check_lead_conflict_status(
        &mut self,
        lead: &Lead,
        pass: &ConflictPass,
        look: &mut Look,
    ) -> Result<bool, OwnerError> {
        if pass.origin == ConflictPassOrigin::Recovered {
            let member = self
                .parts
                .state
                .find_task(&lead.task)
                .map(|found| found.member)
                .ok_or_else(|| OwnerError::LostTask {
                    task: lead.task.clone(),
                })?;
            Ok(matches!(member.status(), TaskStatus::Waiting | TaskStatus::Failed)
                && member.current_run().is_none()
                && !member.questions().iter().any(agent_core::state::Question::is_open))
        } else {
            self.check_conflict_lead_ready(lead, pass.at, look)
        }
    }

    /// Looks at the merge conflict pass of `team`, when one is in progress: the lead is sent again when it was not run
    /// since the pass began, the human is asked when it gave no result, and a result is gated, pushed and read.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved or is found inconsistent.
    pub(super) async fn advance_merge_conflicts(
        &mut self,
        pulls: &Pulls,
        team: &TeamId,
        look: &mut Look,
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        self.advance_conflict_investigations(team, look, now).await?;
        let pass = match self.conflict_in_progress(team)? {
            Some(pass) => pass,
            None => match self.recover_exhausted_split_conflict(pulls, team).await {
                Ok(Some(recovered)) => recovered,
                Ok(None) => return self.wake_unasked_conflict_lead(team, look).await,
                Err(error) => {
                    tracing::warn!(
                        event = LogEvent::OutsideCallCompleted.as_str(),
                        team = %team,
                        error = %error_chain(&error),
                        "recovery of exhausted split conflict failed: waiting for next pass"
                    );
                    return Ok(());
                }
            },
        };
        if self.conflict_publication_delayed(team, &pass.pr, now) {
            return Ok(());
        }
        let lead = self.lead_of(team)?;
        if !self.check_lead_conflict_status(&lead, &pass, look)? {
            return Ok(());
        }
        if pass.origin != ConflictPassOrigin::Recovered && !lead.done {
            return self
                .handle_undone_lead(team, &pass.items, pass.pr.number, &pass.approved_head, now, look)
                .await;
        }
        let ConflictPass {
            pr,
            approved_head,
            items,
            ..
        } = pass;
        let publication = conflict_publication(&pr.branch, &items);
        let site = Site {
            pulls,
            lead: &lead,
            team,
        };
        if self.recover_publish_question(site, &publication, now).await? {
            return Ok(());
        }
        let Some(tip) = self
            .evaluate_conflict_tip(&site, &items, pr.number, &approved_head, now, look)
            .await?
        else {
            return Ok(());
        };
        if !self
            .publish_and_record_conflict(
                ConflictPublicationTarget {
                    site,
                    publication: &publication,
                    items: &items,
                    pr: &pr,
                    approved_head: &approved_head,
                    tip: &tip,
                },
                now,
                look,
            )
            .await?
        {
            return Ok(());
        }
        let changed = match self.read_conflict_changes(&lead, &pr, &approved_head, &tip).await {
            Ok(changed) => changed,
            Err(error) => {
                return self.publish_failed(site, &publication, &error, now, look).await;
            }
        };
        self.apply_lead_change(
            ConflictResolution {
                team,
                items: &items,
                pr: pr.number,
                approved_head: &approved_head,
                changed,
            },
            now,
            look,
        )
        .await
    }

    /// Handles an undone conflict lead by logging the decision, refunding the pass and escalating to the human.
    async fn handle_undone_lead(
        &mut self,
        team: &TeamId,
        items: &[ItemId],
        pr: agent_core::state::PrNumber,
        approved_head: &CommitId,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let allowances = self.team_allowances(team)?;
        let decided = after_lead(&LeadEnd::NoResult, &allowances);
        log_decision(
            "after_lead",
            &format!("pr={pr}, end=no_result"),
            &format!("{decided:?}"),
        );
        self.refund_conflict_pass_on_exit(team, items, pr, "failed_no_result")?;
        self.start_merge_conflict(team, items, ConflictStep::AskHuman, approved_head, now, look)
            .await
    }

    /// Evaluates the conflict resolution tip, returning the commit ID if valid or escalating if unusable.
    async fn evaluate_conflict_tip(
        &mut self,
        site: &Site<'_>,
        items: &[ItemId],
        pr: agent_core::state::PrNumber,
        approved_head: &CommitId,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<Option<CommitId>, OwnerError> {
        let judged = match self.judge_tip(site.lead, approved_head).await {
            Ok(judged) => judged,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::GitCallFailed.as_str(),
                    team = %site.team,
                    pr = %pr,
                    error = %error_chain(&error),
                    "the merge conflict pass cannot be read yet: it is looked at again in a while"
                );
                return Ok(None);
            }
        };
        match judged {
            Judgement::Wait => Ok(None),
            Judgement::Ask => {
                self.refund_conflict_pass_on_exit(site.team, items, pr, "failed_unusable_tip")?;
                self.start_merge_conflict(site.team, items, ConflictStep::AskHuman, approved_head, now, look)
                    .await?;
                Ok(None)
            }
            Judgement::Tip(tip) => Ok(Some(tip)),
        }
    }

    /// Checks whether the conflict lead has completed a run since the pass began.
    fn check_conflict_lead_ready(&mut self, lead: &Lead, at: Timestamp, look: &mut Look) -> Result<bool, OwnerError> {
        let member = self
            .parts
            .state
            .find_task(&lead.task)
            .map(|found| found.member)
            .ok_or_else(|| OwnerError::LostTask {
                task: lead.task.clone(),
            })?;
        if !matches!(member.status(), TaskStatus::Waiting | TaskStatus::Failed) {
            return Ok(false);
        }
        if !ran_since(member, at) {
            self.release_conflict_lead(&lead.task, look)?;
            return Ok(false);
        }
        Ok(true)
    }

    /// Pushes the conflict resolution and records the publication upon success.
    async fn publish_and_record_conflict(
        &mut self,
        target: ConflictPublicationTarget<'_>,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<bool, OwnerError> {
        let ConflictPublicationTarget {
            site,
            publication,
            items,
            pr,
            approved_head,
            tip,
        } = target;
        if let Err(error) = self.push_conflict_resolution(site.lead, pr, approved_head, tip).await {
            self.handle_conflict_publish_failure(
                ConflictPublishSite {
                    site,
                    publication,
                    items,
                    pr: pr.number,
                },
                &error,
                now,
                look,
            )
            .await?;
            return Ok(false);
        }
        self.record_conflict_published(site.team, items, approved_head, pr.number)?;
        self.each_item(site.team, items, |item| {
            if let Some(mut record) = item.conflict_investigation().cloned()
                && record.answered
            {
                // The answer's published tip has already used this head's investigation.
                record.head = tip.clone();
                item.set_conflict_investigation(record);
            }
            Ok(())
        })?;
        self.record_event_at(
            now,
            site.team,
            self.rounds_of_items(site.team, items),
            HistoryEvent::PullPushed {
                pr: pr.number,
                head: tip.clone(),
            },
        );
        Ok(true)
    }

    /// Records publication of a merge conflict pass on `items` of `team`.
    fn record_conflict_published(
        &mut self,
        team: &TeamId,
        items: &[ItemId],
        approved_head: &CommitId,
        pr: agent_core::state::PrNumber,
    ) -> Result<(), OwnerError> {
        self.each_item(team, items, |item| {
            let accounting = if item.conflict_accounted_head() == Some(approved_head) {
                AccountingState::AlreadyRecorded
            } else {
                AccountingState::Unrecorded
            };
            let step = account_conflict_pass(ConflictPublishResult::Published, accounting);
            log_decision(
                "account_conflict_pass",
                &format!("pr={pr}, result=published, accounting={accounting:?}"),
                &format!("{step:?}"),
            );
            if step == ConflictAccountingStep::SpendPass {
                item.record_published_conflict_pass();
            }
            Ok(())
        })
    }

    /// Refunds a merge conflict pass upon an unpublished exit.
    fn refund_conflict_pass_on_exit(
        &mut self,
        team: &TeamId,
        items: &[ItemId],
        pr: agent_core::state::PrNumber,
        reason: &'static str,
    ) -> Result<(), OwnerError> {
        self.each_item(team, items, |item| {
            let accounting = if item.has_provisional_conflict_pass() {
                AccountingState::Unrecorded
            } else {
                AccountingState::AlreadyRecorded
            };
            let step = account_conflict_pass(ConflictPublishResult::Failed, accounting);
            log_decision(
                "account_conflict_pass",
                &format!("pr={pr}, result={reason}, accounting={accounting:?}"),
                &format!("{step:?}"),
            );
            if step == ConflictAccountingStep::RetainBudget && accounting == AccountingState::Unrecorded {
                item.refund_conflict_pass();
            }
            Ok(())
        })
    }

    /// Handles a conflict pass publication failure by refunding the pass and delegating to `publish_failed`.
    async fn handle_conflict_publish_failure(
        &mut self,
        target: ConflictPublishSite<'_>,
        error: &PublishError,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        self.refund_conflict_pass_on_exit(target.site.team, target.items, target.pr, "failed")?;
        self.publish_failed(target.site, target.publication, error, now, look)
            .await
    }

    /// Holds a rejected conflict publication until its shared retry backoff has elapsed.
    fn conflict_publication_delayed(&self, team: &TeamId, pr: &PullRequest, now: Timestamp) -> bool {
        self.watch
            .retry_at
            .get(&(team.clone(), pr.branch.clone()))
            .is_some_and(|retry| *retry > now)
    }

    /// Wakes the lead of `team` once when a merge conflict pass needs the human and the lead has not run since, or
    /// clears a legacy conflict question without a recorded approved head once the human's answer has been acted on.
    async fn wake_unasked_conflict_lead(&mut self, team: &TeamId, look: &mut Look) -> Result<(), OwnerError> {
        let Some((pr, at, items)) = self.conflict_asking_human(team) else {
            return Ok(());
        };
        let lead = self.lead_of(team)?;
        let member = self
            .parts
            .state
            .find_task(&lead.task)
            .map(|found| found.member)
            .ok_or_else(|| OwnerError::LostTask {
                task: lead.task.clone(),
            })?;
        if !ran_since(member, at) {
            if matches!(member.status(), TaskStatus::Waiting | TaskStatus::Failed) {
                return self.release_conflict_lead(&lead.task, look);
            }
            return Ok(());
        }
        let answered_at = member
            .questions()
            .iter()
            .filter_map(|question| match question.state() {
                QuestionState::Answered { at: answered_at, .. } if *answered_at >= at => Some(*answered_at),
                QuestionState::Answered { .. } | QuestionState::Open => None,
            })
            .max();
        if let Some(answered_at) = answered_at
            && member.status() == TaskStatus::Waiting
            && ran_since(member, answered_at)
        {
            self.end_pass(team, &items, pr.number)?;
            self.save().await?;
        }
        Ok(())
    }

    /// The task allowances of `team`.
    fn team_allowances(&self, team: &TeamId) -> Result<Vec<agent_core::task_allowance::TaskAllowance>, OwnerError> {
        Ok(self
            .parts
            .state
            .team(team)
            .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?
            .allowances()
            .to_vec())
    }

    /// Acts on the `changed` files pushed by the lead's conflict pass for `pr` of `team`.
    async fn apply_lead_change(
        &mut self,
        resolution: ConflictResolution<'_>,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let ConflictResolution {
            team,
            items,
            pr,
            approved_head,
            changed,
        } = resolution;
        let allowances = self.team_allowances(team)?;
        let end = LeadEnd::Result { changed };
        let decided = after_lead(&end, &allowances);
        log_decision("after_lead", &format!("pr={pr}, end={end:?}"), &format!("{decided:?}"));
        self.each_item(team, items, |item| {
            item.reset_publish_tries();
            item.clear_publish_question();
            Ok(())
        })?;
        match decided {
            AfterLead::RetryMerge => {
                self.end_pass(team, items, pr)?;
                self.save().await?;
            }
            AfterLead::VerifyFirst => {
                self.end_pass(team, items, pr)?;
                self.each_item(team, items, |item: &mut ItemMut<'_>| {
                    if let Some(context) = item.findings_mut().and_then(|ledger| ledger.verification_mut()) {
                        context.reviewed_head = Some(approved_head.clone());
                    }
                    item.reopen_review_after_conflict()
                })?;
                self.save().await?;
            }
            AfterLead::AskHuman => {
                return self
                    .start_merge_conflict(team, items, ConflictStep::AskHuman, approved_head, now, look)
                    .await;
            }
        }
        Ok(())
    }

    /// When an approved pull request of `team` asked the human about a merge conflict, if one did.
    fn conflict_asking_human(&self, team: &TeamId) -> Option<(PullRequest, Timestamp, Vec<ItemId>)> {
        let found = self.parts.state.team(team)?;
        found
            .items()
            .iter()
            .find_map(|item| match (item.phase(), item.merge_conflict()) {
                (ItemPhase::Approved { pr, .. }, Some(MergeConflict::AskingHuman { at })) => {
                    Some((pr.clone(), *at, items_of(found, pr.number)))
                }
                (ItemPhase::Approved { .. }, Some(MergeConflict::MergingBase { .. }) | None)
                | (
                    ItemPhase::Pending
                    | ItemPhase::InProgress { .. }
                    | ItemPhase::InReview { .. }
                    | ItemPhase::Fixing { .. }
                    | ItemPhase::Done { .. },
                    _,
                ) => None,
            })
    }

    /// The approved pull request of `team` whose merge conflict pass is in progress, with the head the pass is for, when
    /// the pass began and its items.
    fn conflict_in_progress(&self, team: &TeamId) -> Result<Option<ConflictPass>, OwnerError> {
        let Some(found) = self.parts.state.team(team) else {
            return Ok(None);
        };
        let lead = self.lead_of(team)?;
        let questions = self
            .parts
            .state
            .find_task(&lead.task)
            .map(|task| task.member.questions())
            .ok_or_else(|| OwnerError::LostTask {
                task: lead.task.clone(),
            })?;
        Ok(found
            .items()
            .iter()
            .find_map(|item| match (item.phase(), item.merge_conflict()) {
                (ItemPhase::Approved { pr, .. }, Some(MergeConflict::MergingBase { approved_head, at })) => {
                    Some(ConflictPass {
                        pr: pr.clone(),
                        approved_head: approved_head.clone(),
                        at: *at,
                        items: items_of(found, pr.number),
                        origin: ConflictPassOrigin::Fresh,
                    })
                }
                (ItemPhase::Approved { pr, .. }, Some(MergeConflict::AskingHuman { at })) => {
                    let answered_at = questions
                        .iter()
                        .filter_map(|question| match question.state() {
                            QuestionState::Answered { at: answered_at, .. } if *answered_at >= *at => {
                                Some(*answered_at)
                            }
                            QuestionState::Answered { .. } | QuestionState::Open => None,
                        })
                        .max();
                    let approved_head = item.conflict_approved_head().cloned().or_else(|| {
                        item.findings()
                            .and_then(|ledger| ledger.verification())
                            .and_then(|context| context.reviewed_head.clone())
                    });
                    if let Some(answered_at) = answered_at
                        && let Some(approved_head) = approved_head
                    {
                        return Some(ConflictPass {
                            pr: pr.clone(),
                            approved_head,
                            at: answered_at,
                            items: items_of(found, pr.number),
                            origin: ConflictPassOrigin::Fresh,
                        });
                    }
                    None
                }
                (ItemPhase::Approved { .. }, None)
                | (
                    ItemPhase::Pending
                    | ItemPhase::InProgress { .. }
                    | ItemPhase::InReview { .. }
                    | ItemPhase::Fixing { .. }
                    | ItemPhase::Done { .. },
                    _,
                ) => None,
            }))
    }

    /// Evaluates the split pull status for recovery from real git, gate record, and GitHub state.
    async fn evaluate_split_pull_status(
        &self,
        lead: &Lead,
        team: &TeamId,
        pr: &PullRequest,
        remote: &agent_runtime::github::RemotePullRequest,
    ) -> Result<SplitPullStatus, PublishError> {
        match remote.state {
            PullState::Merged => Ok(SplitPullStatus::Merged),
            PullState::Closed => Ok(SplitPullStatus::ClosedUnmerged),
            PullState::Open => {
                let budget = self.conflict_budget(team, pr.number);
                let tip = self.item_tip(lead, &pr.branch).await?.tip;
                let record = self.parts.store.load_gate_record(&lead.task).await?;
                let is_gated = record
                    .as_ref()
                    .is_some_and(|r| r.result.head == tip && r.counts_as_pass());
                let gating = if is_gated {
                    GatingStatus::Gated
                } else {
                    GatingStatus::Ungated
                };
                let conflict = if tip == remote.head {
                    ConflictPublication::Published
                } else {
                    ConflictPublication::Unpublished
                };
                Ok(SplitPullStatus::Open {
                    gating,
                    budget,
                    conflict,
                })
            }
        }
    }

    /// Recovers an exhausted split round in `AskingHuman` when valid gated work is ready.
    async fn recover_exhausted_split_conflict(
        &mut self,
        pulls: &Pulls,
        team: &TeamId,
    ) -> Result<Option<ConflictPass>, PublishError> {
        let Some((pr, at, items)) = self.conflict_asking_human(team) else {
            return Ok(None);
        };
        let Some(found) = self.parts.state.team(team) else {
            return Ok(None);
        };
        let is_split_part = found.rounds().iter().any(|r| {
            r.part_pull_request().is_some_and(|p| p.number == pr.number)
                || (r.part().is_some() && items.iter().any(|id| r.item() == Some(*id)))
        });
        if !is_split_part {
            return Ok(None);
        }
        let lead = self.lead_of(team)?;
        let member = self
            .parts
            .state
            .find_task(&lead.task)
            .map(|t| t.member)
            .ok_or_else(|| OwnerError::LostTask {
                task: lead.task.clone(),
            })?;
        if !matches!(member.status(), TaskStatus::Waiting | TaskStatus::Failed)
            || member.current_run().is_some()
            || member.questions().iter().any(agent_core::state::Question::is_open)
        {
            return Ok(None);
        }
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let github = self.github_for(pulls, &lead.repo, &git).await?;
        let remote = github.get_pull_request(pr.number).await?;
        let status = self.evaluate_split_pull_status(&lead, team, &pr, &remote).await?;
        let recovery = recover_exhausted_split_round(status);
        log_decision(
            "recover_exhausted_split_round",
            &format!("pr={}, status={status:?}", pr.number),
            &format!("{recovery:?}"),
        );
        match recovery {
            ExhaustedRoundRecovery::WaitHuman | ExhaustedRoundRecovery::CompleteMerged => Ok(None),
            ExhaustedRoundRecovery::ResumeMerge => {
                if let SplitPullStatus::Open {
                    conflict: ConflictPublication::Published,
                    ..
                } = status
                {
                    self.end_pass(team, &items, pr.number)?;
                    self.save().await?;
                    Ok(None)
                } else {
                    let mut approved_head = self
                        .parts
                        .state
                        .team(team)
                        .and_then(|t| items.first().and_then(|id| t.item(*id)))
                        .and_then(|item| item.conflict_approved_head().cloned());
                    if approved_head.is_none() {
                        let fallback = remote.head.clone();
                        self.each_item(team, &items, |item| {
                            item.set_conflict_approved_head(fallback.clone());
                            Ok(())
                        })?;
                        self.save().await?;
                        approved_head = Some(fallback);
                    }
                    let approved_head = approved_head.unwrap_or_else(|| remote.head.clone());
                    Ok(Some(ConflictPass {
                        pr,
                        approved_head,
                        at,
                        items,
                        origin: ConflictPassOrigin::Recovered,
                    }))
                }
            }
        }
    }

    /// Whether the lead's tip of `pr` is the commit to push: it merged the base, and the gate of the harness or of the
    /// lead passed at it.
    ///
    /// The tip is the tip of the branch of the pull request the pass is for, whatever the worktree has out; the worktree's
    /// own `HEAD` is read only when no item of the lead's team has a pass for `approved_head`.
    async fn judge_tip(&mut self, lead: &Lead, approved_head: &CommitId) -> Result<Judgement, PublishError> {
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let base = self.base_branch(&lead.repo)?;
        let branch = self.parts.state.find_task(&lead.task).and_then(|found| {
            found.team.items().iter().find_map(|item| {
                if let ItemPhase::Approved { pr, .. } = item.phase()
                    && item.conflict_approved_head() == Some(approved_head)
                {
                    Some(pr.branch.clone())
                } else {
                    None
                }
            })
        });
        let ItemTip { tip, checked_out } = match branch {
            Some(branch) => self.item_tip(lead, &branch).await?,
            None => ItemTip {
                tip: git.rev_parse(&Revision::Head).await?,
                checked_out: true,
            },
        };
        if &tip == approved_head
            || !git
                .is_ancestor(&Revision::RemoteBranch(base), &Revision::Commit(tip.clone()))
                .await?
        {
            return Ok(Judgement::Ask);
        }
        let own = self.own_gate_state(&lead.task, &tip);
        if own == Own::Running {
            return Ok(Judgement::Wait);
        }
        // Completion guarantees the gate record is saved; read it after consuming that completion.
        let record = self.parts.store.load_gate_record(&lead.task).await?;
        if record
            .as_ref()
            .is_some_and(|record| record.result.head == tip && record.counts_as_pass())
        {
            return Ok(Judgement::Tip(tip));
        }
        match own {
            Own::Running => Ok(Judgement::Wait),
            // The gate of the harness runs in the worktree: it does not see the tip of a branch the worktree has not out.
            Own::NotRun if !checked_out => Ok(Judgement::Ask),
            Own::NotRun => {
                self.start_own_gate(&lead.task, &tip, false);
                Ok(Judgement::Wait)
            }
            // A gate that did not pass at the tip is the human's to see: the lead gave no usable result.
            Own::Ran { .. } => Ok(Judgement::Ask),
        }
    }

    /// Pushes `tip` to the branch of the pull request, confirming that the conflict resolution was published.
    async fn push_conflict_resolution(
        &mut self,
        lead: &Lead,
        pr: &PullRequest,
        approved_head: &CommitId,
        tip: &CommitId,
    ) -> Result<(), PublishError> {
        let git: Git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let branches = agent_core::decide::publish::PublicationBranches {
            task: &pr.branch,
            part: None,
            pr: Some(&pr.branch),
        };
        let remote_state = if approved_head == tip {
            agent_core::decide::publish::RemoteState::Equal
        } else if git
            .is_ancestor(&Revision::Commit(approved_head.clone()), &Revision::Commit(tip.clone()))
            .await?
        {
            agent_core::decide::publish::RemoteState::Behind
        } else {
            agent_core::decide::publish::RemoteState::Diverged
        };
        let decided = agent_core::decide::publish::publish_decision(
            agent_core::decide::publish::RoundPublication::ConflictPass,
            &branches,
            remote_state,
        );
        match decided.action {
            agent_core::decide::publish::PublishAction::UpToDate => {
                git.adopt_pushed(decided.branch, tip).await?;
            }
            agent_core::decide::publish::PublishAction::Push => {
                git.push_commit(decided.branch, tip).await?;
            }
            agent_core::decide::publish::PublishAction::Refused => {
                return Err(PublishError::Refused {
                    branch: decided.branch.clone(),
                });
            }
        }
        Ok(())
    }

    /// Reads the files changed by the lead beyond the merge of the base.
    async fn read_conflict_changes(
        &mut self,
        lead: &Lead,
        pr: &PullRequest,
        approved_head: &CommitId,
        tip: &CommitId,
    ) -> Result<Vec<RepoPath>, PublishError> {
        let git: Git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        git.branch_point_if_unheld(&pr.branch, tip).await?;
        let base = self.base_branch(&lead.repo)?;
        let merged_base = git
            .merge_base(&Revision::Commit(tip.clone()), &Revision::RemoteBranch(base))
            .await?;
        let since_point = git
            .merge_base(
                &Revision::Commit(approved_head.clone()),
                &Revision::Commit(merged_base.clone()),
            )
            .await?;
        let paths = |changes: Vec<agent_core::decide::checks::FileChange>| -> Vec<RepoPath> {
            changes.into_iter().map(|change| change.path).collect()
        };
        let on_branch = paths(git.range_changes(&since_point, approved_head).await?);
        let vs_base = paths(git.range_changes(&merged_base, tip).await?);
        let since = paths(git.range_changes(approved_head, tip).await?);
        Ok(changed_by_lead(&since, &on_branch, &vs_base))
    }

    /// Ends the pass of `items`: the pull request is tried again at once, and the passes spent stay counted.
    fn end_pass(
        &mut self,
        team: &TeamId,
        items: &[ItemId],
        number: agent_core::state::PrNumber,
    ) -> Result<(), OwnerError> {
        self.each_item(team, items, |item| {
            item.end_merge_conflict();
            Ok(())
        })?;
        let key = (team.clone(), number);
        self.watch.refused.remove(&key);
        self.watch.polled.remove(&key);
        Ok(())
    }

    /// Changes each of `items` of `team` with `change`.
    pub(in crate::daemon::owner) fn each_item(
        &mut self,
        team: &TeamId,
        items: &[ItemId],
        mut change: impl FnMut(&mut ItemMut<'_>) -> Result<(), AdvanceError>,
    ) -> Result<(), OwnerError> {
        for id in items {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team)
                .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?;
            let mut item = team_mut.item_mut(*id).ok_or_else(|| OwnerError::LostItem {
                team: team.clone(),
                item: *id,
            })?;
            change(&mut item).map_err(|source| OwnerError::Item {
                team: team.clone(),
                item: *id,
                source,
            })?;
        }
        Ok(())
    }
}

/// The publication configuration for a conflict pass on `branch` covering `items`.
fn conflict_publication(branch: &agent_core::ids::BranchName, items: &[ItemId]) -> Publication {
    Publication {
        head_branch: branch.clone(),
        publish: Publish::Open {
            items: items
                .iter()
                .map(|id| Joining {
                    item: *id,
                    branch: branch.clone(),
                })
                .collect(),
        },
    }
}

#[cfg(test)]
mod tests;
