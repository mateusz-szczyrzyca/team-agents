//! The reviewers: drawing two free investigators for a pull request, giving them worktrees at its head, and asking them
//! for their reviews.

use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::history::HistoryEvent;
use agent_core::ids::{AgentName, BranchName, ItemId, TaskId, TeamId};
use agent_core::state::{CommitId, ItemPhase, Member, Participant, PullRequest, Review, ReviewOutcome, Timestamp};
use agent_runtime::git::{Git, Revision, WorktreeBranch};
use agent_runtime::process::error_chain;

use super::super::teams::Look;
use crate::daemon::merge::{count, review_deliveries};
use crate::daemon::review::{Reviewer, Slot, choose_reviewers, free_reviewers};
use crate::daemon::team::{Why, task_branch};

use super::super::{Owner, OwnerError};
use super::{Lead, PublishError};

/// A review of the task `task` that waits for its outcome, asked at `now`.
pub(super) fn pending_review(reviewer: &agent_core::ids::InvestigatorName, task: &TaskId, now: Timestamp) -> Review {
    Review {
        reviewer: reviewer.clone(),
        task: task.clone(),
        requested_at: now,
        outcome: ReviewOutcome::Pending,
    }
}

impl Owner {
    /// Asks two free investigators from outside the team for the reviews of each pull request that has none asked.
    pub(super) async fn ask_for_reviews(&mut self, team_id: &TeamId, look: &mut Look) -> Result<(), OwnerError> {
        let waiting = self.pull_requests_without_reviews(team_id);
        if waiting.is_empty() {
            return Ok(());
        }
        let lead = self.lead_of(team_id)?;
        for (pr, items) in waiting {
            let now = Timestamp::now(&self.parts.clock)?;
            match self.try_ask(team_id, &lead, &pr, &items, now).await {
                Ok(true) => look.changed = true,
                // Fewer than two investigators are free: the pull request waits for them.
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(team = %team_id, pr = %pr.number, error = %error_chain(&error), "the reviews of a pull request cannot be asked for: it is tried again at the next look");
                }
            }
        }
        Ok(())
    }

    /// The pull requests of `team` that are open and have no review asked, with their items.
    pub(super) fn pull_requests_without_reviews(&self, team: &TeamId) -> Vec<(PullRequest, Vec<ItemId>)> {
        let mut found: Vec<(PullRequest, Vec<ItemId>)> = Vec::new();
        let Some(team) = self.parts.state.team(team) else {
            return found;
        };
        for progress in team.items() {
            let ItemPhase::InReview { pr, reviews } = progress.phase() else {
                continue;
            };
            if !reviews.is_empty() {
                continue;
            }
            match found.iter_mut().find(|(known, _)| known.number == pr.number) {
                Some((_, items)) => items.push(progress.id()),
                None => found.push((pr.clone(), vec![progress.id()])),
            }
        }
        found
    }

    /// Asks two reviewers for the reviews of `pr`; `false` when fewer than two are free.
    ///
    /// The worktrees of the reviewers are made, or moved to the commit under review, before the state records the
    /// reviews, so that a reviewer that is queued has a worktree to work in; the ones that were made are removed again
    /// when something later fails, and the state changes as a whole or not at all.
    pub(super) async fn try_ask(
        &mut self,
        team_id: &TeamId,
        lead: &Lead,
        pr: &PullRequest,
        items: &[ItemId],
        now: Timestamp,
    ) -> Result<bool, PublishError> {
        let free = free_reviewers(&self.parts.state, &self.parts.settings.config, team_id);
        let Some(chosen) = choose_reviewers(free, &mut self.parts.rng) else {
            return Ok(false);
        };
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let Some(head) = git.branch_tip(&pr.branch).await? else {
            return Ok(false);
        };
        let mut prepared = Prepared::default();
        for reviewer in chosen {
            if let Err(error) = self
                .prepare_reviewer(&git, lead, reviewer, &head, now, &mut prepared)
                .await
            {
                self.undo(&git, &prepared.made).await;
                return Err(error);
            }
        }
        if let Err(error) = self.record_reviews(team_id, items, &prepared) {
            self.undo(&git, &prepared.made).await;
            return Err(error);
        }
        let pass = count(self.fix_passes_of(team_id, items));
        let rounds = self.rounds_of_items(team_id, items);
        for review in &prepared.reviews {
            self.record_event(
                team_id,
                rounds.clone(),
                HistoryEvent::ReviewAsked {
                    pr: pr.number,
                    pass,
                    head: head.clone(),
                    reviewer: review.reviewer.clone(),
                    task: review.task.clone(),
                },
            );
        }
        tracing::info!(team = %team_id, pr = %pr.number, reviewers = prepared.reviews.len(), "reviews are asked for");
        Ok(true)
    }

    /// Gets the worktree of `reviewer` ready for `head`: its own is moved there, or a new task with a worktree is made.
    async fn prepare_reviewer(
        &mut self,
        git: &Git,
        lead: &Lead,
        reviewer: Reviewer,
        head: &CommitId,
        now: Timestamp,
        prepared: &mut Prepared,
    ) -> Result<(), PublishError> {
        let agent: AgentName = reviewer.name.as_str().parse()?;
        let task = match reviewer.slot {
            Slot::Task(task) => {
                self.worktree_git(&lead.repo, &task, &agent)?.detach(head).await?;
                task
            }
            Slot::New => {
                let task = TaskId::generate(&self.parts.clock, &mut self.parts.rng)?;
                let (path, branch) = self.make_worktree(git, lead, &agent, &task, head).await?;
                let member = Member::new(task.clone(), Participant::Investigator(reviewer.name.clone()));
                prepared.made.push(Made { member, path, branch });
                task
            }
        };
        prepared.reviews.push(pending_review(&reviewer.name, &task, now));
        Ok(())
    }

    /// Records the new reviewers and the reviews asked of them in every item, on a copy of the state that replaces it only
    /// when all of it is recorded.
    fn record_reviews(&mut self, team_id: &TeamId, items: &[ItemId], prepared: &Prepared) -> Result<(), PublishError> {
        let mut next = self.parts.state.clone();
        for made in &prepared.made {
            next.add_reviewer(team_id, made.member.clone())?;
        }
        for id in items {
            let mut team_mut = next.team_mut(team_id).ok_or(PublishError::Lost)?;
            let mut item = team_mut.item_mut(*id).ok_or(PublishError::Lost)?;
            item.ask_reviews(prepared.reviews.clone())?;
        }
        self.parts.state = next;
        Ok(())
    }

    /// Makes the worktree of a new reviewer at `head`, on a branch of its own.
    pub(super) async fn make_worktree(
        &self,
        git: &Git,
        lead: &Lead,
        agent: &AgentName,
        task: &TaskId,
        head: &CommitId,
    ) -> Result<(AbsolutePath, BranchName), PublishError> {
        let path = self.parts.store.home().worktree_dir(&lead.repo, task);
        let text = path.to_str().ok_or_else(|| PublishError::Path {
            path: path.clone(),
            source: None,
        })?;
        let worktree = AbsolutePath::from_str(text).map_err(|source| PublishError::Path {
            path: path.clone(),
            source: Some(source),
        })?;
        let branch = task_branch(agent, task)?;
        git.worktree_add(
            &worktree,
            &WorktreeBranch::New {
                branch: branch.clone(),
                start: Revision::Commit(head.clone()),
            },
        )
        .await?;
        Ok((worktree, branch))
    }

    /// Removes the worktrees and branches `made` of reviewers that could not be asked; what cannot be removed is told.
    pub(super) async fn undo(&self, git: &Git, made: &[Made]) {
        for gone in made {
            let task = gone.member.task();
            if let Err(error) = git
                .worktree_remove(&gone.path, agent_runtime::git::RemoveMode::Force)
                .await
            {
                tracing::warn!(task = %task, error = %error_chain(&error), "the worktree of a reviewer that was not asked cannot be removed");
            }
            if let Err(error) = git
                .branch_delete(&gone.branch, agent_runtime::git::DeleteMode::Force)
                .await
            {
                tracing::warn!(task = %task, error = %error_chain(&error), "the branch of a reviewer that was not asked cannot be removed");
            }
        }
    }

    /// Hands the reviews that were asked of reviewers that wait to the reviewers: the task is queued, and its prompt tells
    /// what it is to review.
    pub(super) fn deliver_reviews(&mut self, team_id: &TeamId, look: &mut Look) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        for task in review_deliveries(team) {
            self.release(&task, Why::Review, look)?;
        }
        Ok(())
    }
}

/// A new reviewer with the worktree and the branch that were made for it.
pub(super) struct Made {
    /// The reviewer's task.
    member: Member,
    /// Its worktree.
    path: AbsolutePath,
    /// Its branch.
    branch: BranchName,
}

/// What is ready for the reviews of a pull request: the reviews, and the reviewers that were made.
#[derive(Default)]
pub(super) struct Prepared {
    /// The reviews asked.
    reviews: Vec<Review>,
    /// The new reviewers.
    made: Vec<Made>,
}
