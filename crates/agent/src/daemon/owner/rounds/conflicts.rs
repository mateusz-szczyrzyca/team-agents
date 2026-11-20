//! The merge conflict pass of an approved pull request: the lead merges the base into the branch, the result is gated
//! and pushed, and then the merge is tried again, the change is reviewed first, or the human is asked. The state of the
//! item says where the pass is, so a restart of the daemon goes on from it.

use agent_core::decide::merge_refusal::{AfterLead, LeadEnd, after_lead, changed_by_lead};
use agent_core::history::HistoryEvent;
use agent_core::ids::{ItemId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::repo_path::RepoPath;
use agent_core::state::{
    AdvanceError, CommitId, ItemMut, ItemPhase, MergeConflict, PullRequest, QuestionState, Timestamp,
};
use agent_core::status::TaskStatus;
use agent_runtime::git::{Git, Revision};
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;

use super::super::teams::Look;
use super::super::{Owner, OwnerError};
use super::publish::Own;
use super::{Lead, PublishError};
use crate::daemon::merge::{items_of, ran_since};
use crate::daemon::team::Why;

/// What the daemon starts on an approved pull request after a conflict refusal or a conflict pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConflictStep {
    /// The lead merges the base branch into the pull request's branch.
    MergeBase,
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
        self.each_item(team, items, |item| {
            match step {
                ConflictStep::AskHuman => {
                    item.set_conflict_approved_head(head.clone());
                    if let Some(context) = item.findings_mut().and_then(|ledger| ledger.verification_mut()) {
                        context.reviewed_head = Some(head.clone());
                    }
                    item.ask_human_about_merge_conflict(now);
                }
                ConflictStep::MergeBase => {
                    item.begin_merge_conflict(head.clone(), now);
                }
            }
            Ok(())
        })?;
        self.parts.state.record_activities(now);
        self.save().await?;
        let lead = self.lead_of(team)?;
        self.release_conflict_lead(&lead.task, look)
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

    /// Looks at the merge conflict pass of `team`, when one is in progress: the lead is sent again when it was not run
    /// since the pass began, the human is asked when it gave no result, and a result is gated, pushed and read.
    ///
    /// # Errors
    /// [`OwnerError`] when the state cannot be saved or is found inconsistent.
    pub(super) async fn advance_merge_conflicts(
        &mut self,
        team: &TeamId,
        look: &mut Look,
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        let Some(ConflictPass {
            pr,
            approved_head,
            at,
            items,
        }) = self.conflict_in_progress(team)?
        else {
            return self.wake_unasked_conflict_lead(team, look).await;
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
        if !matches!(member.status(), TaskStatus::Waiting | TaskStatus::Failed) {
            return Ok(());
        }
        if !ran_since(member, at) {
            return self.release_conflict_lead(&lead.task, look);
        }
        if !lead.done {
            let allowances = self.team_allowances(team)?;
            let decided = after_lead(&LeadEnd::NoResult, &allowances);
            log_decision(
                "after_lead",
                &format!("pr={}, end=no_result", pr.number),
                &format!("{decided:?}"),
            );
            return self
                .start_merge_conflict(team, &items, ConflictStep::AskHuman, &approved_head, now, look)
                .await;
        }
        let judged = match self.judge_tip(&lead, &pr, &approved_head).await {
            Ok(judged) => judged,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::GitCallFailed.as_str(),
                    team = %team,
                    pr = %pr.number,
                    error = %error_chain(&error),
                    "the merge conflict pass cannot be read yet: it is looked at again in a while"
                );
                return Ok(());
            }
        };
        let tip = match judged {
            Judgement::Wait => return Ok(()),
            Judgement::Ask => {
                return self
                    .start_merge_conflict(team, &items, ConflictStep::AskHuman, &approved_head, now, look)
                    .await;
            }
            Judgement::Tip(tip) => tip,
        };
        let changed = match self.read_change(&lead, &pr, &approved_head, &tip).await {
            Ok(changed) => changed,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::GitCallFailed.as_str(),
                    team = %team,
                    pr = %pr.number,
                    error = %error_chain(&error),
                    "the change of the merge conflict pass cannot be read yet: it is looked at again in a while"
                );
                return Ok(());
            }
        };
        self.record_event_at(
            now,
            team,
            self.rounds_of_items(team, &items),
            HistoryEvent::PullPushed {
                pr: pr.number,
                head: tip,
            },
        );
        self.apply_lead_change(team, &items, pr.number, &approved_head, changed, now, look)
            .await
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
    #[expect(
        clippy::too_many_arguments,
        reason = "the team, its items, the pull request, the approved head, the changed files and the look are distinct inputs of the step"
    )]
    async fn apply_lead_change(
        &mut self,
        team: &TeamId,
        items: &[ItemId],
        pr: agent_core::state::PrNumber,
        approved_head: &CommitId,
        changed: Vec<RepoPath>,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let allowances = self.team_allowances(team)?;
        let end = LeadEnd::Result { changed };
        let decided = after_lead(&end, &allowances);
        log_decision("after_lead", &format!("pr={pr}, end={end:?}"), &format!("{decided:?}"));
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
                        .max()?;
                    let approved_head = item.conflict_approved_head().cloned().or_else(|| {
                        item.findings()
                            .and_then(|ledger| ledger.verification())
                            .and_then(|context| context.reviewed_head.clone())
                    })?;
                    Some(ConflictPass {
                        pr: pr.clone(),
                        approved_head,
                        at: answered_at,
                        items: items_of(found, pr.number),
                    })
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

    /// Whether the lead's tip of `pr` is the commit to push: it merged the base, and the gate of the harness or of the
    /// lead passed at it.
    async fn judge_tip(
        &mut self,
        lead: &Lead,
        pr: &PullRequest,
        approved_head: &CommitId,
    ) -> Result<Judgement, PublishError> {
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let base = self.base_branch(&lead.repo)?;
        let Some(tip) = git.branch_tip(&pr.branch).await? else {
            return Ok(Judgement::Ask);
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
        let record = self.parts.store.load_gate_record(&lead.task).await?;
        if record
            .as_ref()
            .is_some_and(|record| record.result.head == tip && record.counts_as_pass())
        {
            return Ok(Judgement::Tip(tip));
        }
        match own {
            Own::Running => Ok(Judgement::Wait),
            Own::NotRun => {
                self.start_own_gate(&lead.task, &tip, false);
                Ok(Judgement::Wait)
            }
            // A gate that did not pass at the tip is the human's to see: the lead gave no usable result.
            Own::Ran { .. } => Ok(Judgement::Ask),
        }
    }

    /// Pushes `tip` to the branch of the pull request and reads the files the lead changed after the merge of the base.
    async fn read_change(
        &mut self,
        lead: &Lead,
        pr: &PullRequest,
        approved_head: &CommitId,
        tip: &CommitId,
    ) -> Result<Vec<RepoPath>, PublishError> {
        let git: Git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        git.push(&pr.branch).await?;
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
    fn each_item(
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
