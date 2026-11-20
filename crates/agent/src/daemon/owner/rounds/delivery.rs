//! Verification and handling of coordinator delivery reports (`DELIVERED: #N`).

use agent_core::ids::{AgentName, ItemId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::protocol::EventKind;
use agent_core::state::{
    Completion, ItemPhase, Member, PrNumber, PullRequest, Round, RoundEnd, RoundLife, Team, Timestamp,
};
use agent_core::status::{ItemStatus, TaskStatus};
use agent_runtime::git::{Revision, TreeState};

use super::super::teams::Look;
use super::super::{Owner, OwnerError};
use super::Lead;
use crate::daemon::team::BusJob;

/// Whether the member of the crew called `name` has finished: its task waits.
fn finished(roster: &[(Role, &Member)], name: &str) -> bool {
    roster
        .iter()
        .any(|(_, member)| member.agent().name() == name && member.status() == TaskStatus::Waiting)
}

/// Finds the pull request in `team` that matches `delivered_pr`, if one belongs to a round of the team.
fn find_delivered_pr(
    team: &Team,
    delivered_pr: PrNumber,
    fallback_branch: &agent_core::ids::BranchName,
    now: Timestamp,
) -> Option<(PullRequest, bool)> {
    for team_item in team.items() {
        match team_item.phase() {
            ItemPhase::InReview { pr, .. } | ItemPhase::Fixing { pr, .. } | ItemPhase::Approved { pr, .. }
                if pr.number == delivered_pr =>
            {
                return Some((pr.clone(), false));
            }
            ItemPhase::Done {
                completion: Completion::Merged { pr, .. },
            } if pr.number == delivered_pr => {
                return Some((pr.clone(), true));
            }
            ItemPhase::Done {
                completion: Completion::Delivered { pr, .. },
            } if *pr == delivered_pr => {
                return Some((
                    PullRequest {
                        number: delivered_pr,
                        branch: fallback_branch.clone(),
                        opened_at: now,
                    },
                    true,
                ));
            }
            _ => {}
        }
    }
    None
}

impl Owner {
    /// Handles a coordinator's delivery report for `team_id`.
    /// Returns `true` if accepted, `false` if refused.
    pub(in crate::daemon::owner) async fn coordinator_delivered(
        &mut self,
        team_id: &TeamId,
        task: &TaskId,
        delivered_pr: PrNumber,
        look: &mut Look,
    ) -> Result<bool, OwnerError> {
        let Some(team) = self.parts.state.team(team_id) else {
            return Ok(false);
        };
        let lead = self.lead_of(team_id)?;
        let roster = team.crew().roster();

        let candidate = team
            .items()
            .iter()
            .find(|progress| {
                matches!(
                    progress.phase(),
                    ItemPhase::InProgress { assignee, .. } if finished(&roster, assignee.as_str())
                )
            })
            .or_else(|| {
                team.items().iter().find(|progress| {
                    matches!(progress.phase(), ItemPhase::InProgress { .. })
                        && team
                            .round_of(progress.id())
                            .is_some_and(|r| matches!(r.life(), RoundLife::Open))
                })
            });

        let Some(item) = candidate else {
            return Ok(false);
        };
        let item_id = item.id();
        let Some(round) = team.round_of(item_id).cloned() else {
            return Ok(false);
        };
        let (assignee, item_branch) = match item.phase() {
            ItemPhase::InProgress { assignee, branch } => (assignee.clone(), branch.clone()),
            _ => return Ok(false),
        };

        let now = Timestamp::now(&self.parts.clock)?;
        let Some((pr, is_merged)) = find_delivered_pr(team, delivered_pr, &item_branch, now) else {
            let text = format!(
                "Delivery report refused: pull request #{delivered_pr} does not belong to a round of this team."
            );
            self.tell_bus(
                team_id,
                BusJob::Message {
                    to: lead.agent.clone(),
                    text,
                },
            );
            return Ok(false);
        };

        let member = team
            .crew()
            .roster()
            .iter()
            .find(|(_, m)| m.agent().name() == assignee.as_str())
            .map(|(_, m)| *m);
        let member_task = member.map_or_else(|| task.clone(), |m| m.task().clone());

        if let Some(text) = self
            .verify_member_branch(&lead, &member_task, &assignee, item_id, &item_branch, &pr)
            .await
        {
            self.tell_bus(
                team_id,
                BusJob::Message {
                    to: lead.agent.clone(),
                    text,
                },
            );
            return Ok(false);
        }

        self.accept_delivery(team_id, item_id, &round, delivered_pr, is_merged, look)
            .await?;
        self.finish_look(team_id, std::mem::take(look)).await?;
        Ok(true)
    }

    /// Verifies that the member branch has a clean tree and holds no extra commits.
    /// Returns `Some(reason)` if refused, or `None` if verified.
    async fn verify_member_branch(
        &self,
        lead: &Lead,
        member_task: &TaskId,
        agent: &AgentName,
        item_id: ItemId,
        item_branch: &agent_core::ids::BranchName,
        pr: &PullRequest,
    ) -> Option<String> {
        let git_path = self.parts.store.home().worktree_dir(&lead.repo, member_task);
        if !git_path.exists() {
            return None;
        }
        let Ok(git) = self.worktree_git(&lead.repo, member_task, agent) else {
            return None;
        };
        if let Ok(TreeState::Dirty(_)) = git.tree_state().await {
            return Some(format!(
                "Delivery report refused: worktree of item {item_id} has uncommitted changes."
            ));
        }
        if let Ok(Some(tip)) = git.branch_tip(item_branch).await {
            let base_branch = self.base_branch(&lead.repo).ok();
            let in_pr = git
                .is_ancestor(&Revision::Commit(tip.clone()), &Revision::Branch(pr.branch.clone()))
                .await
                .unwrap_or_default();
            let in_base = if in_pr {
                true
            } else if let Some(base) = &base_branch {
                let in_local = git
                    .is_ancestor(&Revision::Commit(tip.clone()), &Revision::Branch(base.clone()))
                    .await
                    .unwrap_or_default();
                if in_local {
                    true
                } else {
                    git.is_ancestor(&Revision::Commit(tip.clone()), &Revision::RemoteBranch(base.clone()))
                        .await
                        .unwrap_or_default()
                }
            } else {
                false
            };
            if !in_pr && !in_base {
                return Some(format!(
                    "Delivery report refused: branch {item_branch} of item {item_id} holds commits not in pull request #{} or the base branch.",
                    pr.number
                ));
            }
        }
        None
    }

    /// Transitions an item and its round to delivered or done.
    async fn accept_delivery(
        &mut self,
        team_id: &TeamId,
        item_id: ItemId,
        round: &Round,
        delivered_pr: PrNumber,
        is_merged: bool,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let clock_now = Timestamp::now(&self.parts.clock)?;
        let now = if clock_now < round.opened_at() {
            round.opened_at()
        } else {
            clock_now
        };
        if is_merged {
            let mut team_mut = self
                .parts
                .state
                .team_mut(team_id)
                .ok_or_else(|| OwnerError::LostTeam { team: team_id.clone() })?;
            let mut item_mut = team_mut.item_mut(item_id).ok_or_else(|| OwnerError::LostItem {
                team: team_id.clone(),
                item: item_id,
            })?;
            item_mut.complete_delivered(delivered_pr, now);
            look.events.push(EventKind::ItemStatus {
                team: team_id.clone(),
                item: item_id,
                round: round.id().clone(),
                status: ItemStatus::Done,
            });
            look.changed = true;
            self.close_item_round(team_id, item_id, RoundEnd::Delivered, now, look)?;
            tracing::info!(
                event = LogEvent::ItemPhaseChanged.as_str(),
                team = %team_id,
                item = %item_id,
                phase = "done",
                pr = %delivered_pr,
                "an item is delivered"
            );
            self.cleanup_merged_worktrees(team_id).await;
            self.close_if_finished(team_id, look)?;
        } else {
            self.parts
                .state
                .deliver_round(round.id(), delivered_pr)
                .map_err(|source| OwnerError::DeliverRound {
                    round: round.id().clone(),
                    source,
                })?;
            look.changed = true;
        }
        Ok(())
    }
}
