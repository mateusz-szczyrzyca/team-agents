//! The merge: reading an approved pull request, merging it in autopilot, telling the human what the team does not do by
//! itself, and completing the items of a pull request that is merged.

use agent_core::ids::{ItemId, TeamId};
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::protocol::EventKind;
use agent_core::state::{CommitId, Completion, ItemPhase, PullRequest, Timestamp};
use agent_runtime::github::{GitHub, GitHubError, MergeMethod, MergeRequest, PullState, Rejection};
use agent_runtime::process::error_chain;

use super::super::teams::Look;
use crate::daemon::merge::items_of;

use super::super::{Owner, OwnerError};
use super::publish::Site;
use super::{Announcement, Lead, PublishError, Pulls};

/// How long an approved pull request that waits (for the human, or for a merge that failed) is left alone between two
/// looks at it.
pub(super) const POLL_EVERY: time::Duration = time::Duration::seconds(30);

/// What the human is told about a pull request.
#[derive(Clone, Copy)]
enum Telling<'a> {
    /// It was closed without being merged.
    Closed,
    /// It is approved and the team does not merge by itself.
    Approved,
    /// GitHub refused to merge it, for this reason.
    MergeFailed(&'a str),
}

/// The pull request the human is told about, where it is, and when.
#[derive(Clone, Copy)]
struct About<'a> {
    /// The autopilot, the team and its lead.
    site: Site<'a>,
    /// The pull request.
    pr: &'a PullRequest,
    /// Its address in the browser.
    url: &'a str,
    /// When.
    now: Timestamp,
}

/// What was found of an approved pull request.
pub(super) enum Merge {
    /// It is merged, by the harness or by someone else.
    Merged,
    /// It is not merged and is left as it is for now.
    Waits,
}

/// Whether GitHub refused the merge for a reason that waiting does not change: the pull request cannot be merged as it is,
/// its head moved, the values are refused, or the token may not do it. A busy or failing GitHub is not a refusal.
pub(super) fn refuses(error: &GitHubError) -> bool {
    matches!(
        error,
        GitHubError::Status {
            reason: Rejection::NotAllowed
                | Rejection::Conflict
                | Rejection::Invalid
                | Rejection::Forbidden
                | Rejection::Unauthorized
                | Rejection::NotFound,
            ..
        }
    )
}

impl Owner {
    /// Merges the pull requests of `team_id` that both reviewers approved, and completes the items of those that are
    /// merged.
    pub(super) async fn merge_approved(
        &mut self,
        pulls: &Pulls,
        team_id: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let approved = self.approved_pull_requests(team_id);
        if approved.is_empty() {
            return Ok(());
        }
        let lead = self.lead_of(team_id)?;
        for (pr, items) in approved {
            let now = Timestamp::now(&self.parts.clock)?;
            if !self.due(team_id, &pr, now) {
                continue;
            }
            let site = Site {
                pulls,
                team: team_id,
                lead: &lead,
            };
            match self.try_merge(site, &pr, now).await {
                Ok(Merge::Merged) => {
                    self.watch.polled.remove(&(team_id.clone(), pr.number));
                    self.complete(team_id, &lead, &pr, &items, now, look).await?;
                }
                Ok(Merge::Waits) => {}
                Err(error) => {
                    tracing::warn!(team = %team_id, pr = %pr.number, error = %error_chain(&error), "an approved pull request cannot be merged: it is looked at again in a while");
                }
            }
        }
        Ok(())
    }

    /// The approved pull requests of `team`, with their items.
    pub(super) fn approved_pull_requests(&self, team: &TeamId) -> Vec<(PullRequest, Vec<ItemId>)> {
        let mut found: Vec<(PullRequest, Vec<ItemId>)> = Vec::new();
        let Some(team) = self.parts.state.team(team) else {
            return found;
        };
        for progress in team.items() {
            let ItemPhase::Approved { pr, .. } = progress.phase() else {
                continue;
            };
            if found.iter().all(|(known, _)| known.number != pr.number) {
                // The items of a pull request move together: all of them are approved.
                found.push((pr.clone(), items_of(team, pr.number)));
            }
        }
        found
    }

    /// Whether the approved pull request `pr` is looked at now: the first time at once, then once in [`POLL_EVERY`].
    pub(super) fn due(&mut self, team: &TeamId, pr: &PullRequest, now: Timestamp) -> bool {
        let key = (team.clone(), pr.number);
        let waiting = self
            .watch
            .polled
            .get(&key)
            .is_some_and(|last| last.as_utc().saturating_add(POLL_EVERY) > now.as_utc());
        if waiting {
            return false;
        }
        self.watch.polled.insert(key, now);
        true
    }

    /// Merges `pr` when it is open and the team merges by itself; tells the human what the team does not do by itself:
    /// a pull request that is approved and left to them, one that GitHub refuses to merge, one that was closed.
    pub(super) async fn try_merge(
        &mut self,
        site: Site<'_>,
        pr: &PullRequest,
        now: Timestamp,
    ) -> Result<Merge, PublishError> {
        let Site { pulls, lead, .. } = site;
        let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
        let github = self.github_for(pulls, &lead.repo, &git).await?;
        let remote = github.get_pull_request(pr.number).await?;
        let about = About {
            site,
            pr,
            url: &remote.url,
            now,
        };
        match remote.state {
            PullState::Merged => Ok(Merge::Merged),
            PullState::Closed => {
                self.tell_about(&about, Telling::Closed);
                Ok(Merge::Waits)
            }
            PullState::Open => self.merge_open(&github, &about, &remote.head).await,
        }
    }

    /// Merges the open pull request `about.pr`, at `head`, in autopilot; with `--no-merge` the human is told that it is
    /// approved, and a merge GitHub refused before is not tried again.
    async fn merge_open(&mut self, github: &GitHub, about: &About<'_>, head: &CommitId) -> Result<Merge, PublishError> {
        let (team, pr) = (about.site.team, about.pr);
        let autopilot = self
            .parts
            .state
            .team(team)
            .is_some_and(|found| found.merge() == agent_core::state::MergeMode::Autopilot);
        if !autopilot {
            self.tell_about(about, Telling::Approved);
            return Ok(Merge::Waits);
        }
        let key = (team.clone(), pr.number);
        if self.watch.refused.contains(&key) {
            return Ok(Merge::Waits);
        }
        let request = MergeRequest {
            method: MergeMethod::Merge,
            expected_head: head.clone(),
            commit_title: None,
            commit_message: None,
        };
        match github.merge_pull_request(pr.number, &request).await {
            Ok(_) => Ok(Merge::Merged),
            Err(error) if refuses(&error) => {
                self.watch.refused.insert(key);
                self.tell_about(about, Telling::MergeFailed(&error_chain(&error)));
                Ok(Merge::Waits)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Tells the human `telling` about a pull request, once.
    fn tell_about(&mut self, about: &About<'_>, telling: Telling<'_>) {
        let About { site, pr, url, now } = *about;
        let (team, number) = (site.team.clone(), pr.number);
        let (announcement, prompt) = match telling {
            Telling::Closed => (Announcement::Closed { team, pr: number }, AutopilotPrompt::NoticeClosed),
            Telling::Approved => (
                Announcement::Approved { team, pr: number },
                AutopilotPrompt::NoticeApproved,
            ),
            Telling::MergeFailed(_) => (
                Announcement::MergeFailed { team, pr: number },
                AutopilotPrompt::NoticeMergeFailed,
            ),
        };
        let values = Variables::new()
            .with("team", site.team.as_str())
            .and_then(|values| values.with("pr", number.to_string()))
            .and_then(|values| values.with("url", url))
            .and_then(|values| match telling {
                Telling::Closed => Ok(values),
                Telling::Approved => values.with("repo", site.lead.repo.as_str()),
                Telling::MergeFailed(reason) => values.with("reason", reason),
            });
        self.announce(site.pulls, site.lead, announcement, prompt, values, now);
    }

    /// Completes the items of the merged pull request `pr`; the checkout learns what the merge made of the base branch, so
    /// that the next item builds on it.
    pub(super) async fn complete(
        &mut self,
        team_id: &TeamId,
        lead: &Lead,
        pr: &PullRequest,
        items: &[ItemId],
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        for id in items {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item = team_mut.item_mut(*id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: *id,
            })?;
            item.advance(ItemPhase::Done {
                completion: Completion::Merged {
                    pr: pr.clone(),
                    at: now,
                },
            })
            .map_err(|source| OwnerError::Item {
                team: team_id.clone(),
                item: *id,
                source,
            })?;
            look.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: *id,
                status: agent_core::status::ItemStatus::Done,
            });
            look.changed = true;
            tracing::info!(team = %team_id, item = %id, pr = %pr.number, "an item is merged");
        }
        self.fetch_base(lead).await;
        Ok(())
    }

    /// Fetches the base branch of the repository of `lead` into the checkout. A fetch that fails leaves the next item to
    /// start from the base the checkout had; that is logged.
    pub(super) async fn fetch_base(&self, lead: &Lead) {
        let result = async {
            let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
            let base = self.base_branch(&lead.repo)?;
            git.fetch(&base).await?;
            Ok::<(), PublishError>(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(repo = %lead.repo, error = %error_chain(&error), "the base branch cannot be fetched after a merge: the next item starts from the checkout's");
        }
    }
}
