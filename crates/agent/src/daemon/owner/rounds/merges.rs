//! The merge: reading an approved pull request, merging it in autopilot, telling the human what the team does not do by
//! itself, and completing the items of a pull request that is merged.

use super::waiting::WaitingStep;
use agent_core::decide::merge_refusal::{AfterRefusal, ConflictBudget, Refusal, after_refusal};
use agent_core::history::HistoryEvent;
use agent_core::ids::{ItemId, TeamId};
use agent_core::log::LogEvent;
use agent_core::prompt::Variables;
use agent_core::protocol::EventKind;
use agent_core::state::{CommitId, Completion, ItemPhase, PrNumber, PullRequest, RoundEnd, Timestamp};
use agent_runtime::github::{GitHub, MergeMethod, MergeRefusalKind, MergeRequest, PullState, merge_refusal_kind};
use agent_runtime::process::error_chain;

use super::super::teams::Look;
use crate::daemon::merge::items_of;

use super::super::{Owner, OwnerError};
use super::conflicts::ConflictStep;
use super::publish::Site;
use super::{Announcement, AutopilotNotice, Lead, PublishError, Pulls};

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
    /// It is merged, by the harness or by someone else; the merge commit when GitHub reports it.
    Merged {
        /// The merge commit sha, if reported by GitHub.
        commit: Option<CommitId>,
        /// The address of the pull request on GitHub.
        url: String,
    },
    /// It is not merged and is left as it is for now.
    Waits,
    /// GitHub refused it for a conflict and the lead merges the base, or the human is asked (`step`). `head` is the head
    /// the refusal was for.
    Conflict {
        /// What the daemon starts on the items of the pull request.
        step: ConflictStep,
        /// The head GitHub refused to merge.
        head: CommitId,
    },
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
            if self.in_merge_conflict(team_id, &items) {
                continue;
            }
            let fixing = self
                .parts
                .state
                .team(team_id)
                .and_then(|team| items.first().and_then(|id| team.item(*id)))
                .filter(|item| matches!(item.phase(), ItemPhase::Fixing { .. }))
                .cloned();
            if let Some(item) = fixing {
                let place = self.place_of(team_id, &items);
                let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent);
                let Some(head) = self
                    .head_of(&place, team_id, WaitingStep::ReadMergeHead, Some(pr.number), git)
                    .await?
                else {
                    continue;
                };
                if item
                    .findings()
                    .and_then(|ledger| ledger.verification())
                    .and_then(|context| context.reviewed_head.as_ref())
                    != Some(&head)
                {
                    continue;
                }
            }
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
                Ok(Merge::Merged { commit, url }) => {
                    self.watch.polled.remove(&(team_id.clone(), pr.number));
                    self.record_event_at(
                        now,
                        team_id,
                        self.rounds_of_items(team_id, &items),
                        HistoryEvent::PullMerged { pr: pr.number, commit },
                    );
                    self.complete(
                        &About {
                            site,
                            pr: &pr,
                            url: &url,
                            now,
                        },
                        &items,
                        look,
                    )
                    .await?;
                }
                Ok(Merge::Waits) => {}
                Ok(Merge::Conflict { step, head }) => {
                    self.start_merge_conflict(team_id, &items, step, &head, now, look)
                        .await?;
                }
                Err(error) => {
                    tracing::warn!(
                        event = LogEvent::GithubCallFailed.as_str(),
                        team = %team_id,
                        pr = %pr.number,
                        error = %error_chain(&error),
                        "an approved pull request cannot be merged: it is looked at again in a while"
                    );
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
            let pr = match progress.phase() {
                ItemPhase::Approved { pr, .. } => pr,
                ItemPhase::Fixing { pr, .. }
                    if progress.findings().is_some_and(|ledger| {
                        std::num::NonZeroU32::new(crate::daemon::merge::count(progress.fix_passes().len())).is_some_and(
                            |pass| {
                                agent_core::decide::opinion::after_opinions(
                                    ledger,
                                    pass,
                                    &ledger.fixed_answers(pass),
                                    agent_core::decide::opinion::Tip::Unchanged,
                                ) == agent_core::decide::opinion::Next::DecideUnchanged
                            },
                        )
                    }) =>
                {
                    pr
                }
                ItemPhase::Pending
                | ItemPhase::InProgress { .. }
                | ItemPhase::InReview { .. }
                | ItemPhase::Fixing { .. }
                | ItemPhase::Done { .. } => continue,
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
            PullState::Merged => Ok(Merge::Merged {
                commit: remote.merge_commit_sha,
                url: remote.url,
            }),
            PullState::Closed => {
                let team = site.team;
                let items = self
                    .parts
                    .state
                    .team(team)
                    .map_or_else(Vec::new, |found| items_of(found, pr.number));
                self.record_event(
                    team,
                    self.rounds_of_items(team, &items),
                    HistoryEvent::PullClosed { pr: pr.number },
                );
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
            Ok(merged) => Ok(Merge::Merged {
                commit: Some(merged.commit),
                url: about.url.to_string(),
            }),
            Err(error) => match merge_refusal_kind(&error) {
                // A busy or failing GitHub is tried again in a while; it refuses nothing.
                None => Err(error.into()),
                Some(kind) => {
                    let budget = self.conflict_budget(team, pr.number);
                    let refusal = match kind {
                        MergeRefusalKind::Conflict => Refusal::Conflict,
                        MergeRefusalKind::Other => Refusal::Other,
                    };
                    let decided = after_refusal(refusal, budget);
                    agent_runtime::history::log_decision(
                        "after_refusal",
                        &format!("pr={}, refusal={refusal:?}, budget={budget:?}", pr.number),
                        &format!("{decided:?}"),
                    );
                    match decided {
                        AfterRefusal::TellHuman => {
                            self.watch.refused.insert(key);
                            self.tell_about(about, Telling::MergeFailed(&error_chain(&error)));
                            Ok(Merge::Waits)
                        }
                        after @ (AfterRefusal::LeadMergesBase | AfterRefusal::AskHuman) => Ok(Merge::Conflict {
                            step: match after {
                                AfterRefusal::AskHuman => ConflictStep::AskHuman,
                                AfterRefusal::LeadMergesBase | AfterRefusal::TellHuman => ConflictStep::MergeBase,
                            },
                            head: head.clone(),
                        }),
                    }
                }
            },
        }
    }

    /// The conflict passes of the items of the pull request `number` of `team`: the passes spent and the configured limit.
    fn conflict_budget(&self, team: &TeamId, number: PrNumber) -> ConflictBudget {
        let spent = self
            .parts
            .state
            .team(team)
            .and_then(|found| items_of(found, number).first().and_then(|id| found.item(*id)))
            .map_or(0, agent_core::state::ItemProgress::merge_passes);
        ConflictBudget {
            spent,
            limit: self.parts.settings.config.merge().conflict_passes,
        }
    }

    /// Whether one of `items` is in a merge conflict pass: its approved pull request is not merged while the lead works.
    pub(super) fn in_merge_conflict(&self, team: &TeamId, items: &[ItemId]) -> bool {
        self.parts.state.team(team).is_some_and(|found| {
            items
                .iter()
                .any(|id| found.item(*id).is_some_and(|item| item.merge_conflict().is_some()))
        })
    }

    /// Tells the human `telling` about a pull request, once.
    fn tell_about(&mut self, about: &About<'_>, telling: Telling<'_>) {
        let About { site, pr, url, now } = *about;
        let (team, number) = (site.team.clone(), pr.number);
        let (announcement, notice) = match telling {
            Telling::Closed => (Announcement::Closed { team, pr: number }, AutopilotNotice::Closed),
            Telling::Approved => (Announcement::Approved { team, pr: number }, AutopilotNotice::Approved),
            Telling::MergeFailed(_) => (
                Announcement::MergeFailed { team, pr: number },
                AutopilotNotice::MergeFailed,
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
        self.announce(site.pulls, site.lead, announcement, notice, values, now);
    }

    /// Completes the items of the merged pull request `pr`; the checkout learns what the merge made of the base branch, so
    /// that the next item builds on it.
    async fn complete(&mut self, about: &About<'_>, items: &[ItemId], look: &mut Look) -> Result<(), OwnerError> {
        let About { site, pr, url, now } = *about;
        for id in items {
            let mut team_mut = self
                .parts
                .state
                .team_mut(site.team)
                .ok_or_else(|| OwnerError::LostTeam {
                    team: site.team.clone(),
                })?;
            let mut item = team_mut.item_mut(*id).ok_or_else(|| OwnerError::LostItem {
                team: site.team.clone(),
                item: *id,
            })?;
            let completion = if matches!(item.phase(), ItemPhase::Fixing { .. }) {
                let head = item
                    .findings()
                    .and_then(|ledger| ledger.verification())
                    .and_then(|context| context.reviewed_head.as_ref())
                    .cloned()
                    .ok_or_else(|| OwnerError::LostItem {
                        team: site.team.clone(),
                        item: *id,
                    })?;
                item.complete_unconfirmed(&head, now)
            } else {
                item.advance(ItemPhase::Done {
                    completion: Completion::Merged {
                        pr: pr.clone(),
                        at: now,
                    },
                })
            };
            completion.map_err(|source| OwnerError::Item {
                team: site.team.clone(),
                item: *id,
                source,
            })?;
            let round = self.round_of_item(site.team, *id)?;
            self.close_item_round(site.team, *id, RoundEnd::Merged, now, look)?;
            let status = self
                .parts
                .state
                .team(site.team)
                .and_then(|team| team.item(*id))
                .ok_or_else(|| OwnerError::LostItem {
                    team: site.team.clone(),
                    item: *id,
                })?
                .status();
            look.events.push(EventKind::ItemStatus {
                team: site.team.clone(),
                item: *id,
                round: round.clone(),
                status,
            });
            look.changed = true;
            let span = tracing::error_span!("round", team = %site.team, rounds = %round);
            let _entered = span.enter();
            tracing::info!(
                event = LogEvent::ItemPhaseChanged.as_str(),
                team = %site.team,
                item = %id,
                phase = "done",
                pr = %pr.number,
                "an item is merged"
            );
        }
        // A restart uses the saved Done phases to deduplicate this notice. Never make
        // delivery visible while a failed save can still leave the items approved.
        self.parts.state.record_activities(Timestamp::now(&self.parts.clock)?);
        self.save().await?;
        let announcement = Announcement::Merged {
            team: site.team.clone(),
            pr: pr.number,
        };
        let items_text = items.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
        let values = Variables::new()
            .with("team", site.team.as_str())
            .and_then(|v| v.with("pr", pr.number.to_string()))
            .and_then(|v| v.with("url", url))
            .and_then(|v| v.with("items", items_text));
        self.announce(
            site.pulls,
            site.lead,
            announcement,
            AutopilotNotice::Merged,
            values,
            now,
        );
        self.fetch_base(site.lead).await;
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
            tracing::warn!(
                event = LogEvent::GitCallFailed.as_str(),
                repo = %lead.repo,
                error = %error_chain(&error),
                "the base branch cannot be fetched after a merge: the next item starts from the checkout's"
            );
        }
    }
}

#[cfg(test)]
mod tests;
