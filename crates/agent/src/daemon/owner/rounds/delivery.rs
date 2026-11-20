//! Harness verification of coordinator delivery claims before publication.

use agent_core::decide::delivery::{bind_delivery, delivery_report};
use agent_core::history::HistoryEvent;
use agent_core::ids::{BranchName, RoundId, TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::prompt::{AutopilotPrompt, Variables};
use agent_core::protocol::EventKind;
use agent_core::state::{ItemPhase, PrNumber, RoundEnd, Timestamp};
use agent_runtime::git::{DeliveryGitError, DeliveryRead, Revision};
use agent_runtime::github::PullState;
use agent_runtime::process::error_chain;

use super::super::teams::Look;
use super::super::{Owner, OwnerError};
use super::{PublishError, Pulls};
use crate::daemon::team::Why;

/// Why a checked delivery cannot be retained or its correction rendered.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DeliveryDaemonError {
    /// The checked state rejected a claim.
    #[error(transparent)]
    Deliver(#[from] agent_core::state::DeliverRoundError),
    /// The checked state rejected a completion.
    #[error(transparent)]
    Complete(#[from] agent_core::state::CompleteDeliveryError),
    /// A correction variable was invalid.
    #[error("cannot prepare delivery correction")]
    Variable(#[from] agent_core::prompt::VariablesError),
    /// The loaded correction could not be rendered.
    #[error("cannot render delivery correction")]
    Render(#[from] agent_runtime::autopilot_prompts::RenderAutopilotTextError),
    /// The correction used without an enabled autopilot could not be rendered.
    #[error("cannot render delivery correction without GitHub")]
    Catalog(#[from] agent_core::prompt::PromptError),
}

/// Why the harness refused a claim; no delivery state has changed yet.
#[derive(Debug, thiserror::Error)]
enum ClaimError {
    /// The marker did not follow the coordinator outcome protocol.
    #[error(transparent)]
    Report(#[from] agent_core::decide::delivery::DeliveryReportError),
    /// The ended run did not identify one working item.
    #[error(transparent)]
    Binding(#[from] agent_core::decide::delivery::DeliveryBindingError),
    /// The named PR did not belong to this team.
    #[error(transparent)]
    Team(#[from] agent_core::state::DeliverRoundError),
    /// The member no longer owns an available item checkout.
    #[error("the reported item has no available member checkout")]
    Member,
    /// GitHub cannot be consulted without credentials.
    #[error("GitHub is not configured for checking delivery claims")]
    Disabled,
    /// Repository configuration or a git operation could not be read.
    #[error("cannot prepare the delivery repository")]
    Repository(#[from] PublishError),
    /// Git could not establish the claimed containment.
    #[error(transparent)]
    Git(#[from] DeliveryGitError),
    /// The PR could not be read from GitHub.
    #[error("cannot read the delivering pull request")]
    GitHub(#[from] agent_runtime::github::GitHubError),
    /// A closed PR cannot deliver work until it is merged.
    #[error("the delivering pull request was closed without merging")]
    Closed,
    /// The stored team PR and the repository's PR disagree.
    #[error("the delivering pull request does not use its recorded team branch")]
    Branch,
}

impl Owner {
    /// Verifies a claim from the run that just ended, before any gate or publication.
    pub(in crate::daemon::owner) async fn read_delivery_claim(
        &mut self,
        task: &TaskId,
        team: &TeamId,
        message: &str,
    ) -> Result<(), OwnerError> {
        let claim = match delivery_report(message) {
            Ok(None) => return Ok(()),
            Ok(Some(number)) => self.check_delivery_claim(task, team, number).await,
            Err(error) => Err(ClaimError::Report(error)),
        };
        let mut look = Look::default();
        match claim {
            Ok((round, number, state)) => {
                self.parts
                    .state
                    .team_mut(team)
                    .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?
                    .deliver_round(&round, number)
                    .map_err(DeliveryDaemonError::from)?;
                look.changed = true;
                if state == PullState::Merged {
                    let now = Timestamp::now(&self.parts.clock)?;
                    self.finish_delivery(team, &round, now, &mut look)?;
                }
                tracing::info!(event = LogEvent::RoundChanged.as_str(), %team, %round, pr = %number,
                    change = "delivery_accepted", "the harness verified a delivery claim");
            }
            Err(error) => {
                tracing::warn!(event = LogEvent::RoundChanged.as_str(), %team,
                    change = "delivery_refused", error = %error_chain(&error), "a delivery claim was refused");
                let values = Variables::new()
                    .with("reason", error_chain(&error))
                    .map_err(DeliveryDaemonError::from)?;
                let text = match &self.pulls {
                    Some(pulls) => pulls
                        .texts()
                        .render(AutopilotPrompt::DeliveryRefused, &values)
                        .map_err(DeliveryDaemonError::from)?,
                    None => AutopilotPrompt::DeliveryRefused
                        .spec()
                        .render(
                            include_str!("../../../../../../prompts/harness/pr/delivery-refused.md"),
                            &values,
                        )
                        .map_err(DeliveryDaemonError::from)?,
                };
                self.release(task, Why::Gate(text), &mut look)?;
            }
        }
        self.parts.state.record_activities(Timestamp::now(&self.parts.clock)?);
        self.save().await?;
        let now = Timestamp::now(&self.parts.clock)?;
        for event in look.events {
            self.publish(now, event);
        }
        self.look_at(team);
        Ok(())
    }

    /// Reads team ownership and clean branch containment without modifying round state.
    async fn check_delivery_claim(
        &mut self,
        task: &TaskId,
        team_id: &TeamId,
        number: PrNumber,
    ) -> Result<(RoundId, PrNumber, PullState), ClaimError> {
        let team = self.parts.state.team(team_id).ok_or(ClaimError::Member)?;
        let run = team
            .find(task)
            .and_then(|(_, member)| member.runs().last())
            .ok_or(ClaimError::Member)?;
        let round = bind_delivery(team, run.rounds())?;
        let pr = team
            .delivery_pull_request(number)
            .cloned()
            .ok_or(agent_core::state::DeliverRoundError::NoTeamPr { number })?;
        let progress = team
            .round(&round)
            .and_then(agent_core::state::Round::item)
            .and_then(|id| team.item(id))
            .ok_or(ClaimError::Member)?;
        let ItemPhase::InProgress { assignee, branch } = progress.phase() else {
            return Err(ClaimError::Member);
        };
        let member = self.task_of(team_id, assignee).ok_or(ClaimError::Member)?;
        let git = self.worktree_git(team.repo(), &member, assignee)?;
        let item = branch.clone();
        let repo = team.repo().clone();
        let base = self.base_branch(&repo)?;
        let pulls = self.pulls.clone().ok_or(ClaimError::Disabled)?;
        let github = self.github_for(&pulls, &repo, &git).await?;
        let remote = github.get_pull_request(number).await?;
        if remote.head_branch != pr.branch {
            return Err(ClaimError::Branch);
        }
        if remote.state == PullState::Closed {
            return Err(ClaimError::Closed);
        }
        git.fetch(&base).await.map_err(|source| DeliveryGitError::Read {
            read: DeliveryRead::Base,
            source,
        })?;
        let base = Revision::RemoteBranch(base);
        git.verify_delivery(&item, &pr.branch, &base).await?;
        verify_published_tip(&git, &item, &base, &remote.head).await?;
        Ok((round, number, remote.state))
    }

    /// Polls accepted claims independently of the delivering item's current review or fix pass.
    pub(in crate::daemon::owner) async fn poll_deliveries(
        &mut self,
        pulls: &Pulls,
        team: &TeamId,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let now = Timestamp::now(&self.parts.clock)?;
        let waiting: Vec<_> = self
            .parts
            .state
            .team(team)
            .into_iter()
            .flat_map(agent_core::state::Team::delivery_waits)
            .filter_map(|round| round.delivered_by().map(|pr| (round.id().clone(), pr.number)))
            .filter(|(round, _)| {
                self.watch
                    .delivery_polled
                    .get(round)
                    .is_none_or(|last| last.as_utc().saturating_add(super::merges::POLL_EVERY) <= now.as_utc())
            })
            .collect();
        for (round, number) in waiting {
            self.watch.delivery_polled.insert(round.clone(), now);
            let lead = self.lead_of(team)?;
            let observed = async {
                let git = self.worktree_git(&lead.repo, &lead.task, &lead.agent)?;
                let github = self.github_for(pulls, &lead.repo, &git).await?;
                Ok::<_, ClaimError>(github.get_pull_request(number).await?.state)
            }
            .await;
            match observed {
                Ok(PullState::Merged) => {
                    self.finish_delivery(team, &round, now, look)?;
                    tracing::info!(event = LogEvent::RoundChanged.as_str(), %team, %round, pr = %number,
                        change = "delivery_merged", "a delivered item completed at its PR merge");
                }
                Ok(PullState::Open) => {}
                Ok(PullState::Closed) => {
                    // Reopening and member rescheduling belong to the second part of this feature.
                    // Preserve the accepted claim rather than falsely completing a closed, unmerged PR.
                    tracing::warn!(event = LogEvent::RoundChanged.as_str(), %team, %round, pr = %number,
                        change = "delivery_closed", "delivering PR closed without merging; item remains waiting");
                }
                Err(error) => {
                    // A failed observation proves no merge. Retry this durable claim at the next poll.
                    tracing::warn!(event = LogEvent::RoundChanged.as_str(), %team, %round, pr = %number,
                        change = "delivery_poll_failed", error = %error_chain(&error),
                        "cannot observe delivering PR; item remains waiting");
                }
            }
        }
        Ok(())
    }

    /// Closes checked delivery state and records the same lifecycle events as an ordinary merge.
    fn finish_delivery(
        &mut self,
        team: &TeamId,
        round: &RoundId,
        now: Timestamp,
        look: &mut Look,
    ) -> Result<(), OwnerError> {
        let found = self
            .parts
            .state
            .team(team)
            .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?;
        let item = found
            .round(round)
            .and_then(agent_core::state::Round::item)
            .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?;
        // A stop abandoned the round while the claim waited: its end was recorded then and is not recorded again.
        let was_open = found.round(round).is_some_and(agent_core::state::Round::is_open);
        self.parts
            .state
            .team_mut(team)
            .ok_or_else(|| OwnerError::LostTeam { team: team.clone() })?
            .complete_delivery(round, now)
            .map_err(DeliveryDaemonError::from)?;
        let status = self
            .parts
            .state
            .team(team)
            .and_then(|team| team.item(item))
            .ok_or_else(|| OwnerError::LostItem {
                team: team.clone(),
                item,
            })?
            .status();
        if was_open {
            self.record_event_at(
                now,
                team,
                vec![round.clone()],
                HistoryEvent::RoundClosed {
                    round: round.clone(),
                    end: RoundEnd::Merged,
                },
            );
            look.events.push(EventKind::RoundClosed {
                team: team.clone(),
                round: round.clone(),
                item: Some(item),
                how: RoundEnd::Merged,
            });
        }
        look.events.push(EventKind::ItemStatus {
            team: team.clone(),
            item,
            round: round.clone(),
            status,
        });
        look.changed = true;
        self.cleanup_round_targets(team, round);
        Ok(())
    }
}

/// Rejects unpublished additions to the local PR branch, using GitHub's actual head as the evidence.
async fn verify_published_tip(
    git: &agent_runtime::git::Git,
    item: &BranchName,
    base: &Revision,
    head: &agent_core::state::CommitId,
) -> Result<(), DeliveryGitError> {
    let tip = git
        .rev_parse(&Revision::Branch(item.clone()))
        .await
        .map_err(|source| DeliveryGitError::Read {
            read: DeliveryRead::Item,
            source,
        })?;
    let tip = Revision::Commit(tip);
    if git
        .is_ancestor(&tip, base)
        .await
        .map_err(|source| DeliveryGitError::Read {
            read: DeliveryRead::Base,
            source,
        })?
        || git
            .is_ancestor(&tip, &Revision::Commit(head.clone()))
            .await
            .map_err(|source| DeliveryGitError::Read {
                read: DeliveryRead::PullRequest,
                source,
            })?
    {
        Ok(())
    } else {
        Err(DeliveryGitError::OwnCommits)
    }
}
