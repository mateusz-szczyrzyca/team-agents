//! Publishing: pushing the branch of the lead, opening or updating its pull request, and putting the items in review.

use agent_core::config::RepoName;
use agent_core::decide::round::{RoundEvent, RoundPolicy, RoundState, Stage};
use std::num::NonZeroU32;

use agent_core::ids::{BranchName, ItemId, TeamId};
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::protocol::EventKind;
use agent_core::state::{CommitId, GateRecord, ItemPhase, PullRequest, Timestamp};
use agent_runtime::git::{Git, Revision};
use agent_runtime::github::{NewPullRequest, PullRequestUpdate, Readiness};
use agent_runtime::process::error_chain;

use super::super::teams::Look;
use crate::daemon::pr::{Joining, Publication, Publish, plan_publications, pull_text};

use super::super::{Owner, OwnerError};
use crate::daemon::merge::count;

use super::checks::{Held, Reviewing};
use super::{Announcement, Lead, PublishError, Pulls};

/// How long a branch that could not be published is left alone before the next attempt.
pub(super) const PUBLISH_BACKOFF: time::Duration = time::Duration::seconds(60);

/// Where a publication is made: the autopilot, the team and its lead.
#[derive(Clone, Copy)]
pub(super) struct Site<'a> {
    /// The autopilot.
    pub(super) pulls: &'a Pulls,
    /// The team.
    pub(super) team: &'a TeamId,
    /// The lead of the team.
    pub(super) lead: &'a Lead,
}

/// A branch whose gate passed at its tip.
pub(super) struct Ready {
    /// Git in the lead's worktree.
    pub(super) git: Git,
    /// The tip of the branch.
    pub(super) tip: CommitId,
    /// What the gate recorded.
    pub(super) record: GateRecord,
}

/// What an attempt to publish a branch came to.
pub(super) enum Attempt {
    /// There is nothing to publish yet.
    Nothing,
    /// The pull request was opened or updated.
    Published(Published),
    /// The checks before the review hold the round back; the owner does what they ask.
    Held(Held),
}

/// A pull request that was opened or updated.
pub(super) struct Published {
    /// The pull request.
    pub(super) pr: PullRequest,
    /// The commit the gate passed on, which was pushed.
    pub(super) gated: CommitId,
    /// The commit GitHub says the pull request carries.
    pub(super) carried: CommitId,
    /// The items that go in it, in the order of the list.
    pub(super) items: Vec<ItemId>,
}

/// The items of `items` whose branch is part of the history of `tip`.
pub(super) async fn contained(git: &Git, items: &[Joining], tip: &CommitId) -> Result<Vec<ItemId>, PublishError> {
    let mut found = Vec::new();
    for joining in items {
        let part = git
            .is_ancestor(
                &Revision::Branch(joining.branch.clone()),
                &Revision::Commit(tip.clone()),
            )
            .await?;
        if part {
            found.push(joining.item);
        }
    }
    Ok(found)
}

impl Owner {
    /// Publishes the branches of `team_id` that are ready: its pull requests are opened or updated, and its items go in
    /// review.
    pub(super) async fn publish_branches(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(());
        };
        let plan = plan_publications(team).map_err(|source| OwnerError::Publications {
            team: team_id.clone(),
            source,
        })?;
        if plan.is_empty() {
            return Ok(());
        }
        let lead = self.lead_of(team_id)?;
        for publication in plan {
            let now = Timestamp::now(&self.parts.clock)?;
            let key = (team_id.clone(), publication.head_branch.clone());
            if self.watch.retry_at.get(&key).is_some_and(|retry| *retry > now) {
                continue;
            }
            let site = Site {
                pulls,
                team: team_id,
                lead: &lead,
            };
            match self.try_publish(site, &publication, now).await {
                Ok(Attempt::Nothing) => {}
                Ok(Attempt::Published(published)) => {
                    self.watch.retry_at.remove(&key);
                    self.record_publication(team_id, &publication, &published, look)?;
                }
                Ok(Attempt::Held(held)) => {
                    self.watch.retry_at.remove(&key);
                    self.hold_round(pulls, team_id, &lead, held, look)?;
                }
                Err(error) => self.publish_failed(pulls, team_id, &lead, &publication, &error, now)?,
            }
        }
        Ok(())
    }

    /// Pushes the branch of `publication` and opens or updates its pull request when the lead's gate passed at its tip and
    /// the checks before the review let the round go to it.
    pub(super) async fn try_publish(
        &mut self,
        site: Site<'_>,
        publication: &Publication,
        now: Timestamp,
    ) -> Result<Attempt, PublishError> {
        let lead = site.lead;
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let Some(tip) = git.branch_tip(&publication.head_branch).await? else {
            return Ok(Attempt::Nothing);
        };
        let Some(record) = self.parts.store.load_gate_record(&lead.task).await? else {
            return Ok(Attempt::Nothing);
        };
        if !record.counts_as_pass() || record.result.head != tip {
            return Ok(Attempt::Nothing);
        }
        let ready = Ready { git, tip, record };
        match &publication.publish {
            Publish::Open { items } => {
                self.open_pull_request(site, &publication.head_branch, items, &ready, now)
                    .await
            }
            Publish::Update { pr, fixing, joining } => {
                self.update_pull_request(site, (pr, fixing), joining, &ready, now).await
            }
        }
    }

    /// Opens the pull request of the finished `items` whose branches `ready.tip` contains: pushes the branch, finds the
    /// pull request an earlier attempt opened or opens a draft, and marks it ready.
    async fn open_pull_request(
        &mut self,
        site: Site<'_>,
        head_branch: &BranchName,
        items: &[Joining],
        ready: &Ready,
        now: Timestamp,
    ) -> Result<Attempt, PublishError> {
        let Site { pulls, team, lead } = site;
        let ids = contained(&ready.git, items, &ready.tip).await?;
        if ids.is_empty() {
            return Ok(Attempt::Nothing);
        }
        let github = self.github_for(pulls, &lead.repo, &ready.git).await?;
        ready.git.push(head_branch).await?;
        let text = self.pull_text_of(pulls, team, &lead.repo, &ids, &ready.record)?;
        let base = self.base_branch(&lead.repo)?;
        let remote = match github.find_open_pull_request(head_branch).await? {
            Some(found) => found,
            None => {
                github
                    .create_pull_request(&NewPullRequest {
                        title: text.title,
                        body: text.body,
                        head: head_branch.clone(),
                        base,
                        readiness: Readiness::Draft,
                    })
                    .await?
            }
        };
        // The round goes to the reviewers only when the checks let it: until then the pull request stays a draft.
        let reviewing = Reviewing {
            items: &ids,
            ready,
            pr: remote.number,
            description: &remote.body,
        };
        if let Some(held) = self.check_before_review(site, &reviewing, now).await? {
            return Ok(Attempt::Held(held));
        }
        github.mark_ready(remote.number).await?;
        Ok(Attempt::Published(Published {
            pr: PullRequest {
                number: remote.number,
                branch: head_branch.clone(),
                opened_at: now,
            },
            gated: ready.tip.clone(),
            carried: remote.head,
            items: ids,
        }))
    }

    /// Updates the pull request `pr` whose items `fixing` were fixed: pushes the branch, tells the reviewers in a comment,
    /// and takes the finished `joining` items whose branches `ready.tip` contains in with it.
    async fn update_pull_request(
        &mut self,
        site: Site<'_>,
        (pr, fixing): (&PullRequest, &[ItemId]),
        joining: &[Joining],
        ready: &Ready,
        now: Timestamp,
    ) -> Result<Attempt, PublishError> {
        let Site { pulls, team, lead } = site;
        let joined = contained(&ready.git, joining, &ready.tip).await?;
        let github = self.github_for(pulls, &lead.repo, &ready.git).await?;
        let before = github.get_pull_request(pr.number).await?;
        if before.head == ready.tip && joined.is_empty() {
            return Ok(Attempt::Nothing);
        }
        let mut ids = fixing.to_vec();
        ids.extend(joined.iter().copied());
        let ids = self.in_list_order(team, &ids);
        // A head the checks hold back is not pushed: the reviewers keep the head they reviewed.
        let reviewing = Reviewing {
            items: &ids,
            ready,
            pr: pr.number,
            description: &before.body,
        };
        if let Some(held) = self.check_before_review(site, &reviewing, now).await? {
            return Ok(Attempt::Held(held));
        }
        ready.git.push(&pr.branch).await?;
        if !joined.is_empty() {
            let text = self.pull_text_of(pulls, team, &lead.repo, &ids, &ready.record)?;
            let update = PullRequestUpdate {
                title: Some(text.title),
                body: Some(text.body),
                ..PullRequestUpdate::default()
            };
            github.update_pull_request(pr.number, &update).await?;
        }
        let comment = pulls.texts.render(
            AutopilotPrompt::PrUpdate,
            &Variables::new()
                .with("head", ready.tip.as_str())?
                .with("pass", self.fix_passes_of(team, fixing).to_string())?
                .with("gate", ready.record.summary())?,
        )?;
        github.post_comment(pr.number, comment.trim_end()).await?;
        let after = github.get_pull_request(pr.number).await?;
        Ok(Attempt::Published(Published {
            pr: pr.clone(),
            gated: ready.tip.clone(),
            carried: after.head,
            items: ids,
        }))
    }

    /// The title and the description of the pull request of `ids`.
    pub(super) fn pull_text_of(
        &self,
        pulls: &Pulls,
        team_id: &TeamId,
        repo: &RepoName,
        ids: &[ItemId],
        record: &GateRecord,
    ) -> Result<crate::daemon::pr::PullText, PublishError> {
        let team = self
            .parts
            .state
            .team(team_id)
            .ok_or_else(|| PublishError::RepoGone { repo: repo.clone() })?;
        Ok(pull_text(&pulls.texts, team, repo, ids, record)?)
    }

    /// The branch the pull requests of `repo` are made against.
    pub(super) fn base_branch(&self, repo: &RepoName) -> Result<BranchName, PublishError> {
        self.parts
            .settings
            .config
            .repo(repo)
            .map(|configured| configured.base_branch.clone())
            .ok_or_else(|| PublishError::RepoGone { repo: repo.clone() })
    }

    /// `ids` in the order of the task list of `team`.
    pub(super) fn in_list_order(&self, team: &TeamId, ids: &[ItemId]) -> Vec<ItemId> {
        self.parts.state.team(team).map_or_else(Vec::new, |found| {
            found
                .items()
                .iter()
                .map(agent_core::state::ItemProgress::id)
                .filter(|id| ids.contains(id))
                .collect()
        })
    }

    /// How many fix passes the first of `items` has had.
    pub(super) fn fix_passes_of(&self, team: &TeamId, items: &[ItemId]) -> usize {
        items
            .first()
            .and_then(|first| self.parts.state.team(team).and_then(|found| found.item(*first)))
            .map_or(0, |progress| progress.fix_passes().len())
    }

    /// The items of `published` for which the round machine agrees that the pull request carries the commit the gate passed
    /// on; the others are looked at again.
    fn confirmed(&self, team_id: &TeamId, publication: &Publication, published: &Published) -> Vec<ItemId> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Vec::new();
        };
        let policy = RoundPolicy::new(team.merge(), self.parts.settings.config.review());
        let mut confirmed = Vec::new();
        for id in &published.items {
            let stage = match (&publication.publish, team.item(*id)) {
                (Publish::Update { fixing, .. }, Some(progress)) if fixing.contains(id) => Stage::Fixing {
                    pr: published.pr.number,
                    pass: NonZeroU32::MIN.saturating_add(count(progress.fix_passes().len()).saturating_sub(1)),
                },
                _ => Stage::First,
            };
            let publishing = RoundState::Publishing {
                stage,
                head: published.gated.clone(),
            };
            let event = RoundEvent::PullRequestPublished {
                pr: published.pr.number,
                head: published.carried.clone(),
            };
            match publishing.transition(&event, policy) {
                Ok(_) => confirmed.push(*id),
                Err(error) => {
                    tracing::warn!(team = %team_id, item = %id, error = %error_chain(&error), "a pull request does not carry what the gate passed: it is looked at again");
                }
            }
        }
        confirmed
    }

    /// Records that the items of `published` are in review, after the round machine checked that the pull request carries
    /// the commit the gate passed on.
    pub(super) fn record_publication(
        &mut self,
        team_id: &TeamId,
        publication: &Publication,
        published: &Published,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        for id in self.confirmed(team_id, publication, published) {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team_mut.item_mut(id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: id,
            })?;
            item.advance(ItemPhase::InReview {
                pr: published.pr.clone(),
                reviews: Vec::new(),
            })
            .map_err(|source| OwnerError::Item {
                team: team_id.clone(),
                item: id,
                source,
            })?;
            // An item that was being fixed has its pull request already; the others have it opened for them now.
            let opened = !matches!(&publication.publish, Publish::Update { fixing, .. } if fixing.contains(&id));
            if opened {
                look.events.push(EventKind::PullRequestOpened {
                    team: team_id.clone(),
                    item: id,
                    pr: published.pr.number,
                });
            }
            look.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: id,
                status: agent_core::status::ItemStatus::InReview,
            });
            look.changed = true;
            tracing::info!(team = %team_id, item = %id, pr = %published.pr.number, "an item is in review");
        }
        Ok(())
    }

    /// What follows a branch that could not be published: it is logged, tried again after a while, and the human is told
    /// once.
    pub(super) fn publish_failed(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        lead: &Lead,
        publication: &Publication,
        error: &PublishError,
        now: Timestamp,
    ) -> Result<(), OwnerError> {
        let reason = error_chain(error);
        tracing::warn!(team = %team_id, branch = %publication.head_branch, %reason, "a branch cannot be published: it is tried again in a minute");
        let retry = Timestamp::from_utc(now.as_utc().saturating_add(PUBLISH_BACKOFF))?;
        self.watch
            .retry_at
            .insert((team_id.clone(), publication.head_branch.clone()), retry);
        let announcement = Announcement::PublishFailed {
            team: team_id.clone(),
            branch: publication.head_branch.clone(),
        };
        if !self.watch.announced.insert(announcement) {
            return Ok(());
        }
        let items = match &publication.publish {
            Publish::Open { items } => items.iter().map(|joining| joining.item.to_string()).collect::<Vec<_>>(),
            Publish::Update { fixing, joining, .. } => fixing
                .iter()
                .copied()
                .chain(joining.iter().map(|joining| joining.item))
                .map(|id| id.to_string())
                .collect(),
        };
        let values = Variables::new()
            .with("team", team_id.as_str())
            .and_then(|values| values.with("items", items.join(", ")))
            .and_then(|values| values.with("reason", &reason));
        self.tell_human(pulls, AutopilotPrompt::NoticePublishFailed, values, lead, now);
        Ok(())
    }
}
